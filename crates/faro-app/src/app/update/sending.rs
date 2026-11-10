//! Sending messages (M4 task 3, D39). The app owns the `Sending` state (D31):
//! the message is shown and stored at once, then the backend's answer moves it
//! to `Sent` or `Failed`; receipts take it from there.

use faro_core::backend::{
    BackendError, BackendEvent, OutgoingAttachment, OutgoingEdit, OutgoingMessage,
};
use faro_core::domain::{
    AttachmentRef, AttachmentStatus, ConversationId, DeliveryStatus, Direction, Mention, Message,
    MessageBody, MessageRef, ServiceId, StyleSpan, Timestamp,
};
use faro_core::file_names;

use super::{on_backend, summary_entry};
use crate::app::event::ClipboardError;
use crate::app::event::Command;
use crate::app::event::Effect;
use crate::app::input::Input;
use crate::app::keymap::Action;
use crate::app::state::{Focus, Notice, Popup, StagedAttachment, State, TerminalName};

/// Signal's object-replacement placeholder for an inline mention (D72).
const MENTION_PLACEHOLDER: char = '\u{FFFC}';

/// Block outgoing sends if a participant in `conversation` has an unacknowledged
/// safety-number change (D163), or if the conversation is blocked (D176).
pub fn block_if_key_changed(state: &mut State, conversation: ConversationId) -> bool {
    if state.summary(conversation).is_some_and(|s| s.state.blocked) {
        state.notice = Some(Notice::warning(format!(
            "{} is blocked — unblock before sending",
            state.conversation_title(conversation)
        )));
        return true;
    }
    let Some(&who) = state.changed_identities(conversation).first() else {
        return false;
    };
    state.notice = Some(Notice::warning(format!(
        "Safety number with {} changed — verify or approve before sending",
        state.person_name(who)
    )));
    true
}

/// Send what's in the input (and/or staged attachment) to the open conversation,
/// execute a composer `/command`, or submit an in-progress message edit. Blank
/// input with no staged attachment does nothing.
pub fn send(state: &mut State) -> Vec<Effect> {
    let Some(open) = state.open.as_ref() else {
        return vec![];
    };
    if let Some(target_sent_at) = open.editing {
        return send_edit(state, target_sent_at);
    }
    if crate::app::slash::is_slash_input(open.input.text().trim()) {
        return crate::app::slash::execute_from_input(state);
    }
    send_message(state)
}

/// Send the current composer text (and/or staged attachment) as an outgoing
/// message, unescaping a leading `//` into `/`.
pub fn send_message(state: &mut State) -> Vec<Effect> {
    let Some(open) = state.open.as_ref() else {
        return vec![];
    };
    let conversation = open.id;
    let raw = open.input.text();
    let trimmed_start = raw.trim_start();
    let raw_trimmed = trimmed_start.trim_end();
    let unescaped_slash = raw_trimmed.starts_with("//");
    let text = raw_trimmed
        .strip_prefix("//")
        .map_or_else(|| raw_trimmed.to_owned(), |rest| format!("/{rest}"));
    let trim_offset = (raw.len() - trimmed_start.len()) + usize::from(unescaped_slash);
    let input_styles = shift_styles(open.input.styles(), trim_offset, text.len());
    let has_staged = open.staged_attachment.is_some();
    if text.is_empty() && !has_staged {
        return vec![];
    }
    if block_if_key_changed(state, conversation) {
        return vec![];
    }
    let Some(open) = state.open.as_mut() else {
        return vec![];
    };
    let quote = open.reply_to.take();
    let staged = open.staged_attachment.take();
    let known = std::mem::take(&mut open.mentions);
    reset_composer_after_send(open);
    state.since_you_left = None;

    let sent_at = next_sent_at(state.last_sent_at, wall_clock());
    state.last_sent_at = sent_at;
    let msg_ref = MessageRef {
        sender: state.me,
        sent_at,
    };
    let (attachments, outgoing_attachments) = staged.map_or_else(
        || (vec![], vec![]),
        |s| stage_attachment(s, msg_ref, &state.attachments_dir),
    );
    let (body, mentions) = extract_mentions_and_styles(&text, &input_styles, &known);
    let expires_in_secs = state
        .summary(conversation)
        .and_then(|s| s.disappearing_timer);
    let message = Message {
        conversation,
        sender: state.me,
        sent_at,
        direction: Direction::Outgoing(DeliveryStatus::Sending),
        body: (!body.expose().is_empty()).then(|| body.clone()),
        mentions: mentions.clone(),
        quote: quote.clone(),
        previews: vec![],
        attachments,
        reactions: vec![],
        edited_at: None,
        deleted: false,
        expires_in_secs,
        expire_started_at: expires_in_secs.map(|_| sent_at),
        system_event: None,
    };
    let was_request = state.summary(conversation).is_some_and(|s| s.state.request);
    let mut effects = clear_draft(state, conversation);
    if was_request {
        if let Some(summary) = summary_entry(state, conversation) {
            summary.state.request = false;
            effects.push(Effect::SaveConversationState(
                conversation,
                Box::new(summary.state.clone()),
            ));
        }
        if state.list.view == crate::app::state::SidebarView::Archive {
            state.list.view = crate::app::state::SidebarView::Chats;
        }
        effects.push(Effect::Backend(Command::RespondMessageRequest {
            conversation,
            action: faro_core::backend::MessageRequestAction::Accept,
        }));
    }
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
            expires_in_secs,
        },
    }));
    effects
}

fn reset_composer_after_send(open: &mut crate::app::state::OpenConversation) {
    open.mention_selected = 0;
    open.mention_dismissed_at = None;
    open.shortcode_selected = 0;
    open.shortcode_dismissed_at = None;
    open.slash_selected = 0;
    open.slash_dismissed_for = None;
    open.input = Input::default();
    open.draft_edited = None;
    // The message itself ends our typing indicator; no "stopped" needed.
    open.typing_sent = None;
    // Jump to the newest message, where this one appears.
    open.scroll = None;
    open.restored_anchor = None;
    open.unseen = 0;
}

fn stage_attachment(
    staged: crate::app::state::StagedAttachment,
    msg_ref: MessageRef,
    attachments_dir: &std::path::Path,
) -> (Vec<AttachmentRef>, Vec<OutgoingAttachment>) {
    let file_name =
        file_names::attachment_file_name(msg_ref, 0, Some(&staged.name), &staged.content_type);
    let dest = attachments_dir.join(file_name);
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

/// Forward `text` and `source_attachments` as a new outgoing message to `conversation`.
pub fn forward(
    state: &mut State,
    conversation: ConversationId,
    text: &str,
    source_attachments: &[AttachmentRef],
) -> Vec<Effect> {
    if block_if_key_changed(state, conversation) {
        return vec![];
    }
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
        let file_name = file_names::attachment_file_name(
            msg_ref,
            idx,
            src.file_name.as_deref(),
            &src.content_type,
        );
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
    let expires_in_secs = state
        .summary(conversation)
        .and_then(|s| s.disappearing_timer);
    let message = Message {
        conversation,
        sender: state.me,
        sent_at,
        direction: Direction::Outgoing(DeliveryStatus::Sending),
        body: (!text.is_empty()).then(|| body.clone()),
        mentions: vec![],
        quote: None,
        previews: vec![],
        attachments,
        reactions: vec![],
        edited_at: None,
        deleted: false,
        expires_in_secs,
        expire_started_at: expires_in_secs.map(|_| sent_at),
        system_event: None,
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
            expires_in_secs,
        },
    }));
    effects
}

fn send_edit(state: &mut State, target_sent_at: Timestamp) -> Vec<Effect> {
    let Some(open) = state.open.as_ref() else {
        return vec![];
    };
    let raw = open.input.text();
    let trimmed_start = raw.trim_start();
    let text = trimmed_start.trim_end().to_owned();
    if text.is_empty() {
        return vec![];
    }
    let trim_offset = raw.len() - trimmed_start.len();
    let input_styles = shift_styles(open.input.styles(), trim_offset, text.len());
    let conversation = open.id;
    if block_if_key_changed(state, conversation) {
        return vec![];
    }
    let Some(open) = state.open.as_mut() else {
        return vec![];
    };
    let known = std::mem::take(&mut open.mentions);
    open.editing = None;
    open.mention_selected = 0;
    open.mention_dismissed_at = None;
    open.shortcode_selected = 0;
    open.shortcode_dismissed_at = None;
    open.slash_selected = 0;
    open.slash_dismissed_for = None;
    let restored = state
        .summary(conversation)
        .and_then(|s| s.state.draft.as_ref())
        .map(|d| Input::with_styles(d.expose(), d.styles().to_vec()))
        .unwrap_or_default();
    if let Some(open) = state.open.as_mut() {
        open.input = restored;
    }
    let (body, mentions) = extract_mentions_and_styles(&text, &input_styles, &known);
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

fn shift_styles(styles: &[StyleSpan], trim_offset: usize, max_len: usize) -> Vec<StyleSpan> {
    styles
        .iter()
        .filter_map(|span| {
            let start = span.start.saturating_sub(trim_offset).min(max_len);
            let end = (span.start + span.len)
                .saturating_sub(trim_offset)
                .min(max_len);
            (end > start).then_some(StyleSpan {
                start,
                len: end - start,
                style: span.style,
            })
        })
        .collect()
}

/// Replace `@<DisplayName>` occurrences (at the start of `text` or after
/// whitespace, and followed by end-of-string or a non-alphanumeric character)
/// with `U+FFFC` (`MENTION_PLACEHOLDER`), recording each `Mention`'s UTF-8 byte
/// offset and length.
#[cfg(test)]
pub fn extract_mentions(text: &str, known: &[(ServiceId, String)]) -> (String, Vec<Mention>) {
    let (body, mentions) = extract_mentions_and_styles(text, &[], known);
    (body.expose().to_owned(), mentions)
}

/// Like [`extract_mentions`], while also adjusting `styles` byte offsets across
/// any `@<DisplayName>` → `U+FFFC` replacements (D177).
pub fn extract_mentions_and_styles(
    text: &str,
    styles: &[StyleSpan],
    known: &[(ServiceId, String)],
) -> (MessageBody, Vec<Mention>) {
    if known.is_empty() || !text.contains('@') {
        return (MessageBody::with_styles(text, styles.to_vec()), Vec::new());
    }
    let mut ordered: Vec<&(ServiceId, String)> =
        known.iter().filter(|(_, name)| !name.is_empty()).collect();
    ordered.sort_by_key(|(_, name)| std::cmp::Reverse(name.len()));
    let mut out = String::with_capacity(text.len());
    let mut mentions = Vec::new();
    let mut byte_map = vec![0usize; text.len() + 1];
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
                let tok_len = 1 + name.len();
                if let Some(slots) = byte_map.get_mut(pos..pos + tok_len) {
                    slots.fill(start);
                }
                out.push(MENTION_PLACEHOLDER);
                mentions.push(Mention {
                    who: *who,
                    start,
                    len: MENTION_PLACEHOLDER.len_utf8(),
                });
                pos += tok_len;
                if let Some(slot) = byte_map.get_mut(pos) {
                    *slot = out.len();
                }
                continue;
            }
        }
        let Some(ch) = rest.chars().next() else {
            break;
        };
        let start = out.len();
        let ch_len = ch.len_utf8();
        for i in 0..ch_len {
            if let Some(slot) = byte_map.get_mut(pos + i) {
                *slot = start + i;
            }
        }
        out.push(ch);
        pos += ch_len;
        if let Some(slot) = byte_map.get_mut(pos) {
            *slot = out.len();
        }
    }
    let mapped_styles = styles
        .iter()
        .filter_map(|span| {
            let start = *byte_map.get(span.start)?;
            let end = *byte_map.get(span.start.checked_add(span.len)?)?;
            (end > start).then_some(StyleSpan {
                start,
                len: end - start,
                style: span.style,
            })
        })
        .collect();
    (MessageBody::with_styles(out, mapped_styles), mentions)
}

/// Result of `Effect::StageFile` (D77): attach to the open conversation or show
/// an error notice (e.g. > 100 MB).
pub fn on_file_staged(state: &mut State, result: Result<StagedAttachment, String>) -> Vec<Effect> {
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
    vec![]
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

/// `Ctrl+V` (D91, D150): ask the terminal for the clipboard's image, or read
/// the computer's own clipboard. The app loop pauses input while it waits
/// (D90), so this can't be pressed twice.
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
    state.notice = Some(Notice::warning("Reading the clipboard…"));
    vec![Effect::ReadClipboardImage {
        conversation,
        system: state.config.media.paste_from_system,
    }]
}

/// The terminal is about to ask the user for the clipboard (D150).
pub fn on_clipboard_prompt(state: &mut State, terminal: TerminalName) -> Vec<Effect> {
    if state.pasting.is_some() {
        state.notice = Some(Notice::warning(format!(
            "Waiting for {}… allow the clipboard prompt",
            terminal.label()
        )));
    }
    vec![]
}

/// The clipboard read is over: stage the image (never send it, D91), or say
/// why not.
pub fn on_clipboard_image(
    state: &mut State,
    conversation: ConversationId,
    result: Result<StagedAttachment, ClipboardError>,
) -> Vec<Effect> {
    state.pasting = None;
    let staged = match result {
        Ok(staged) => staged,
        Err(error) => {
            state.notice = Some(error.notice());
            return vec![];
        }
    };
    let Some(open) = state.open.as_mut().filter(|open| open.id == conversation) else {
        state.notice = Some(Notice::warning(
            "The pasted image was dropped: the conversation changed",
        ));
        return vec![];
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
    vec![]
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
pub(crate) fn clear_draft(state: &mut State, conversation: ConversationId) -> Vec<Effect> {
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
pub(super) fn next_sent_at(last: Timestamp, now: Timestamp) -> Timestamp {
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
