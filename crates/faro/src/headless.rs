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

/// Receive and log `backend`'s events until `shutdown` completes or the backend
/// stops.
pub async fn run<B: Backend>(
    backend: &B,
    mut events: BackendEvents,
    shutdown: impl Future<Output = ()>,
) -> Summary {
    let start = Instant::now();
    if let Err(error) = log_account(backend).await {
        tracing::warn!(%error, "couldn't read account, contacts or groups");
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
    }
    Summary {
        counts,
        elapsed: start.elapsed(),
    }
}

async fn log_account<B: Backend>(backend: &B) -> faro_core::backend::Result<()> {
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

    #[tokio::test(start_paused = true)]
    async fn runs_until_shutdown_and_counts_events() {
        let (backend, events) = MockBackend::new(&MockConfig::deterministic(1));
        let shutdown = tokio::time::sleep(Duration::from_secs(120));
        let summary = run(&backend, events, shutdown).await;

        assert_eq!(summary.elapsed, Duration::from_secs(120));
        assert!(summary.count("connection") >= 2);
        assert!(summary.count("message") >= 60, "backlog plus activity");
        assert_eq!(summary.total(), summary.counts.values().sum::<u64>());
    }

    #[tokio::test(start_paused = true)]
    async fn stops_when_the_backend_goes_away() {
        let (backend, _its_events) = MockBackend::new(&MockConfig::deterministic(1));
        let (gone, events) = MockBackend::new(&MockConfig::deterministic(1));
        drop(gone);
        // Never completes: only the closed channel can end the run.
        let summary = run(&backend, events, std::future::pending()).await;
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
