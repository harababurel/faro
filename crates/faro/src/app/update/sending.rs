//! Sending messages (M4 task 3, D39). The app owns the `Sending` state (D31):
//! the message is shown and stored at once, then the backend's answer moves it
//! to `Sent` or `Failed`; receipts take it from there.

use faro_core::backend::{BackendError, BackendEvent, OutgoingMessage};
use faro_core::domain::{
    ConversationId, DeliveryStatus, Direction, Message, MessageBody, MessageRef, Timestamp,
};

use super::{on_backend, summary_entry};
use crate::app::commands::Command;
use crate::app::event::Effect;
use crate::app::input::Input;
use crate::app::state::{Notice, State};

/// Send what's in the input to the open conversation. Blank input does nothing.
pub fn send(state: &mut State) -> Vec<Effect> {
    let Some(open) = state.open.as_mut() else {
        return vec![];
    };
    let text = open.input.text().trim().to_owned();
    if text.is_empty() {
        return vec![];
    }
    let conversation = open.id;
    let quote = open.reply_to.take();
    open.input = Input::default();
    open.draft_edited = None;
    // Jump to the newest message, where this one appears.
    open.scroll = None;
    open.unseen = 0;

    let sent_at = next_sent_at(state.last_sent_at, wall_clock());
    state.last_sent_at = sent_at;
    let body = MessageBody::new(text);
    let message = Message {
        conversation,
        sender: state.me,
        sent_at,
        direction: Direction::Outgoing(DeliveryStatus::Sending),
        body: Some(body.clone()),
        quote: quote.clone(),
        attachments: vec![],
        reactions: vec![],
        edited_at: None,
        deleted: false,
    };
    let mut effects = clear_draft(state, conversation);
    // Shown and stored like any new message (`Persist` runs before later receipts).
    effects.extend(on_backend(state, BackendEvent::Message(message)));
    effects.push(Effect::Backend(Command::Send {
        conversation,
        message: OutgoingMessage {
            sent_at,
            body,
            quote,
        },
    }));
    effects
}

/// The backend's answer: `Sent`, or `Failed` with a notice.
pub fn on_send_result(
    state: &mut State,
    sent_at: Timestamp,
    result: Result<(), BackendError>,
) -> Vec<Effect> {
    let status = match result {
        Ok(()) => DeliveryStatus::Sent,
        Err(error) => {
            state.notice = Some(Notice::error(format!("Message not sent: {error}")));
            DeliveryStatus::Failed
        }
    };
    let message = MessageRef {
        sender: state.me,
        sent_at,
    };
    on_backend(state, BackendEvent::DeliveryUpdate { message, status })
}

/// A sent message's draft is gone; store that if there was one.
fn clear_draft(state: &mut State, conversation: ConversationId) -> Vec<Effect> {
    match summary_entry(state, conversation) {
        Some(summary) if summary.state.draft.is_some() => {
            summary.state.draft = None;
            vec![Effect::SaveConversationState(
                conversation,
                Box::new(summary.state.clone()),
            )]
        }
        Some(_) | None => vec![],
    }
}

/// Now, but always after the previous send.
fn next_sent_at(last: Timestamp, now: Timestamp) -> Timestamp {
    if now > last {
        now
    } else {
        Timestamp::from_millis(last.as_millis().saturating_add(1))
    }
}

/// Reading the clock is not I/O; this keeps the reducer synchronous.
pub(super) fn wall_clock() -> Timestamp {
    let ms = jiff::Timestamp::now().as_millisecond();
    Timestamp::from_millis(u64::try_from(ms).unwrap_or(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sent_at_always_moves_forward() {
        let t = Timestamp::from_millis;
        assert_eq!(next_sent_at(t(100), t(200)), t(200));
        assert_eq!(next_sent_at(t(200), t(200)), t(201));
        // The clock went backwards.
        assert_eq!(next_sent_at(t(300), t(200)), t(301));
    }
}
