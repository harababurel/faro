//! The boundary between faro and a Signal implementation (DESIGN.md §5, D6, D30).
//!
//! A backend runs in its own task. The app core never awaits it directly: it sends
//! commands to that task and receives [`BackendEvent`]s over a channel (D22).

mod error;
mod event;

use std::future::Future;
use std::path::PathBuf;

use tokio::sync::mpsc;

use crate::domain::{
    AttachmentRef, Contact, ConversationId, Group, Mention, MessageBody, MessageRef, PhoneNumber,
    Quote, ServiceId, Timestamp, TypingState,
};
pub use error::BackendError;
pub use event::BackendEvent;

pub type Result<T, E = BackendError> = std::result::Result<T, E>;

/// Capacity of the event channel. Bounded so a stalled consumer applies
/// backpressure to the backend task instead of growing memory without limit.
pub const EVENT_CHANNEL_CAPACITY: usize = 1024;

/// Receiving end of a backend's events. Each backend's constructor returns one
/// alongside the backend; there is exactly one consumer (the app core).
pub type BackendEvents = mpsc::Receiver<BackendEvent>;

/// A Signal implementation: presage (M7) or the mock.
///
/// Futures are not required to be `Send`, so implementations can wrap libraries
/// whose futures aren't (the backend task can run on a `LocalSet`). Callers that
/// need `Send` should drive the backend from a single dedicated task anyway.
///
/// Linking a new device is not part of this trait: it happens before a backend
/// exists and is backend-specific (decided in M7).
pub trait Backend {
    /// What this backend can do; the UI hides actions it can't perform.
    fn capabilities(&self) -> Capabilities;

    /// The linked account.
    fn whoami(&self) -> impl Future<Output = Result<Account>>;

    fn contacts(&self) -> impl Future<Output = Result<Vec<Contact>>>;

    fn groups(&self) -> impl Future<Output = Result<Vec<Group>>>;

    /// What the backend itself kept about these conversations (messages,
    /// reactions, edits, deletions, receipts), oldest first, so faro can
    /// pick up anything received while it wasn't running (D64). The store
    /// ignores what it already has. Backends that keep nothing return none.
    fn stored_events(
        &self,
        conversations: &[ConversationId],
    ) -> impl Future<Output = Result<Vec<BackendEvent>>> {
        let _ = conversations;
        std::future::ready(Ok(Vec::new()))
    }

    /// Send a message. `Ok` means it left this device; later progress
    /// (delivered, read) arrives as [`BackendEvent::DeliveryUpdate`] for
    /// `MessageRef { sender: me, sent_at: msg.sent_at }`.
    fn send(&self, to: ConversationId, msg: OutgoingMessage) -> impl Future<Output = Result<()>>;

    /// Set or take back this account's reaction to `target`.
    fn react(
        &self,
        conversation: ConversationId,
        target: MessageRef,
        change: ReactionChange,
    ) -> impl Future<Output = Result<()>>;

    /// Replace the text of one of this account's messages.
    /// Requires [`Capability::Edit`].
    fn edit(
        &self,
        conversation: ConversationId,
        target: MessageRef,
        edit: OutgoingEdit,
    ) -> impl Future<Output = Result<()>>;

    /// Requires [`Capability::DeleteForEveryone`].
    fn delete_for_everyone(
        &self,
        conversation: ConversationId,
        target: MessageRef,
    ) -> impl Future<Output = Result<()>>;

    /// Send read receipts for these messages.
    fn mark_read(&self, messages: &[MessageRef]) -> impl Future<Output = Result<()>>;

    fn send_typing(
        &self,
        to: ConversationId,
        state: TypingState,
    ) -> impl Future<Output = Result<()>>;

    /// Download an attachment's content; faro writes it to disk (D75).
    /// Requires [`Capability::Attachments`].
    fn fetch_attachment(&self, attachment: &AttachmentRef)
    -> impl Future<Output = Result<Vec<u8>>>;
}

/// The linked account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Account {
    pub service_id: ServiceId,
    pub phone: Option<PhoneNumber>,
}

/// A message to send. faro picks `sent_at` (as every Signal client does), so the
/// message's identity is known before sending: the local echo, delivery updates
/// and later reactions all refer to the same [`MessageRef`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutgoingMessage {
    pub sent_at: Timestamp,
    /// May be empty when there's an attachment (it's then the caption).
    pub body: MessageBody,
    pub mentions: Vec<Mention>,
    pub quote: Option<Quote>,
    /// Files to upload with it (D77); sending them requires
    /// [`Capability::Attachments`].
    pub attachments: Vec<OutgoingAttachment>,
}

/// A file to send: faro's own copy of it (D77).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutgoingAttachment {
    pub path: PathBuf,
    pub content_type: String,
    pub file_name: Option<String>,
    pub size: u64,
    pub width: Option<u32>,
    pub height: Option<u32>,
}

/// A change to this account's reaction to a message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReactionChange {
    /// React with this emoji, replacing any earlier reaction.
    Set(String),
    /// Take back the reaction with this emoji. Signal clients ignore a
    /// removal that doesn't name the emoji it removes (D73).
    Remove(String),
}

impl ReactionChange {
    /// The reaction left afterwards.
    pub fn emoji(&self) -> Option<&str> {
        match self {
            Self::Set(emoji) => Some(emoji),
            Self::Remove(_) => None,
        }
    }
}

/// New text for an edited message; `edited_at` is the edit's own timestamp.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutgoingEdit {
    pub edited_at: Timestamp,
    pub body: MessageBody,
    pub mentions: Vec<Mention>,
}

/// An optional backend feature. Core messaging (text, replies, receipts, typing,
/// reactions) is required of every backend and has no capability.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Capability {
    Edit,
    DeleteForEveryone,
    Attachments,
}

impl Capability {
    pub const ALL: [Self; 3] = [Self::Edit, Self::DeleteForEveryone, Self::Attachments];

    const fn bit(self) -> u8 {
        match self {
            Self::Edit => 1,
            Self::DeleteForEveryone => 1 << 1,
            Self::Attachments => 1 << 2,
        }
    }
}

/// The set of optional features a backend supports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Capabilities(u8);

impl Capabilities {
    pub const NONE: Self = Self(0);

    pub fn all() -> Self {
        Self::from_iter(Capability::ALL)
    }

    #[must_use]
    pub const fn with(self, capability: Capability) -> Self {
        Self(self.0 | capability.bit())
    }

    pub const fn supports(self, capability: Capability) -> bool {
        self.0 & capability.bit() != 0
    }

    /// `Ok` if supported, otherwise the error a backend should return.
    pub fn require(self, capability: Capability) -> Result<()> {
        if self.supports(capability) {
            Ok(())
        } else {
            Err(BackendError::Unsupported(capability))
        }
    }
}

impl FromIterator<Capability> for Capabilities {
    fn from_iter<I: IntoIterator<Item = Capability>>(iter: I) -> Self {
        iter.into_iter().fold(Self::NONE, Self::with)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capability_bits_are_distinct() {
        let all = Capabilities::all();
        for cap in Capability::ALL {
            assert!(all.supports(cap));
            let only = Capabilities::NONE.with(cap);
            for other in Capability::ALL {
                assert_eq!(only.supports(other), cap == other);
            }
        }
    }

    /// Minimal backend: shows the trait is implementable without `Send` futures
    /// and usable generically, and that the event channel works end to end.
    struct Stub {
        events: mpsc::Sender<BackendEvent>,
        me: ServiceId,
    }

    #[expect(
        clippy::unused_async_trait_impl,
        reason = "a stub mirrors how real backends write the trait: plain async fns"
    )]
    impl Backend for Stub {
        fn capabilities(&self) -> Capabilities {
            Capabilities::NONE
        }
        async fn whoami(&self) -> Result<Account> {
            Ok(Account {
                service_id: self.me,
                phone: None,
            })
        }
        async fn contacts(&self) -> Result<Vec<Contact>> {
            Ok(vec![])
        }
        async fn groups(&self) -> Result<Vec<Group>> {
            Ok(vec![])
        }
        async fn send(&self, _to: ConversationId, msg: OutgoingMessage) -> Result<()> {
            let message = MessageRef {
                sender: self.me,
                sent_at: msg.sent_at,
            };
            let status = crate::domain::DeliveryStatus::Sent;
            self.events
                .send(BackendEvent::DeliveryUpdate {
                    message,
                    status,
                    by: None,
                    updated_at: None,
                })
                .await
                .map_err(|_| BackendError::Network("event channel closed".into()))
        }
        async fn react(&self, _: ConversationId, _: MessageRef, _: ReactionChange) -> Result<()> {
            Ok(())
        }
        async fn edit(&self, _: ConversationId, _: MessageRef, _: OutgoingEdit) -> Result<()> {
            self.capabilities().require(Capability::Edit)
        }
        async fn delete_for_everyone(&self, _: ConversationId, _: MessageRef) -> Result<()> {
            self.capabilities().require(Capability::DeleteForEveryone)
        }
        async fn mark_read(&self, _: &[MessageRef]) -> Result<()> {
            Ok(())
        }
        async fn send_typing(&self, _: ConversationId, _: TypingState) -> Result<()> {
            Ok(())
        }
        async fn fetch_attachment(&self, _: &AttachmentRef) -> Result<Vec<u8>> {
            self.capabilities()
                .require(Capability::Attachments)
                .map(|()| Vec::new())
        }
    }

    async fn send_hello<B: Backend>(backend: &B, to: ConversationId) -> Result<()> {
        let msg = OutgoingMessage {
            sent_at: Timestamp::from_millis(42),
            body: MessageBody::new("hello"),
            mentions: vec![],
            quote: None,
            attachments: vec![],
        };
        backend.send(to, msg).await
    }

    #[tokio::test]
    #[expect(
        clippy::disallowed_methods,
        reason = "#[tokio::test] expands to block_on; tests may block"
    )]
    async fn stub_backend_round_trip() {
        let (tx, mut rx) = mpsc::channel(EVENT_CHANNEL_CAPACITY);
        let me = ServiceId::Aci(uuid::Uuid::from_u128(7));
        let backend = Stub { events: tx, me };

        assert_eq!(backend.whoami().await.unwrap().service_id, me);
        send_hello(&backend, ConversationId::Direct(me))
            .await
            .unwrap();
        assert_eq!(
            rx.recv().await,
            Some(BackendEvent::DeliveryUpdate {
                message: MessageRef {
                    sender: me,
                    sent_at: Timestamp::from_millis(42)
                },
                status: crate::domain::DeliveryStatus::Sent,
                by: None,
                updated_at: None,
            })
        );

        let target = MessageRef {
            sender: me,
            sent_at: Timestamp::from_millis(42),
        };
        let err = backend
            .delete_for_everyone(ConversationId::Direct(me), target)
            .await
            .unwrap_err();
        assert_eq!(
            err,
            BackendError::Unsupported(Capability::DeleteForEveryone)
        );
        assert!(!err.is_transient());
    }

    #[test]
    fn require_reports_the_missing_capability() {
        let caps = Capabilities::NONE.with(Capability::Edit);
        assert!(caps.require(Capability::Edit).is_ok());
        assert!(matches!(
            caps.require(Capability::Attachments),
            Err(BackendError::Unsupported(Capability::Attachments))
        ));
    }
}
