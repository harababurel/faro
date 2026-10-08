//! A fake Signal backend for development, tests and demos (DESIGN.md §4.3, D15, D31).
//!
//! It has a fixed cast of contacts and groups (`mock/world.rs`), delivers a backlog of
//! "queued" messages on connect, then keeps up scripted activity: incoming messages
//! with typing indicators, replies, reactions, edits, deletes, messages "sent from
//! the phone", profile changes, outages and decryption errors. Sent messages get
//! `Sent` → `Delivered` → `Read` updates and sometimes a reply.
//!
//! Everything random comes from one seeded generator (`mock/sim.rs`), and all timing
//! goes through tokio's clock. With [`MockConfig::deterministic`] and a paused clock
//! (`#[tokio::test(start_paused = true)]`), the event sequence is identical on every
//! run.

mod sim;
mod world;

use std::future::{Future, ready};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use tokio::sync::{Notify, mpsc};
use tokio::task::JoinHandle;
use tokio::time::{self, Instant};

use self::sim::{Millis, Sim, millis};
use self::world::World;
use crate::backend::{
    Account, Backend, BackendError, BackendEvent, BackendEvents, Capabilities, Capability,
    EVENT_CHANNEL_CAPACITY, OutgoingEdit, OutgoingMessage, ReactionChange, Result,
};
use crate::domain::{
    AttachmentRef, Contact, ConversationId, Group, MessageRef, Timestamp, TypingState,
};

/// Wall-clock time at which deterministic runs start: 2026-01-01T00:00:00Z.
pub const DETERMINISTIC_EPOCH: Timestamp = Timestamp::from_millis(1_767_225_600_000);

/// Where the mock's wall clock starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MockEpoch {
    /// The real current time: timestamps look normal in the UI.
    Now,
    /// A fixed time, so timestamps are reproducible.
    Fixed(Timestamp),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MockConfig {
    /// Seeds all scripted behaviour. The cast (IDs, names) never changes.
    pub seed: u64,
    pub epoch: MockEpoch,
    /// Mean time between scripted activities; `None` for a quiet mock that only
    /// reacts to commands. A few milliseconds gives a flood for stress tests.
    pub activity: Option<Duration>,
    /// Messages delivered right after connecting, dated over the past three days.
    pub backlog: usize,
    /// Scripted outages (the UI's offline state).
    pub disconnects: bool,
    /// Scripted unsolicited errors (`BackendEvent::Error`).
    pub errors: bool,
    pub capabilities: Capabilities,
}

impl MockConfig {
    /// Same events on every run (given a paused clock and the same commands).
    pub fn deterministic(seed: u64) -> Self {
        Self {
            seed,
            epoch: MockEpoch::Fixed(DETERMINISTIC_EPOCH),
            ..Self::default()
        }
    }
}

impl Default for MockConfig {
    fn default() -> Self {
        Self {
            seed: 0,
            epoch: MockEpoch::Now,
            activity: Some(Duration::from_secs(4)),
            backlog: 60,
            disconnects: true,
            errors: true,
            capabilities: Capabilities::NONE
                .with(Capability::Edit)
                .with(Capability::DeleteForEveryone)
                .with(Capability::Attachments),
        }
    }
}

/// See the [module docs](self).
pub struct MockBackend {
    shared: Arc<Shared>,
    capabilities: Capabilities,
    driver: JoinHandle<()>,
}

/// State shared between the backend (commands) and the driver task (events).
struct Shared {
    sim: Mutex<Sim>,
    /// Wakes the driver when a command schedules something.
    wake: Notify,
    /// Tokio time at `Millis` 0. Tokio's clock, so tests can pause and advance it.
    start: Instant,
}

impl Shared {
    fn sim(&self) -> MutexGuard<'_, Sim> {
        // A panic while holding the lock can't leave `Sim` half-updated in a way
        // that matters for a mock, so keep going.
        self.sim.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn now(&self) -> Millis {
        millis(self.start.elapsed())
    }

    /// Change the schedule and wake the driver to look at it.
    fn schedule<T>(&self, f: impl FnOnce(&mut Sim, Millis) -> T) -> T {
        let result = f(&mut self.sim(), self.now());
        self.wake.notify_one();
        result
    }
}

impl MockBackend {
    /// Starts the mock's driver task, so this must be called from within a Tokio
    /// runtime (`faro`'s main and `#[tokio::test]` both are); otherwise
    /// `tokio::spawn` panics.
    pub fn new(config: &MockConfig) -> (Self, BackendEvents) {
        let (events, receiver) = mpsc::channel(EVENT_CHANNEL_CAPACITY);
        let shared = Arc::new(Shared {
            sim: Mutex::new(Sim::new(config, World::build())),
            wake: Notify::new(),
            start: Instant::now(),
        });
        let driver = tokio::spawn(drive(Arc::clone(&shared), events));
        let backend = Self {
            shared,
            capabilities: config.capabilities,
            driver,
        };
        (backend, receiver)
    }

    /// Drop the connection now and restore it after `duration`. Ignored if an
    /// outage is already in progress.
    pub fn simulate_outage(&self, duration: Duration) {
        self.shared
            .schedule(|sim, now| sim.schedule_outage(now, duration));
    }

    /// Emit `error` as an unsolicited [`BackendEvent::Error`].
    pub fn inject_error(&self, error: BackendError) {
        self.shared
            .schedule(|sim, now| sim.schedule_error(now, error));
    }

    /// A round trip to the "server": fails if offline (before or after the
    /// simulated latency) or if `check` rejects the request.
    async fn request(&self, check: impl FnOnce(&Sim) -> Result<()>) -> Result<()> {
        let latency = {
            let mut sim = self.shared.sim();
            sim.require_online()?;
            check(&sim)?;
            sim.request_latency()
        };
        time::sleep(latency).await;
        self.shared.sim().require_online()
    }
}

impl Drop for MockBackend {
    fn drop(&mut self) {
        self.driver.abort();
    }
}

/// Sleep until the next scheduled entry (or a command's wake-up), then forward
/// what's due. Ends when the receiver is dropped or the backend aborts it.
async fn drive(shared: Arc<Shared>, events: mpsc::Sender<BackendEvent>) {
    loop {
        // Created before reading the schedule: `notify_one` stores a permit, so a
        // command that schedules something in between still wakes us.
        let wake = shared.wake.notified();
        let next = shared.sim().next_due();
        let Some(due) = next else {
            wake.await;
            continue;
        };
        tokio::select! {
            () = time::sleep_until(shared.start + Duration::from_millis(due)) => {}
            () = wake => continue,
        }
        let batch = shared.sim().pop_due(shared.now());
        for event in batch {
            if events.send(event).await.is_err() {
                return;
            }
        }
    }
}

impl Backend for MockBackend {
    fn capabilities(&self) -> Capabilities {
        self.capabilities
    }

    fn whoami(&self) -> impl Future<Output = Result<Account>> {
        ready(Ok(self.shared.sim().world().account.clone()))
    }

    fn contacts(&self) -> impl Future<Output = Result<Vec<Contact>>> {
        ready(Ok(self.shared.sim().world().contacts.clone()))
    }

    fn groups(&self) -> impl Future<Output = Result<Vec<Group>>> {
        ready(Ok(self.shared.sim().world().groups.clone()))
    }

    async fn send(&self, to: ConversationId, msg: OutgoingMessage) -> Result<()> {
        if !msg.attachments.is_empty() {
            self.capabilities.require(Capability::Attachments)?;
        }
        self.request(|_| Ok(())).await?;
        self.shared.schedule(|sim, now| sim.sent(now, to, msg))
    }

    async fn react(&self, _: ConversationId, _: MessageRef, _: ReactionChange) -> Result<()> {
        self.request(|_| Ok(())).await
    }

    async fn edit(&self, _: ConversationId, target: MessageRef, _: OutgoingEdit) -> Result<()> {
        self.capabilities.require(Capability::Edit)?;
        self.request(|sim| sim.require_own(target)).await
    }

    async fn delete_for_everyone(&self, _: ConversationId, target: MessageRef) -> Result<()> {
        self.capabilities.require(Capability::DeleteForEveryone)?;
        self.request(|sim| sim.require_own(target)).await
    }

    async fn mark_read(&self, _: &[MessageRef]) -> Result<()> {
        self.request(|_| Ok(())).await
    }

    async fn send_typing(&self, _: ConversationId, _: TypingState) -> Result<()> {
        self.request(|_| Ok(())).await
    }

    async fn fetch_attachment(&self, attachment: &AttachmentRef) -> Result<Vec<u8>> {
        self.capabilities.require(Capability::Attachments)?;
        self.request(|_| Ok(())).await?;
        world::attachment_bytes(&attachment.id.0).ok_or(BackendError::NotFound)
    }
}

#[cfg(test)]
#[expect(
    clippy::disallowed_methods,
    reason = "#[tokio::test] expands to block_on; tests may block"
)]
mod tests {
    use super::*;
    use crate::domain::{ConnectionState, DeliveryStatus, MessageBody};

    fn quiet() -> MockConfig {
        MockConfig {
            activity: None,
            backlog: 0,
            ..MockConfig::deterministic(0)
        }
    }

    async fn collect(events: &mut BackendEvents, n: usize) -> Vec<BackendEvent> {
        let mut out = Vec::with_capacity(n);
        while out.len() < n {
            out.push(events.recv().await.expect("mock keeps running"));
        }
        out
    }

    /// Receive until `pred` matches; return everything received, including the match.
    async fn until(
        events: &mut BackendEvents,
        pred: impl Fn(&BackendEvent) -> bool,
    ) -> Vec<BackendEvent> {
        let mut out = Vec::new();
        loop {
            let event = events.recv().await.expect("mock keeps running");
            let done = pred(&event);
            out.push(event);
            if done {
                return out;
            }
        }
    }

    fn is_connected(e: &BackendEvent) -> bool {
        *e == BackendEvent::Connection(ConnectionState::Connected)
    }

    fn hello(offset: u64) -> OutgoingMessage {
        OutgoingMessage {
            sent_at: Timestamp::from_millis(DETERMINISTIC_EPOCH.as_millis() + offset),
            body: MessageBody::new("hello"),
            mentions: vec![],
            quote: None,
            attachments: vec![],
        }
    }

    /// M1 acceptance: a given seed always produces the same event sequence.
    #[tokio::test(start_paused = true)]
    async fn same_seed_same_event_sequence() {
        let run = async |seed| {
            let (_backend, mut events) = MockBackend::new(&MockConfig::deterministic(seed));
            collect(&mut events, 400).await
        };
        let first = run(42).await;
        assert_eq!(first, run(42).await);
        assert_ne!(first, run(43).await);
    }

    #[tokio::test(start_paused = true)]
    async fn sent_message_progresses_to_read() {
        let (backend, mut events) = MockBackend::new(&quiet());
        until(&mut events, is_connected).await;

        let me = backend.whoami().await.unwrap().service_id;
        let alice = backend.contacts().await.unwrap()[0].service_id;
        let msg = hello(1);
        let target = MessageRef {
            sender: me,
            sent_at: msg.sent_at,
        };
        backend.send(alice.into(), msg).await.unwrap();

        let statuses: Vec<DeliveryStatus> = until(&mut events, |e| {
            matches!(
                e,
                BackendEvent::DeliveryUpdate {
                    message,
                    status: DeliveryStatus::Read,
                    ..
                } if *message == target
            )
        })
        .await
        .into_iter()
        .filter_map(|e| match e {
            BackendEvent::DeliveryUpdate {
                message, status, ..
            } if message == target => Some(status),
            BackendEvent::Connection(_)
            | BackendEvent::Message(_)
            | BackendEvent::DeliveryUpdate { .. }
            | BackendEvent::Reaction { .. }
            | BackendEvent::Edited { .. }
            | BackendEvent::Deleted { .. }
            | BackendEvent::Typing { .. }
            | BackendEvent::ReadElsewhere(_)
            | BackendEvent::ContactUpdated(_)
            | BackendEvent::GroupUpdated(_)
            | BackendEvent::Error(_) => None,
        })
        .collect();
        assert_eq!(
            statuses,
            [
                DeliveryStatus::Sent,
                DeliveryStatus::Delivered,
                DeliveryStatus::Read
            ]
        );
    }

    #[tokio::test(start_paused = true)]
    async fn commands_fail_while_offline_and_recover() {
        let (backend, mut events) = MockBackend::new(&quiet());
        let alice = backend.contacts().await.unwrap()[0].service_id;

        // Before the first connection completes.
        let err = backend.send(alice.into(), hello(1)).await.unwrap_err();
        assert!(matches!(err, BackendError::Network(_)) && err.is_transient());
        until(&mut events, is_connected).await;

        backend.simulate_outage(Duration::from_secs(5));
        let offline = BackendEvent::Connection(ConnectionState::Disconnected);
        until(&mut events, |e| *e == offline).await;
        let err = backend.send(alice.into(), hello(2)).await.unwrap_err();
        assert!(err.is_transient());

        until(&mut events, is_connected).await;
        backend.send(alice.into(), hello(3)).await.unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn injected_errors_are_emitted() {
        let (backend, mut events) = MockBackend::new(&quiet());
        until(&mut events, is_connected).await;
        backend.inject_error(BackendError::Unlinked);
        let error = BackendEvent::Error(BackendError::Unlinked);
        until(&mut events, |e| *e == error).await;
    }

    #[tokio::test(start_paused = true)]
    async fn capabilities_and_ownership_are_enforced() {
        let (backend, mut events) = MockBackend::new(&quiet());
        until(&mut events, is_connected).await;
        let me = backend.whoami().await.unwrap().service_id;
        let alice = backend.contacts().await.unwrap()[0].service_id;
        let mine = MessageRef {
            sender: me,
            sent_at: Timestamp::from_millis(1),
        };
        let theirs = MessageRef {
            sender: alice,
            ..mine
        };
        let edit = OutgoingEdit {
            edited_at: Timestamp::from_millis(2),
            body: MessageBody::new("fixed"),
            mentions: vec![],
        };
        let conv = ConversationId::Direct(alice);

        backend.edit(conv, mine, edit.clone()).await.unwrap();
        assert_eq!(
            backend.edit(conv, theirs, edit.clone()).await,
            Err(BackendError::NotFound)
        );
        backend.delete_for_everyone(conv, mine).await.unwrap();
        let known = world::mock_attachment(0).unwrap();
        let bytes = backend.fetch_attachment(&known).await.unwrap();
        assert!(bytes.starts_with(b"\x89PNG\r\n\x1a\n"));
        let unknown = AttachmentRef::remote("x", "image/png");
        assert_eq!(
            backend.fetch_attachment(&unknown).await,
            Err(BackendError::NotFound)
        );

        let (limited, _events) = MockBackend::new(&MockConfig {
            capabilities: Capabilities::NONE,
            ..quiet()
        });
        assert_eq!(
            limited.edit(conv, mine, edit).await,
            Err(BackendError::Unsupported(Capability::Edit))
        );
        assert_eq!(
            limited.fetch_attachment(&known).await,
            Err(BackendError::Unsupported(Capability::Attachments))
        );
    }

    #[tokio::test(start_paused = true)]
    async fn dropping_the_backend_closes_the_event_stream() {
        let (backend, mut events) = MockBackend::new(&MockConfig::deterministic(1));
        collect(&mut events, 10).await;
        drop(backend);
        while events.recv().await.is_some() {}
    }
}
