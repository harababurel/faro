//! What flows into the reducer (`AppEvent`) and out of it (`Effect`), D22.

use faro_core::backend::BackendEvent;
use faro_core::domain::{ConversationId, Message};
use faro_core::store::ConversationSummary;

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
    /// A store operation failed; the text is safe to show (no message content).
    StoreFailed(String),
    /// A deadline from `State::next_deadline` passed.
    Tick,
    /// SIGTERM or SIGHUP (e.g. the tmux pane was closed), or the terminal went away.
    Shutdown,
}

/// Work the reducer asks for; run by the effects worker (`effects.rs`), whose
/// results come back as `AppEvent`s.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    /// Answered with `AppEvent::Loaded`.
    LoadConversations,
    /// Answered with `AppEvent::MessagesLoaded`.
    LoadMessages(ConversationId),
    Persist(Box<BackendEvent>),
    MarkRead(ConversationId),
    /// Reopen this conversation on the next start.
    RememberOpen(ConversationId),
}
