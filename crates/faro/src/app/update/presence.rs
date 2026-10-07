//! What we tell others (M4 task 6, D42): typing indicators while writing, and
//! read receipts once messages have actually been seen.

use faro_core::domain::{Direction, Message, MessageRef, TypingState};
use tokio::time::Instant;

use crate::app::commands::Command;
use crate::app::event::Effect;
use crate::app::state::{State, TYPING_PAUSE, TYPING_REFRESH, TypingSent};

fn typing(conversation: faro_core::domain::ConversationId, state: TypingState) -> Effect {
    Effect::Backend(Command::Typing {
        conversation,
        state,
    })
}

/// The input's text changed: "started typing" on the first edit and every
/// `TYPING_REFRESH` while it goes on; "stopped" once the input is empty.
pub fn on_edited(state: &mut State) -> Vec<Effect> {
    let Some(open) = state.open.as_mut() else {
        return vec![];
    };
    let id = open.id;
    if open.input.is_empty() {
        return match open.typing_sent.take() {
            Some(_) => vec![typing(id, TypingState::Stopped)],
            None => vec![],
        };
    }
    let now = Instant::now();
    match open.typing_sent.as_mut() {
        Some(sent) if now < sent.started + TYPING_REFRESH => {
            sent.last_edit = now;
            vec![]
        }
        Some(_) | None => {
            open.typing_sent = Some(TypingSent {
                started: now,
                last_edit: now,
            });
            vec![typing(id, TypingState::Started)]
        }
    }
}

/// "Stopped typing", if we said we were (switching away, closing, quitting).
pub fn stop_typing(state: &mut State) -> Vec<Effect> {
    let Some(open) = state.open.as_mut() else {
        return vec![];
    };
    match open.typing_sent.take() {
        Some(_) => vec![typing(open.id, TypingState::Stopped)],
        None => vec![],
    }
}

/// `TYPING_PAUSE` without an edit: stopped.
pub fn on_tick(state: &mut State, now: Instant) -> Vec<Effect> {
    let paused = state
        .open
        .as_ref()
        .and_then(|o| o.typing_sent)
        .is_some_and(|t| now >= t.last_edit + TYPING_PAUSE);
    if paused { stop_typing(state) } else { vec![] }
}

/// Remember `message` for a read receipt if it's an incoming message in the
/// open conversation. While it isn't being seen, it counts as unread and the
/// "new messages" rule goes above it (if there isn't one yet).
pub fn on_new_message(state: &mut State, message: &Message) {
    let seeing = state.seeing_open();
    let Some(open) = state.open.as_mut().filter(|o| o.id == message.conversation) else {
        return;
    };
    if message.direction != Direction::Incoming || message.deleted {
        return;
    }
    let reference = message.reference();
    if !open.unreceipted.contains(&reference) {
        open.unreceipted.push(reference);
    }
    if !seeing && open.unread_marker.is_none() {
        open.unread_marker = Some(reference);
    }
}

/// The open conversation's history arrived: its unread messages wait for
/// receipts.
pub fn on_history(state: &mut State, unread: &[MessageRef]) {
    if let Some(open) = state.open.as_mut() {
        for reference in unread {
            if !open.unreceipted.contains(reference) {
                open.unreceipted.push(*reference);
            }
        }
    }
}

/// If the open conversation is being seen, everything waiting is read: send
/// the receipts and clear the unread count (here and in the store).
pub fn catch_up(state: &mut State) -> Vec<Effect> {
    if !state.seeing_open() {
        return vec![];
    }
    let Some(open) = state.open.as_mut().filter(|o| !o.unreceipted.is_empty()) else {
        return vec![];
    };
    let id = open.id;
    let messages = std::mem::take(&mut open.unreceipted);
    if let Some(summary) = state.conversations.iter_mut().find(|c| c.id == id) {
        summary.unread = 0;
    }
    vec![
        Effect::MarkRead(id),
        Effect::Backend(Command::MarkRead(messages)),
    ]
}
