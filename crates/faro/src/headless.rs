//! Stand-in for the TUI until M3: runs a backend, logs what it emits and reports a
//! summary when stopped. Delete once the TUI exists.
//!
//! Logs only event kinds, connection states and error messages: never message
//! bodies, names or phone numbers (DESIGN.md §12).

use std::collections::BTreeMap;
use std::fmt;
use std::future::Future;
use std::time::Duration;

use faro_core::backend::{Backend, BackendEvent, BackendEvents};
use faro_core::store::Store;
use tokio::time::Instant;

/// What a run saw, for the exit message and the log.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Summary {
    pub counts: BTreeMap<&'static str, u64>,
    pub elapsed: Duration,
}

impl Summary {
    pub fn total(&self) -> u64 {
        self.counts.values().sum()
    }

    #[cfg(test)]
    pub fn count(&self, kind: &str) -> u64 {
        self.counts.get(kind).copied().unwrap_or(0)
    }
}

impl fmt::Display for Summary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} events in {}s", self.total(), self.elapsed.as_secs())?;
        let mut kinds = self.counts.iter();
        if let Some((kind, n)) = kinds.next() {
            write!(f, " ({kind}: {n}")?;
            kinds.try_for_each(|(kind, n)| write!(f, ", {kind}: {n}"))?;
            f.write_str(")")?;
        }
        Ok(())
    }
}

/// Receive, log and store `backend`'s events until `shutdown` completes or the
/// backend stops.
pub async fn run<B: Backend>(
    backend: &B,
    mut events: BackendEvents,
    store: &Store,
    shutdown: impl Future<Output = ()>,
) -> Summary {
    let start = Instant::now();
    if let Err(error) = sync_directory(backend, store).await {
        tracing::warn!(%error, "couldn't load account, contacts or groups");
    }

    let mut counts = BTreeMap::new();
    tokio::pin!(shutdown);
    loop {
        let event = tokio::select! {
            () = &mut shutdown => break,
            event = events.recv() => event,
        };
        let Some(event) = event else {
            tracing::warn!("backend stopped sending events");
            break;
        };
        log_event(&event);
        *counts.entry(kind(&event)).or_insert(0) += 1;
        if let Err(error) = store.apply(event).await {
            tracing::warn!(%error, "couldn't store a backend event");
        }
    }
    Summary {
        counts,
        elapsed: start.elapsed(),
    }
}

/// Log the account and copy contacts and groups into the store.
async fn sync_directory<B: Backend>(backend: &B, store: &Store) -> anyhow::Result<()> {
    let account = backend.whoami().await?;
    let contacts = backend.contacts().await?;
    let groups = backend.groups().await?;
    tracing::info!(
        account = %account.service_id,
        contacts = contacts.len(),
        groups = groups.len(),
        capabilities = ?backend.capabilities(),
        "backend ready"
    );
    store.upsert_contacts(contacts).await?;
    store.upsert_groups(groups).await?;
    Ok(())
}

fn log_event(event: &BackendEvent) {
    match event {
        BackendEvent::Connection(state) => tracing::info!(?state, "connection"),
        BackendEvent::Error(error) => tracing::warn!(%error, "backend error"),
        BackendEvent::Message(_)
        | BackendEvent::DeliveryUpdate { .. }
        | BackendEvent::Reaction { .. }
        | BackendEvent::Edited { .. }
        | BackendEvent::Deleted { .. }
        | BackendEvent::Typing { .. }
        | BackendEvent::ContactUpdated(_)
        | BackendEvent::GroupUpdated(_) => tracing::debug!(kind = kind(event), "backend event"),
    }
}

fn kind(event: &BackendEvent) -> &'static str {
    match event {
        BackendEvent::Connection(_) => "connection",
        BackendEvent::Message(_) => "message",
        BackendEvent::DeliveryUpdate { .. } => "delivery",
        BackendEvent::Reaction { .. } => "reaction",
        BackendEvent::Edited { .. } => "edit",
        BackendEvent::Deleted { .. } => "delete",
        BackendEvent::Typing { .. } => "typing",
        BackendEvent::ContactUpdated(_) => "contact",
        BackendEvent::GroupUpdated(_) => "group",
        BackendEvent::Error(_) => "error",
    }
}

#[cfg(test)]
#[expect(
    clippy::disallowed_methods,
    reason = "#[tokio::test] expands to block_on; tests may block"
)]
mod tests {
    use faro_core::mock::{MockBackend, MockConfig};

    use super::*;

    // Real time, not a paused clock: the store does real I/O on SQLite's own
    // thread, and a paused clock would auto-advance while waiting for it.
    #[tokio::test]
    async fn runs_until_shutdown_and_stores_events() {
        let (backend, events) = MockBackend::new(&MockConfig::deterministic(1));
        let store = Store::open_in_memory().await.unwrap();
        // The mock connects at once and delivers its backlog at 400 ms.
        let shutdown = tokio::time::sleep(Duration::from_millis(1500));
        let summary = run(&backend, events, &store, shutdown).await;

        assert!(summary.elapsed >= Duration::from_millis(1500));
        assert!(summary.count("connection") >= 2);
        assert!(summary.count("message") >= 60, "the backlog");
        assert_eq!(summary.total(), summary.counts.values().sum::<u64>());

        // Everything was persisted: every message, and a conversation per contact.
        let stats = store.stats().await.unwrap();
        assert_eq!(stats.messages, summary.count("message"));
        assert!(stats.conversations >= 70 + 6, "cast contacts and groups");
    }

    #[tokio::test(start_paused = true)]
    async fn stops_when_the_backend_goes_away() {
        let (backend, _its_events) = MockBackend::new(&MockConfig::deterministic(1));
        let (gone, events) = MockBackend::new(&MockConfig::deterministic(1));
        drop(gone);
        // Never completes: only the closed channel can end the run.
        let store = Store::open_in_memory().await.unwrap();
        let summary = run(&backend, events, &store, std::future::pending()).await;
        assert_eq!(summary.total(), 0, "aborted before its first event");
    }

    #[test]
    fn summary_reads_well() {
        let mut summary = Summary {
            elapsed: Duration::from_secs(61),
            ..Summary::default()
        };
        assert_eq!(summary.to_string(), "0 events in 61s");
        summary.counts.insert("message", 3);
        summary.counts.insert("connection", 2);
        assert_eq!(
            summary.to_string(),
            "5 events in 61s (connection: 2, message: 3)"
        );
    }
}
