//! Sending messages (M4 task 3, D39). The app owns the `Sending` state (D31):
//! the message is shown and stored at once, then the backend's answer moves it
//! to `Sent` or `Failed`; receipts take it from there.

use faro_core::backend::{
    BackendError, BackendEvent, OutgoingAttachment, OutgoingEdit, OutgoingMessage,
};
use faro_core::domain::{
    AttachmentRef, AttachmentStatus, ConversationId, DeliveryStatus, Direction, Mention, Message,
    MessageBody, MessageRef, ServiceId, Timestamp,
};
use faro_core::paths;

use super::{on_backend, summary_entry};
use crate::app::clipboard::ClipboardError;
use crate::app::commands::Command;
use crate::app::event::Effect;
use crate::app::input::Input;
use crate::app::keymap::Action;
use crate::app::state::{Focus, Notice, Popup, StagedAttachment, State};

/// Signal's object-replacement placeholder for an inline mention (D72).
const MENTION_PLACEHOLDER: char = '\u{FFFC}';

/// Send what's in the input (and/or staged attachment) to the open conversation,
/// or submit an in-progress message edit. Blank input with no staged attachment
/// does nothing.
pub fn send(state: &mut State) -> Vec<Effect> {
    let Some(open) = state.open.as_mut() else {
        return vec![];
    };
    let text = open.input.text().trim().to_owned();
    if let Some(target_sent_at) = open.editing {
        return send_edit(state, target_sent_at, &text);
    }
    if text.is_empty() && open.staged_attachment.is_none() {
        return vec![];
    }
    let conversation = open.id;
    let quote = open.reply_to.take();
    let staged = open.staged_attachment.take();
    let known = std::mem::take(&mut open.mentions);
    open.mention_selected = 0;
    open.mention_dismissed_at = None;
    open.shortcode_selected = 0;
    open.shortcode_dismissed_at = None;
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
    let (body_text, mentions) = extract_mentions(&text, &known);
    let body = MessageBody::new(body_text.clone());
    let message = Message {
        conversation,
        sender: state.me,
        sent_at,
        direction: Direction::Outgoing(DeliveryStatus::Sending),
        body: (!body_text.is_empty()).then(|| body.clone()),
        mentions: mentions.clone(),
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
            mentions,
            quote,
            attachments: outgoing_attachments,
        },
    }));
    effects
}

/// Forward `text` and `source_attachments` as a new outgoing message to `conversation`.
pub fn forward(
    state: &mut State,
    conversation: ConversationId,
    text: &str,
    source_attachments: &[AttachmentRef],
) -> Vec<Effect> {
    if let Some(open) = state.open.as_mut().filter(|o| o.id == conversation) {
        open.scroll = None;
        open.unseen = 0;
    }
    let sent_at = next_sent_at(state.last_sent_at, wall_clock());
    state.last_sent_at = sent_at;
    let msg_ref = MessageRef {
        sender: state.me,
        sent_at,
    };
    let mut attachments = Vec::new();
    let mut outgoing_attachments = Vec::new();
    for (idx, src) in source_attachments.iter().enumerate() {
        let Some(src_path) = src.local_path() else {
            continue;
        };
        let file_name =
            paths::attachment_file_name(msg_ref, idx, src.file_name.as_deref(), &src.content_type);
        let dest = state.attachments_dir.join(file_name);
        attachments.push(AttachmentRef {
            file_name: src.file_name.clone(),
            size: src.size,
            width: src.width,
            height: src.height,
            status: AttachmentStatus::Downloaded(dest),
            voice_note: src.voice_note,
            duration_secs: src.duration_secs,
            waveform: src.waveform.clone(),
            ..AttachmentRef::remote(String::new(), src.content_type.clone())
        });
        outgoing_attachments.push(OutgoingAttachment {
            path: src_path.to_path_buf(),
            content_type: src.content_type.clone(),
            file_name: src.file_name.clone(),
            size: src.size.unwrap_or(0),
            width: src.width,
            height: src.height,
        });
    }
    let body = MessageBody::new(text);
    let message = Message {
        conversation,
        sender: state.me,
        sent_at,
        direction: Direction::Outgoing(DeliveryStatus::Sending),
        body: (!text.is_empty()).then(|| body.clone()),
        mentions: vec![],
        quote: None,
        attachments,
        reactions: vec![],
        edited_at: None,
        deleted: false,
    };
    state.notice = Some(Notice::info(format!(
        "Forwarded to {}",
        state.conversation_title(conversation)
    )));
    let mut effects = on_backend(state, BackendEvent::Message(message));
    effects.push(Effect::Backend(Command::Send {
        conversation,
        message: OutgoingMessage {
            sent_at,
            body,
            mentions: vec![],
            quote: None,
            attachments: outgoing_attachments,
        },
    }));
    effects
}

fn send_edit(state: &mut State, target_sent_at: Timestamp, text: &str) -> Vec<Effect> {
    if text.is_empty() {
        return vec![];
    }
    let Some(open) = state.open.as_mut() else {
        return vec![];
    };
    let conversation = open.id;
    let known = std::mem::take(&mut open.mentions);
    open.editing = None;
    open.mention_selected = 0;
    open.mention_dismissed_at = None;
    open.shortcode_selected = 0;
    open.shortcode_dismissed_at = None;
    let restored = state
        .summary(conversation)
        .and_then(|s| s.state.draft.as_ref())
        .map(|d| Input::new(d.expose()))
        .unwrap_or_default();
    if let Some(open) = state.open.as_mut() {
        open.input = restored;
    }
    let (body_text, mentions) = extract_mentions(text, &known);
    let body = MessageBody::new(body_text);
    let edited_at = next_sent_at(state.last_sent_at, wall_clock());
    state.last_sent_at = edited_at;
    let target = MessageRef {
        sender: state.me,
        sent_at: target_sent_at,
    };
    let mut effects = on_backend(
        state,
        BackendEvent::Edited {
            conversation,
            target,
            body: body.clone(),
            mentions: mentions.clone(),
            edited_at,
        },
    );
    effects.push(Effect::Backend(Command::Edit {
        conversation,
        target,
        edit: OutgoingEdit {
            edited_at,
            body,
            mentions,
        },
    }));
    effects
}

/// Replace `@<DisplayName>` occurrences (at the start of `text` or after
/// whitespace, and followed by end-of-string or a non-alphanumeric character)
/// with `U+FFFC` (`MENTION_PLACEHOLDER`), recording each `Mention`'s UTF-8 byte
/// offset and length.
pub fn extract_mentions(text: &str, known: &[(ServiceId, String)]) -> (String, Vec<Mention>) {
    if known.is_empty() || !text.contains('@') {
        return (text.to_owned(), Vec::new());
    }
    let mut ordered: Vec<&(ServiceId, String)> =
        known.iter().filter(|(_, name)| !name.is_empty()).collect();
    ordered.sort_by_key(|(_, name)| std::cmp::Reverse(name.len()));
    let mut out = String::with_capacity(text.len());
    let mut mentions = Vec::new();
    let mut pos = 0;
    while pos < text.len() {
        let rest = text.get(pos..).unwrap_or("");
        let at_word_start = pos == 0
            || text
                .get(..pos)
                .and_then(|s| s.chars().next_back())
                .is_some_and(char::is_whitespace);
        if at_word_start && let Some(after_at) = rest.strip_prefix('@') {
            let matched = ordered.iter().find(|(_, name)| {
                after_at.starts_with(name.as_str())
                    && after_at
                        .get(name.len()..)
                        .and_then(|s| s.chars().next())
                        .is_none_or(|c| !c.is_alphanumeric())
            });
            if let Some((who, name)) = matched {
                let start = out.len();
                out.push(MENTION_PLACEHOLDER);
                mentions.push(Mention {
                    who: *who,
                    start,
                    len: MENTION_PLACEHOLDER.len_utf8(),
                });
                pos += 1 + name.len();
                continue;
            }
        }
        let Some(ch) = rest.chars().next() else {
            break;
        };
        out.push(ch);
        pos += ch.len_utf8();
    }
    (out, mentions)
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

/// `Ctrl+V` (D91): ask kitty for the clipboard's image. The app loop pauses
/// input while it waits (D90), so this can't be pressed twice.
pub fn paste_image(state: &mut State) -> Vec<Effect> {
    let Some(open) = state.open.as_ref() else {
        state.notice = Some(Notice::info("Open a conversation to paste an image"));
        return vec![];
    };
    if state.pasting.is_some() {
        return vec![];
    }
    let conversation = open.id;
    state.pasting = Some(conversation);
    state.notice = Some(Notice::warning(
        "Waiting for kitty… allow the clipboard prompt",
    ));
    vec![Effect::ReadClipboardImage { conversation }]
}

/// The clipboard read is over: stage the image (never send it, D91), or say
/// why not.
pub fn on_clipboard_image(
    state: &mut State,
    conversation: ConversationId,
    result: Result<StagedAttachment, ClipboardError>,
) {
    state.pasting = None;
    let staged = match result {
        Ok(staged) => staged,
        Err(error) => {
            state.notice = Some(error.notice());
            return;
        }
    };
    let Some(open) = state.open.as_mut().filter(|open| open.id == conversation) else {
        state.notice = Some(Notice::warning(
            "The pasted image was dropped: the conversation changed",
        ));
        return;
    };
    open.staged_attachment = Some(staged);
    state.focus = Focus::Input;
    let view = state
        .keymap
        .hint(&[Action::ViewStagedImage], Focus::Input)
        .map(|key| format!("{key} shows it, "))
        .unwrap_or_default();
    state.notice = Some(Notice::info(format!(
        "Image pasted: {view}Enter sends it, Esc removes it"
    )));
}

/// `Ctrl+O` (D91): the staged image, full size.
pub fn view_staged(state: &mut State) {
    match state
        .open
        .as_ref()
        .and_then(|open| open.staged_attachment.as_ref())
    {
        Some(staged) if staged.content_type.starts_with("image/") => {
            state.popup = Some(Popup::StagedImage);
        }
        Some(_) => state.notice = Some(Notice::info("Only images can be shown")),
        None => state.notice = Some(Notice::info("No image is waiting to be sent")),
    }
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
    on_backend(
        state,
        BackendEvent::DeliveryUpdate {
            message,
            status,
            by: None,
            updated_at: None,
        },
    )
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
    use uuid::Uuid;

    #[test]
    fn sent_at_always_moves_forward() {
        let t = Timestamp::from_millis;
        assert_eq!(next_sent_at(t(100), t(200)), t(200));
        assert_eq!(next_sent_at(t(200), t(200)), t(201));
        // The clock went backwards.
        assert_eq!(next_sent_at(t(300), t(200)), t(301));
    }

    #[test]
    fn extract_mentions_replaces_known_names_with_placeholder() {
        let alice = ServiceId::Aci(Uuid::from_u128(1));
        let bob = ServiceId::Aci(Uuid::from_u128(2));
        let known = vec![(alice, "Alice".to_owned()), (bob, "Bob Smith".to_owned())];
        let (body, mentions) = extract_mentions("Hey @Alice, check with @Bob Smith!", &known);
        assert_eq!(body, "Hey \u{FFFC}, check with \u{FFFC}!");
        assert_eq!(
            mentions,
            vec![
                Mention {
                    who: alice,
                    start: 4,
                    len: 3,
                },
                Mention {
                    who: bob,
                    start: 20,
                    len: 3,
                },
            ]
        );
    }
}
