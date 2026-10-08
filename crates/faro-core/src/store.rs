//! faro's own database: message history, conversations and UI state (DESIGN §6).
//!
//! The store runs as one tokio task that owns the only connection (D22). The rest
//! of the app holds a cheap, cloneable [`Store`] handle and talks to the task over a
//! channel, so nothing outside this module ever waits on SQLite directly.
//!
//! Applying the same backend event twice is harmless: duplicates are ignored,
//! receipts never regress and only newer edits win.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::str::FromStr as _;

use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqliteSynchronous};
use tokio::sync::{mpsc, oneshot};

use crate::backend::BackendEvent;
use crate::domain::{
    Contact, ConversationId, Group, Message, MessageBody, MessageRef, PhoneNumber, ServiceId,
    Timestamp,
};
use crate::paths;

mod codec;
mod db;
#[cfg(test)]
mod tests;

use db::Db;

pub type Result<T, E = StoreError> = std::result::Result<T, E>;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("database migration failed: {0}")]
    Migration(#[from] sqlx::migrate::MigrateError),
    #[error("could not prepare the database file: {0}")]
    Io(#[from] std::io::Error),
    /// A stored value failed validation; names the column.
    #[error("corrupt value in the database: {0}")]
    Corrupt(&'static str),
    /// A value is too large to store; names what it was.
    #[error("value out of range: {0}")]
    OutOfRange(&'static str),
    /// The store task has stopped (closed, or failed to start).
    #[error("the store is closed")]
    Closed,
}

/// One row of the conversation list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationSummary {
    pub id: ConversationId,
    /// Contact or group name, if known. The UI falls back to the phone number or ID.
    pub title: Option<String>,
    /// Direct conversations only, if known.
    pub phone: Option<PhoneNumber>,
    /// Newest message's send time; `None` if there are no messages yet.
    pub last_activity: Option<Timestamp>,
    pub unread: u32,
    pub state: ConversationState,
}

/// Per-conversation UI state, local to this device.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConversationState {
    pub hidden: bool,
    pub archived: bool,
    /// Position among pinned conversations (lower first); `None` if not pinned.
    pub pinned: Option<u32>,
    pub muted_until: Option<Timestamp>,
    pub draft: Option<MessageBody>,
    /// On the allowlist: still shown while allowlist mode is on.
    pub allowed: bool,
    /// What notifies while not muted (D49).
    pub notify: NotifyLevel,
}

/// Which new messages in a conversation notify (bell, window title), D49.
/// A mute silences everything whatever the level.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum NotifyLevel {
    #[default]
    All,
    /// Only messages that mention us. Mentions aren't modelled until the
    /// Signal backend (M7), so for now nothing notifies.
    Mentions,
}

impl NotifyLevel {
    /// The database form.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Mentions => "mentions",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "all" => Some(Self::All),
            "mentions" => Some(Self::Mentions),
            _ => None,
        }
    }
}

impl ConversationState {
    /// `muted_until` for a mute with no end.
    pub const MUTED_FOREVER: Timestamp = Timestamp::from_millis(9_223_372_036_854_775_807);

    /// Whether the conversation is muted at `now`.
    pub fn muted_at(&self, now: Timestamp) -> bool {
        self.muted_until.is_some_and(|until| until > now)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StoreStats {
    pub conversations: u64,
    pub messages: u64,
}

/// Handle to the store task. Cheap to clone; the task stops when [`Store::close`]
/// is called or every handle is dropped.
#[derive(Debug, Clone)]
pub struct Store {
    requests: mpsc::Sender<Request>,
}

/// Requests queued beyond this make callers wait (backpressure), not grow memory.
const QUEUE: usize = 256;

type Reply<T> = oneshot::Sender<Result<T>>;

enum Request {
    Apply(Box<BackendEvent>, Reply<()>),
    UpsertContacts(Vec<Contact>, Reply<()>),
    UpsertGroups(Vec<Group>, Reply<()>),
    Conversations(Reply<Vec<ConversationSummary>>),
    Messages {
        conversation: ConversationId,
        before: Option<MessageRef>,
        limit: u32,
        reply: Reply<Vec<Message>>,
    },
    SetConversationState(ConversationId, Box<ConversationState>, Reply<()>),
    MarkRead(ConversationId, Reply<()>),
    FailUnsent(Reply<u64>),
    AppState(String, Reply<Option<String>>),
    SetAppState(String, String, Reply<()>),
    RemoveAppState(String, Reply<()>),
    Nicknames(Reply<HashMap<ServiceId, String>>),
    SetNickname(ServiceId, Option<String>, Reply<()>),
    Stats(Reply<StoreStats>),
    Close(Reply<()>),
}

impl Store {
    /// Open (creating if needed) the database at `path` and start the store task.
    /// The parent directory must exist. A new file is created as `0600`.
    pub async fn open(path: &Path) -> Result<Self> {
        let owned: PathBuf = path.to_owned();
        // SQLite creates its `-wal`/`-shm` files with the database file's mode, so
        // creating the file ourselves keeps all three private.
        tokio::task::spawn_blocking(move || paths::open_private_append(&owned))
            .await
            .map_err(|e| StoreError::Io(std::io::Error::other(e)))??;
        let options = SqliteConnectOptions::new()
            .filename(path)
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Normal)
            .foreign_keys(true);
        Self::start(&options).await
    }

    /// A throwaway in-memory store, for tests and the mock.
    pub async fn open_in_memory() -> Result<Self> {
        let options = SqliteConnectOptions::from_str("sqlite::memory:")?.foreign_keys(true);
        Self::start(&options).await
    }

    async fn start(options: &SqliteConnectOptions) -> Result<Self> {
        let db = Db::open(options).await?;
        let (requests, queue) = mpsc::channel(QUEUE);
        tokio::spawn(serve(db, queue));
        Ok(Self { requests })
    }

    /// Persist a backend event. Transient events (connection, typing, errors) are ignored.
    pub async fn apply(&self, event: BackendEvent) -> Result<()> {
        self.call(|reply| Request::Apply(Box::new(event), reply))
            .await
    }

    /// Insert or update contacts; each gets a (possibly empty) direct conversation.
    pub async fn upsert_contacts(&self, contacts: Vec<Contact>) -> Result<()> {
        self.call(|reply| Request::UpsertContacts(contacts, reply))
            .await
    }

    /// Insert or update groups, replacing their titles and member lists.
    pub async fn upsert_groups(&self, groups: Vec<Group>) -> Result<()> {
        self.call(|reply| Request::UpsertGroups(groups, reply))
            .await
    }

    /// All conversations: pinned first, then by most recent activity, then empty ones.
    pub async fn conversations(&self) -> Result<Vec<ConversationSummary>> {
        self.call(Request::Conversations).await
    }

    /// Up to `limit` messages older than `before` (the newest ones if `None`),
    /// oldest first. Pass the first message of a page as `before` to get the
    /// previous page.
    pub async fn messages(
        &self,
        conversation: ConversationId,
        before: Option<MessageRef>,
        limit: u32,
    ) -> Result<Vec<Message>> {
        self.call(|reply| Request::Messages {
            conversation,
            before,
            limit,
            reply,
        })
        .await
    }

    pub async fn set_conversation_state(
        &self,
        conversation: ConversationId,
        state: ConversationState,
    ) -> Result<()> {
        self.call(|reply| Request::SetConversationState(conversation, Box::new(state), reply))
            .await
    }

    pub async fn mark_read(&self, conversation: ConversationId) -> Result<()> {
        self.call(|reply| Request::MarkRead(conversation, reply))
            .await
    }

    /// Mark outgoing messages still "sending" as failed and return how many.
    /// Call on startup: a send that was in flight when faro stopped has an
    /// unknown outcome, and showing it as failed lets the user retry it.
    pub async fn fail_unsent(&self) -> Result<u64> {
        self.call(Request::FailUnsent).await
    }

    /// A small piece of app state (last open conversation, panel sizes…).
    pub async fn app_state(&self, key: &str) -> Result<Option<String>> {
        let key = key.to_owned();
        self.call(|reply| Request::AppState(key, reply)).await
    }

    pub async fn set_app_state(&self, key: &str, value: &str) -> Result<()> {
        let (key, value) = (key.to_owned(), value.to_owned());
        self.call(|reply| Request::SetAppState(key, value, reply))
            .await
    }

    /// Forget a piece of app state (e.g. a preference that moved to the
    /// config file).
    pub async fn remove_app_state(&self, key: &str) -> Result<()> {
        let key = key.to_owned();
        self.call(|reply| Request::RemoveAppState(key, reply)).await
    }

    pub async fn stats(&self) -> Result<StoreStats> {
        self.call(Request::Stats).await
    }

    /// Everyone the user gave a nickname (D65).
    pub async fn nicknames(&self) -> Result<HashMap<ServiceId, String>> {
        self.call(Request::Nicknames).await
    }

    /// Give someone a nickname, or take it away (`None`).
    pub async fn set_nickname(&self, id: ServiceId, nickname: Option<String>) -> Result<()> {
        self.call(|reply| Request::SetNickname(id, nickname, reply))
            .await
    }

    /// Stop the store task and close the database cleanly (checkpointing the WAL).
    /// Other handles get [`StoreError::Closed`] afterwards.
    pub async fn close(self) -> Result<()> {
        self.call(Request::Close).await
    }

    async fn call<T>(&self, request: impl FnOnce(Reply<T>) -> Request) -> Result<T> {
        let (reply, response) = oneshot::channel();
        self.requests
            .send(request(reply))
            .await
            .map_err(|_| StoreError::Closed)?;
        response.await.map_err(|_| StoreError::Closed)?
    }
}

/// The store task: handle requests one at a time until closed or every handle is gone.
async fn serve(mut db: Db, mut queue: mpsc::Receiver<Request>) {
    while let Some(request) = queue.recv().await {
        if let Some(reply) = handle(&mut db, request).await {
            queue.close();
            // The caller may have stopped waiting; nothing to do then.
            let _ = reply.send(db.close().await);
            return;
        }
    }
    if let Err(error) = db.close().await {
        tracing::warn!(%error, "closing the store failed");
    }
}

/// Handle one request. Returns the reply channel if it was `Close`.
async fn handle(db: &mut Db, request: Request) -> Option<Reply<()>> {
    // A caller that stopped waiting (dropped its future) just misses the reply,
    // hence the ignored `send` results.
    match request {
        Request::Apply(event, reply) => {
            let _ = reply.send(db.apply(*event).await);
        }
        Request::UpsertContacts(contacts, reply) => {
            let _ = reply.send(db.upsert_contacts(&contacts).await);
        }
        Request::UpsertGroups(groups, reply) => {
            let _ = reply.send(db.upsert_groups(&groups).await);
        }
        Request::Conversations(reply) => {
            let _ = reply.send(db.conversations().await);
        }
        Request::Messages {
            conversation,
            before,
            limit,
            reply,
        } => {
            let _ = reply.send(db.messages(conversation, before, limit).await);
        }
        Request::SetConversationState(conversation, state, reply) => {
            let _ = reply.send(db.set_conversation_state(conversation, &state).await);
        }
        Request::MarkRead(conversation, reply) => {
            let _ = reply.send(db.mark_read(conversation).await);
        }
        Request::FailUnsent(reply) => {
            let _ = reply.send(db.fail_unsent().await);
        }
        Request::AppState(key, reply) => {
            let _ = reply.send(db.app_state(&key).await);
        }
        Request::SetAppState(key, value, reply) => {
            let _ = reply.send(db.set_app_state(&key, &value).await);
        }
        Request::RemoveAppState(key, reply) => {
            let _ = reply.send(db.remove_app_state(&key).await);
        }
        Request::Stats(reply) => {
            let _ = reply.send(db.stats().await);
        }
        Request::Nicknames(reply) => {
            let _ = reply.send(db.nicknames().await);
        }
        Request::SetNickname(id, nickname, reply) => {
            let _ = reply.send(db.set_nickname(id, nickname.as_deref()).await);
        }
        Request::Close(reply) => return Some(reply),
    }
    None
}
