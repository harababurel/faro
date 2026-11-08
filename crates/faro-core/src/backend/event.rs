use super::{BackendError, IdentityUpdate};
use crate::domain::{
    ConnectionState, Contact, ConversationId, DeliveryStatus, Group, Mention, Message, MessageBody,
    MessageRef, ServiceId, Timestamp, TypingState,
};

/// An action taken on a message-request or conversation block state (D176).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageRequestAction {
    /// Accept the message request into the main conversation list.
    Accept,
    /// Delete the conversation and its messages without blocking.
    Delete,
    /// Block the conversation without deleting its history.
    Block,
    /// Block the conversation and delete its local message history.
    BlockAndDelete,
    /// Report the conversation as spam without blocking.
    Spam,
    /// Block the conversation, report it as spam, and delete its history.
    BlockAndSpam,
    /// Unblock a previously blocked conversation.
    Unblock,
}

/// Something that happened on the Signal side, delivered to the app core.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackendEvent {
    Connection(ConnectionState),

    /// A new message: incoming, or sent by this account from another device
    /// (a sync message, with `Direction::Outgoing`).
    Message(Message),

    /// Progress of one of this account's messages. Apply with
    /// [`DeliveryStatus::update`] so late or duplicate receipts can't regress it.
    /// `by` and `updated_at` record who sent a delivery or read receipt and when
    /// (`None` for local `Sending`/`Sent`/`Failed` transitions, D85).
    DeliveryUpdate {
        message: MessageRef,
        status: DeliveryStatus,
        by: Option<ServiceId>,
        updated_at: Option<Timestamp>,
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
        /// Replace the message's mentions (D72).
        mentions: Vec<Mention>,
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

    /// A contact's identity key (60-digit safety number) or verification
    /// state was learned or changed (D163).
    IdentityUpdated(IdentityUpdate),

    /// A conversation's disappearing-message timer was set (`Some(seconds)`)
    /// or turned off (`None`) (D165).
    DisappearingTimer {
        conversation: ConversationId,
        seconds: Option<u32>,
    },

    /// The full set of blocked conversations synced from another device (D176).
    BlockedSynced {
        conversations: Vec<ConversationId>,
    },

    /// Another of our linked devices responded to a message request or
    /// changed block state (D176).
    MessageRequestResponded {
        conversation: ConversationId,
        action: MessageRequestAction,
    },

    /// A problem that didn't come from a request, e.g. a message that failed to
    /// decrypt or the device being unlinked.
    Error(BackendError),
}
