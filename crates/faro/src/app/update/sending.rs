//! Sending messages (M4 task 3, D39). The app owns the `Sending` state (D31):
//! the message is shown and stored at once, then the backend's answer moves it
//! to `Sent` or `Failed`; receipts take it from there.

use faro_core::backend::{BackendError, BackendEvent, OutgoingAttachment, OutgoingMessage};
use faro_core::domain::{
    AttachmentRef, AttachmentStatus, ConversationId, DeliveryStatus, Direction, Message,
    MessageBody, MessageRef, Timestamp,
};
use faro_core::paths;

use super::{on_backend, summary_entry};
use crate::app::commands::Command;
use crate::app::event::Effect;
use crate::app::input::Input;
use crate::app::state::{Focus, Notice, StagedAttachment, State};

/// Send what's in the input (and/or staged attachment) to the open conversation.
/// Blank input with no staged attachment does nothing.
pub fn send(state: &mut State) -> Vec<Effect> {
    let Some(open) = state.open.as_mut() else {
        return vec![];
    };
    let text = open.input.text().trim().to_owned();
    if text.is_empty() && open.staged_attachment.is_none() {
        return vec![];
    }
    let conversation = open.id;
    let quote = open.reply_to.take();
    let staged = open.staged_attachment.take();
    open.input = Input::default();
    open.draft_edited = None;
    // The message itself ends our typing indicator; no "stopped" needed.
    open.typing_sent = None;
    // Jump to the newest message, where this one appears.
    open.scroll = None;
    open.unseen = 0;

    let sent_at = next_sent_at(state.last_sent_at, wall_clock());
    state.last_sent_at = sent_at;
    let msg_ref = MessageRef {
        sender: state.me,
        sent_at,
    };
    let (attachments, outgoing_attachments) = match staged {
        Some(staged) => {
            let file_name =
                paths::attachment_file_name(msg_ref, 0, Some(&staged.name), &staged.content_type);
            let dest = state.attachments_dir.join(file_name);
            let att = AttachmentRef {
                file_name: Some(staged.name.clone()),
                size: Some(staged.size),
                width: staged.width,
                height: staged.height,
                status: AttachmentStatus::Downloaded(dest),
                ..AttachmentRef::remote(String::new(), staged.content_type.clone())
            };
            let out = OutgoingAttachment {
                path: staged.path,
                content_type: staged.content_type,
                file_name: Some(staged.name),
                size: staged.size,
                width: staged.width,
                height: staged.height,
            };
            (vec![att], vec![out])
        }
        None => (vec![], vec![]),
    };
    let body = MessageBody::new(text.clone());
    let message = Message {
        conversation,
        sender: state.me,
        sent_at,
        direction: Direction::Outgoing(DeliveryStatus::Sending),
        body: (!text.is_empty()).then(|| body.clone()),
        mentions: vec![],
        quote: quote.clone(),
        attachments,
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
            attachments: outgoing_attachments,
        },
    }));
    effects
}

/// Result of `Effect::StageFile` (D77): attach to the open conversation or show
/// an error notice (e.g. > 100 MB).
pub fn on_file_staged(state: &mut State, result: Result<StagedAttachment, String>) {
    match result {
        Ok(staged) => {
            if let Some(open) = state.open.as_mut() {
                open.staged_attachment = Some(staged);
                state.focus = Focus::Input;
            }
        }
        Err(error) => {
            state.notice = Some(Notice::error(format!("Couldn't attach file: {error}")));
        }
    }
}

/// Result of `Effect::CheckPaste` (D77): if the pasted path is a regular file on
/// disk, stage it; otherwise insert the raw text into the input as normal.
pub fn on_paste_checked(
    state: &mut State,
    raw: &str,
    staged: Option<StagedAttachment>,
) -> Vec<Effect> {
    if let Some(staged) = staged {
        if let Some(open) = state.open.as_mut() {
            open.staged_attachment = Some(staged);
            state.focus = Focus::Input;
        }
        return vec![];
    }
    super::navigation::insert_pasted_text(state, raw);
    super::presence::on_edited(state)
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
pub fn wall_clock() -> Timestamp {
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
