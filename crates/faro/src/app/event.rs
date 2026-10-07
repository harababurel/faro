//! What flows into the reducer (`AppEvent`) and out of it (`Effect`), D22.

use faro_core::backend::{BackendError, BackendEvent};
use faro_core::domain::{ConversationId, Message, MessageRef, Timestamp};
use faro_core::store::{ConversationState, ConversationSummary};

use super::commands::Command;
use super::state::Geometry;

#[derive(Debug)]
pub enum AppEvent {
    Terminal(crossterm::event::Event),
    Backend(BackendEvent),
    /// Startup data from the store.
    Loaded {
        conversations: Vec<ConversationSummary>,
        last_open: Option<ConversationId>,
    },
    MessagesLoaded {
        conversation: ConversationId,
        messages: Vec<Message>,
    },
    /// A page of messages older than the first one loaded.
    OlderLoaded {
        conversation: ConversationId,
        messages: Vec<Message>,
    },
    /// The backend finished (or gave up) sending our message `sent_at`.
    SendResult {
        sent_at: Timestamp,
        result: Result<(), BackendError>,
    },
    /// A frame was drawn; this is where things ended up.
    Drawn(Geometry),
    /// A store operation failed; the text is safe to show (no message content).
    StoreFailed(String),
    /// A deadline from `State::next_deadline` passed.
    Tick,
    /// SIGTERM or SIGHUP (e.g. the tmux pane was closed), or the terminal went away.
    Shutdown,
}

/// Work the reducer asks for; run by the effects worker (`effects.rs`, store
/// work) or the command task (`commands.rs`, `Effect::Backend`), whose results
/// come back as `AppEvent`s.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    /// Answered with `AppEvent::Loaded`.
    LoadConversations,
    /// Answered with `AppEvent::MessagesLoaded`.
    LoadMessages(ConversationId),
    /// Answered with `AppEvent::OlderLoaded`.
    LoadOlder {
        conversation: ConversationId,
        before: MessageRef,
    },
    Persist(Box<BackendEvent>),
    MarkRead(ConversationId),
    /// Store hide/archive/pin/mute/draft for a conversation.
    SaveConversationState(ConversationId, Box<ConversationState>),
    /// Reopen this conversation on the next start.
    RememberOpen(ConversationId),
    /// Ask the backend to do something; routed to the command task.
    Backend(Command),
}
