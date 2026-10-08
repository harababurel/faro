use super::BackendError;
use crate::domain::{
    ConnectionState, Contact, ConversationId, DeliveryStatus, Group, Message, MessageBody,
    MessageRef, ServiceId, Timestamp, TypingState,
};

/// Something that happened on the Signal side, delivered to the app core.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackendEvent {
    Connection(ConnectionState),

    /// A new message: incoming, or sent by this account from another device
    /// (a sync message, with `Direction::Outgoing`).
    Message(Message),

    /// Progress of one of this account's messages. Apply with
    /// [`DeliveryStatus::update`] so late or duplicate receipts can't regress it.
    DeliveryUpdate {
        message: MessageRef,
        status: DeliveryStatus,
    },

    /// `sender` set (`Some`) or removed (`None`) their reaction to `target`.
    Reaction {
        conversation: ConversationId,
        target: MessageRef,
        sender: ServiceId,
        emoji: Option<String>,
    },

    Edited {
        conversation: ConversationId,
        target: MessageRef,
        body: MessageBody,
        edited_at: Timestamp,
    },

    Deleted {
        conversation: ConversationId,
        target: MessageRef,
    },

    Typing {
        conversation: ConversationId,
        sender: ServiceId,
        state: TypingState,
    },

    /// This account read these incoming messages on another device (a read
    /// sync from the phone, D70). As in Signal, each one also means that
    /// everything before it in its conversation was read. The conversation
    /// isn't given: the store finds it from the message, and messages it
    /// doesn't have are ignored.
    ReadElsewhere(Vec<MessageRef>),

    /// A contact was added or changed (name, profile, phone).
    ContactUpdated(Contact),

    /// A group was added or changed (title, members).
    GroupUpdated(Group),

    /// A problem that didn't come from a request, e.g. a message that failed to
    /// decrypt or the device being unlinked.
    Error(BackendError),
}
