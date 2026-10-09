//! Message selection and what can be done with the selected message (M4 task 4,
//! D40; attachments, D75): reply, react, copy, open links, download/view/open/
//! save attachments, retry a failed send.

use std::path::PathBuf;

use faro_core::backend::{BackendError, BackendEvent, Capability, OutgoingMessage, ReactionChange};
use faro_core::config::{AutoDownload, TranscribeMode};
use faro_core::domain::{
    AttachmentRef, AttachmentStatus, ConversationId, DeliveryStatus, Direction, Message,
    MessageBody, MessageRef, Quote, ServiceId, Timestamp, TranscriptSource,
};
use faro_core::store::{RecipientReceipt, RemoteTranscription};
use tokio::time::Instant;

use super::{curation, on_backend};
use crate::app::commands::Command;
use crate::app::event::Effect;
use crate::app::input::Input;
use crate::app::keymap::PopupKey;
use crate::app::state::{
    AUTO_DOWNLOAD_LIMIT, DownloadJob, Focus, MAX_DOWNLOADS_IN_FLIGHT, MessageAction, Notice,
    PlayingVoiceNote, Popup, REACTIONS, REMOTE_CONSENT_CHOICES, Selection, State,
};

/// At most this many links get their own menu entry.
const MAX_LINK_ITEMS: usize = 5;

/// Focus the message pane and select the newest message on screen.
pub fn enter(state: &mut State) {
    state.focus = Focus::Messages;
    select_initial(state);
}

/// Select the message on the bottom row: the newest one, or where the view is
/// scrolled to.
pub fn select_initial(state: &mut State) {
    let Some(open) = state.open.as_mut() else {
        return;
    };
    let message = open
        .scroll
        .map(|anchor| anchor.message)
        .or_else(|| open.messages.last().map(Message::reference));
    open.selected = message.map(|message| Selection {
        message,
        reveal: true,
    });
}

/// Select `message` (a mouse click).
pub fn select(state: &mut State, message: MessageRef) {
    state.focus = Focus::Messages;
    if let Some(open) = state.open.as_mut() {
        open.selected = Some(Selection {
            message,
            reveal: true,
        });
    }
}

/// One message older (`older`) or newer. Past the newest, back to the input.
pub fn step(state: &mut State, older: bool) {
    let Some(open) = state.open.as_mut() else {
        return;
    };
    let current = open.selected.and_then(|s| {
        open.messages
            .iter()
            .rposition(|m| m.reference() == s.message)
    });
    let Some(at) = current else {
        select_initial(state);
        return;
    };
    let next = if older {
        at.saturating_sub(1)
    } else if at + 1 >= open.messages.len() {
        state.focus = Focus::Input;
        return;
    } else {
        at + 1
    };
    open.selected = open.messages.get(next).map(|m| Selection {
        message: m.reference(),
        reveal: true,
    });
}

/// The oldest loaded (`oldest`) or the newest message.
pub fn jump(state: &mut State, oldest: bool) {
    let Some(open) = state.open.as_mut() else {
        return;
    };
    let message = if oldest {
        open.messages.first()
    } else {
        open.messages.last()
    };
    open.selected = message.map(|m| Selection {
        message: m.reference(),
        reveal: true,
    });
}

/// Jump to the open conversation's unread marker (`u`), loading the surrounding
/// history if the marker is older than the currently loaded page (M12).
pub fn jump_unread(state: &mut State) -> Vec<Effect> {
    let Some(open) = state.open.as_mut() else {
        return vec![];
    };
    let Some(target) = open.unread_marker else {
        state.notice = Some(Notice::info("No unread marker in this conversation"));
        return vec![];
    };
    if open.messages.iter().any(|m| m.reference() == target) {
        open.selected = Some(Selection {
            message: target,
            reveal: true,
        });
        state.focus = Focus::Messages;
        vec![]
    } else {
        vec![Effect::LoadMessagesAround {
            conversation: open.id,
            target,
        }]
    }
}

/// The selected message, if the message pane has one.
fn selected(state: &State) -> Option<&Message> {
    let open = state.open.as_ref()?;
    let target = open.selected?.message;
    open.messages.iter().rfind(|m| m.reference() == target)
}

fn find(state: &State, target: MessageRef) -> Option<&Message> {
    let open = state.open.as_ref()?;
    open.messages.iter().rfind(|m| m.reference() == target)
}

fn attachment_label(attachment: &AttachmentRef) -> String {
    if attachment.is_voice_note() {
        return "voice note".to_owned();
    }
    attachment
        .file_name
        .as_deref()
        .map_or_else(|| attachment.content_type.clone(), str::to_owned)
}

/// What `message` offers, in menu order.
fn actions(state: &State, message: &Message) -> Vec<MessageAction> {
    let mut items = Vec::new();
    if message.direction == Direction::Outgoing(DeliveryStatus::Failed) {
        items.push(MessageAction::Retry);
    }
    if !message.deleted {
        items.extend([MessageAction::Reply, MessageAction::React]);
    }
    if !message.deleted && state.readable_body(message).is_some() {
        items.push(MessageAction::Copy);
    }
    if !message.deleted
        && (state.readable_body(message).is_some() || !message.attachments.is_empty())
    {
        items.push(MessageAction::Forward);
    }
    if let Some(body) = message.body.as_ref().filter(|_| !message.deleted) {
        let links = find_links(body.expose());
        items.extend(
            links
                .into_iter()
                .take(MAX_LINK_ITEMS)
                .map(MessageAction::OpenLink),
        );
    }
    if !message.deleted {
        let msg_ref = message.reference();
        for (index, att) in message.attachments.iter().enumerate() {
            if state.is_downloading(msg_ref, index) {
                continue;
            }
            let name = attachment_label(att);
            match &att.status {
                AttachmentStatus::Remote | AttachmentStatus::Failed => {
                    if !att.id.0.is_empty() {
                        items.push(MessageAction::DownloadAttachment {
                            index,
                            name: name.clone(),
                        });
                        items.push(MessageAction::SaveAttachment { index, name });
                    }
                }
                AttachmentStatus::Downloaded(_) => {
                    if att.is_voice_note() {
                        if state.can_play_audio() {
                            if state.playing_voice_note(msg_ref, index).is_some() {
                                items.push(MessageAction::StopVoiceNote { index });
                            } else {
                                items.push(MessageAction::PlayVoiceNote { index });
                            }
                        }
                        if !state.is_transcribing(msg_ref, index) {
                            items.push(MessageAction::TranscribeVoiceNote { index });
                        }
                    } else {
                        if att.is_image() {
                            items.push(MessageAction::ViewAttachment {
                                index,
                                name: name.clone(),
                            });
                        }
                        if !state.env.over_ssh {
                            items.push(MessageAction::OpenAttachment {
                                index,
                                name: name.clone(),
                            });
                        }
                    }
                    items.push(MessageAction::SaveAttachment {
                        index,
                        name: name.clone(),
                    });
                    items.push(MessageAction::CopyAttachmentPath { index, name });
                }
            }
        }
        items.push(MessageAction::Info);
    }
    if matches!(message.direction, Direction::Outgoing(_)) && !message.deleted {
        if state.capabilities.supports(Capability::Edit) {
            items.push(MessageAction::Edit);
        }
        if state.capabilities.supports(Capability::DeleteForEveryone) {
            items.push(MessageAction::Delete);
        }
    }
    items
}

/// `Enter` on a message: the action menu.
pub fn menu(state: &mut State) {
    let Some(message) = selected(state) else {
        return;
    };
    let target = message.reference();
    let items = actions(state, message);
    if !items.is_empty() {
        state.popup = Some(Popup::Actions {
            target,
            items,
            selected: 0,
        });
    }
}

/// The keys of the action menu's shortcuts, as typed in the message pane.
fn shortcut(c: char) -> Option<fn(&MessageAction) -> bool> {
    match c {
        'r' => Some(|a| *a == MessageAction::Reply),
        '+' => Some(|a| *a == MessageAction::React),
        'y' => Some(|a| *a == MessageAction::Copy),
        'f' => Some(|a| *a == MessageAction::Forward),
        'i' => Some(|a| *a == MessageAction::Info),
        'e' => Some(|a| *a == MessageAction::Edit),
        'D' => Some(|a| *a == MessageAction::Delete),
        'o' => Some(|a| matches!(a, MessageAction::OpenLink(_))),
        'x' => Some(|a| {
            matches!(
                a,
                MessageAction::PlayVoiceNote { .. } | MessageAction::StopVoiceNote { .. }
            )
        }),
        't' => Some(|a| matches!(a, MessageAction::TranscribeVoiceNote { .. })),
        'd' => Some(|a| matches!(a, MessageAction::DownloadAttachment { .. })),
        'v' => Some(|a| {
            matches!(
                a,
                MessageAction::ViewAttachment { .. } | MessageAction::OpenAttachment { .. }
            )
        }),
        's' => Some(|a| matches!(a, MessageAction::SaveAttachment { .. })),
        _ => None,
    }
}

pub fn on_popup(state: &mut State, key: PopupKey) -> Vec<Effect> {
    let Some(popup) = state.popup.take() else {
        return vec![];
    };
    match popup {
        Popup::Actions {
            target,
            items,
            selected,
        } => {
            let chosen = match key {
                PopupKey::Previous | PopupKey::Next => {
                    let selected = cycle(selected, items.len(), key == PopupKey::Next);
                    state.popup = Some(Popup::Actions {
                        target,
                        items,
                        selected,
                    });
                    return vec![];
                }
                PopupKey::Choose => items.get(selected).cloned(),
                PopupKey::Char(c) => {
                    shortcut(c).and_then(|is| items.iter().find(|a| is(a)).cloned())
                }
                PopupKey::Backspace => {
                    state.popup = Some(Popup::Actions {
                        target,
                        items,
                        selected,
                    });
                    return vec![];
                }
                PopupKey::Close => None,
            };
            chosen.map_or_else(Vec::new, |action| perform(state, target, action))
        }
        Popup::React {
            target,
            selected,
            query,
        } => on_react_key(state, target, selected, query, key),
        Popup::MessageInfo { target, receipts } => match key {
            PopupKey::Close | PopupKey::Choose | PopupKey::Char('q' | 'i') => vec![],
            PopupKey::Previous | PopupKey::Next | PopupKey::Backspace | PopupKey::Char(_) => {
                state.popup = Some(Popup::MessageInfo { target, receipts });
                vec![]
            }
        },
        Popup::ImageViewer { target, index } => match key {
            PopupKey::Char('s') => {
                state.popup = Some(Popup::ImageViewer { target, index });
                save_one(state, target, index)
            }
            PopupKey::Char('y') => {
                state.popup = Some(Popup::ImageViewer { target, index });
                copy_one_path(state, target, index)
            }
            PopupKey::Char('o') if !state.env.over_ssh => {
                state.popup = Some(Popup::ImageViewer { target, index });
                open_one_with_system(state, target, index)
            }
            PopupKey::Close | PopupKey::Choose | PopupKey::Char('q' | 'v') => vec![],
            PopupKey::Previous | PopupKey::Next | PopupKey::Backspace | PopupKey::Char(_) => {
                state.popup = Some(Popup::ImageViewer { target, index });
                vec![]
            }
        },
        Popup::StagedImage => match key {
            PopupKey::Close | PopupKey::Choose | PopupKey::Char('q') => vec![],
            PopupKey::Previous | PopupKey::Next | PopupKey::Backspace | PopupKey::Char(_) => {
                state.popup = Some(Popup::StagedImage);
                vec![]
            }
        },
        Popup::ConfirmDelete { target } => match key {
            PopupKey::Choose | PopupKey::Char('y' | 'Y') => confirm_delete(state, target),
            PopupKey::Close | PopupKey::Char('n' | 'N' | 'q') => vec![],
            PopupKey::Previous | PopupKey::Next | PopupKey::Backspace | PopupKey::Char(_) => {
                state.popup = Some(Popup::ConfirmDelete { target });
                vec![]
            }
        },
        Popup::ConfirmRemoteTranscription {
            conversation,
            target,
            index,
            selected,
        } => on_remote_consent_key(state, conversation, target, index, selected, key),
        Popup::Conversation {
            target,
            items,
            selected,
        } => curation::on_menu_key(state, target, items, selected, key),
        Popup::Mute { target, selected } => curation::on_mute_key(state, target, selected, key),
    }
}

pub(super) fn cycle(at: usize, len: usize, forward: bool) -> usize {
    match len {
        0 => 0,
        _ if forward => (at + 1) % len,
        _ => (at + len - 1) % len,
    }
}

pub fn perform(state: &mut State, target: MessageRef, action: MessageAction) -> Vec<Effect> {
    match action {
        MessageAction::Reply => {
            reply(state, target);
            vec![]
        }
        MessageAction::React => {
            react_picker(state, target);
            vec![]
        }
        MessageAction::Copy => copy(state, target),
        MessageAction::Forward => forward_target(state, target),
        MessageAction::Info => message_info(state, target),
        MessageAction::Edit => edit_target(state, target),
        MessageAction::Delete => {
            delete_target(state, target);
            vec![]
        }
        MessageAction::OpenLink(url) => open_url(state, url),
        MessageAction::PlayVoiceNote { index } => play_one_voice_note(state, target, index),
        MessageAction::StopVoiceNote { .. } => stop_voice_note(state),
        MessageAction::TranscribeVoiceNote { index } => {
            transcribe_one_voice_note(state, target, index, true)
        }
        MessageAction::DownloadAttachment { index, .. } => download_one(state, target, index, true),
        MessageAction::ViewAttachment { index, .. } => {
            if find(state, target)
                .and_then(|m| m.attachments.get(index))
                .is_some_and(|a| a.is_downloaded() && a.is_image())
            {
                state.popup = Some(Popup::ImageViewer { target, index });
            }
            vec![]
        }
        MessageAction::OpenAttachment { index, .. } => open_one_with_system(state, target, index),
        MessageAction::SaveAttachment { index, .. } => save_one(state, target, index),
        MessageAction::CopyAttachmentPath { index, .. } => copy_one_path(state, target, index),
        MessageAction::Retry => retry(state, target),
    }
}

/// `f`: forward the selected message to another conversation.
pub fn forward_message(state: &mut State) -> Vec<Effect> {
    let Some(message) = selected(state).filter(|m| !m.deleted) else {
        return vec![];
    };
    let target = message.reference();
    forward_target(state, target)
}

fn forward_target(state: &mut State, target: MessageRef) -> Vec<Effect> {
    let Some(message) = find(state, target).filter(|m| !m.deleted) else {
        return vec![];
    };
    let has_text = state
        .readable_body(message)
        .is_some_and(|s| !s.trim().is_empty());
    if !has_text && !message.attachments.iter().any(AttachmentRef::is_downloaded) {
        state.notice = Some(Notice::info(
            "Press d to download the attachment before forwarding",
        ));
        return vec![];
    }
    super::palette::open(state, crate::app::palette::Level::ForwardTo(target));
    vec![]
}

fn message_info(state: &mut State, target: MessageRef) -> Vec<Effect> {
    let Some(open) = state.open.as_ref() else {
        return vec![];
    };
    let conversation = open.id;
    let Some(message) = find(state, target).filter(|m| !m.deleted) else {
        return vec![];
    };
    let is_outgoing = matches!(message.direction, Direction::Outgoing(_));
    state.popup = Some(Popup::MessageInfo {
        target,
        receipts: Vec::new(),
    });
    if is_outgoing {
        vec![Effect::LoadMessageReceipts {
            conversation,
            message: target,
        }]
    } else {
        vec![]
    }
}

pub fn on_receipts_loaded(
    state: &mut State,
    conversation: ConversationId,
    message: MessageRef,
    loaded: Vec<RecipientReceipt>,
) {
    if state.open.as_ref().is_some_and(|o| o.id == conversation)
        && let Some(Popup::MessageInfo { target, receipts }) = state.popup.as_mut()
        && *target == message
    {
        *receipts = loaded;
    }
}

pub fn on_delivery_update(
    state: &mut State,
    message: MessageRef,
    status: DeliveryStatus,
    by: Option<ServiceId>,
    updated_at: Option<Timestamp>,
) {
    if let Some(Popup::MessageInfo { target, receipts }) = state.popup.as_mut()
        && *target == message
        && let (Some(recipient), Some(at)) = (by, updated_at)
        && matches!(status, DeliveryStatus::Delivered | DeliveryStatus::Read)
    {
        if let Some(existing) = receipts.iter_mut().find(|r| r.recipient == recipient) {
            if !(existing.status == DeliveryStatus::Read && status == DeliveryStatus::Delivered) {
                existing.status = existing.status.update(status);
                existing.updated_at = at;
            }
        } else {
            receipts.push(RecipientReceipt {
                recipient,
                status,
                updated_at: at,
            });
        }
        receipts.sort_by_key(|r| (r.updated_at, r.recipient.to_string()));
    }
}

/// Run an action on the selected message (its key in the message pane).
pub fn on_selected(state: &mut State, action: MessageAction) -> Vec<Effect> {
    match selected(state).map(Message::reference) {
        Some(target) => perform(state, target, action),
        None => vec![],
    }
}

/// `e`: start editing the selected outgoing message.
pub fn edit_message(state: &mut State) -> Vec<Effect> {
    let Some(message) = selected(state).filter(|m| !m.deleted) else {
        return vec![];
    };
    if !matches!(message.direction, Direction::Outgoing(_)) {
        state.notice = Some(Notice::info("Only your own messages can be edited"));
        return vec![];
    }
    if !state.capabilities.supports(Capability::Edit) {
        state.notice = Some(Notice::info("Editing is not supported by this backend"));
        return vec![];
    }
    let target = message.reference();
    edit_target(state, target)
}

fn edit_target(state: &mut State, target: MessageRef) -> Vec<Effect> {
    if !state.capabilities.supports(Capability::Edit) {
        return vec![];
    }
    let Some(message) =
        find(state, target).filter(|m| matches!(m.direction, Direction::Outgoing(_)) && !m.deleted)
    else {
        return vec![];
    };
    let body = state.editable_body(message);
    let existing_mentions: Vec<(ServiceId, String)> = message
        .mentions
        .iter()
        .map(|m| (m.who, state.person_name(m.who)))
        .collect();
    let effects = super::save_draft(state);
    if let Some(open) = state.open.as_mut() {
        open.editing = Some(target.sent_at);
        open.reply_to = None;
        open.staged_attachment = None;
        open.input = Input::new(&body);
        open.mentions = existing_mentions;
        open.mention_selected = 0;
        open.mention_dismissed_at = None;
        open.shortcode_selected = 0;
        open.shortcode_dismissed_at = None;
    }
    state.focus = Focus::Input;
    effects
}

/// `D`: prompt to delete the selected outgoing message for everyone.
pub fn delete_message(state: &mut State) -> Vec<Effect> {
    let Some(message) = selected(state).filter(|m| !m.deleted) else {
        return vec![];
    };
    if !matches!(message.direction, Direction::Outgoing(_)) {
        state.notice = Some(Notice::info(
            "Only your own messages can be deleted for everyone",
        ));
        return vec![];
    }
    if !state.capabilities.supports(Capability::DeleteForEveryone) {
        state.notice = Some(Notice::info(
            "Deleting messages is not supported by this backend",
        ));
        return vec![];
    }
    let target = message.reference();
    delete_target(state, target);
    vec![]
}

fn delete_target(state: &mut State, target: MessageRef) {
    if !state.capabilities.supports(Capability::DeleteForEveryone) {
        return;
    }
    if find(state, target)
        .is_some_and(|m| matches!(m.direction, Direction::Outgoing(_)) && !m.deleted)
    {
        state.popup = Some(Popup::ConfirmDelete { target });
    }
}

fn confirm_delete(state: &mut State, target: MessageRef) -> Vec<Effect> {
    let Some(conversation) = state.open.as_ref().map(|o| o.id) else {
        return vec![];
    };
    if !find(state, target)
        .is_some_and(|m| matches!(m.direction, Direction::Outgoing(_)) && !m.deleted)
    {
        return vec![];
    }
    if let Some(open) = state.open.as_mut()
        && open.editing == Some(target.sent_at)
    {
        open.editing = None;
        open.input = Input::default();
        open.mentions.clear();
        open.mention_selected = 0;
        open.mention_dismissed_at = None;
        open.shortcode_selected = 0;
        open.shortcode_dismissed_at = None;
    }
    let mut effects = on_backend(
        state,
        BackendEvent::Deleted {
            conversation,
            target,
        },
    );
    effects.push(Effect::Backend(Command::DeleteForEveryone {
        conversation,
        target,
    }));
    effects
}

fn reply(state: &mut State, target: MessageRef) {
    let Some(message) = find(state, target).filter(|m| !m.deleted) else {
        return;
    };
    // Mentions as `@Name`: the quote goes out without them (D72).
    let quote = Quote {
        target,
        preview: state.readable_body(message).map(MessageBody::new),
    };
    if let Some(open) = state.open.as_mut() {
        open.reply_to = Some(quote);
    }
    state.focus = Focus::Input;
}

use crate::ui::shortcodes::REACT_SEARCH_LIMIT;

pub fn react_picker(state: &mut State, target: MessageRef) {
    let Some(message) = find(state, target).filter(|m| !m.deleted) else {
        return;
    };
    let mine = own_reaction(state, message);
    let selected = mine
        .and_then(|emoji| REACTIONS.iter().position(|r| *r == emoji))
        .unwrap_or(0);
    state.popup = Some(Popup::React {
        target,
        selected,
        query: None,
    });
}

fn on_react_key(
    state: &mut State,
    target: MessageRef,
    selected: usize,
    query: Option<String>,
    key: PopupKey,
) -> Vec<Effect> {
    if let Some(q) = query {
        return on_react_search_key(state, target, selected, q, key);
    }
    let chosen = match key {
        PopupKey::Previous | PopupKey::Next => {
            let selected = cycle(selected, REACTIONS.len(), key == PopupKey::Next);
            state.popup = Some(Popup::React {
                target,
                selected,
                query: None,
            });
            return vec![];
        }
        PopupKey::Choose => Some(selected),
        PopupKey::Char(c) if c == '/' || c == ':' => {
            state.popup = Some(Popup::React {
                target,
                selected: 0,
                query: Some(String::new()),
            });
            return vec![];
        }
        PopupKey::Char(c) if c.is_ascii_alphabetic() => {
            state.popup = Some(Popup::React {
                target,
                selected: 0,
                query: Some(c.to_string()),
            });
            return vec![];
        }
        PopupKey::Char(c) => c
            .to_digit(10)
            .and_then(|d| usize::try_from(d).ok())
            .and_then(|d| d.checked_sub(1))
            .filter(|i| *i < REACTIONS.len()),
        PopupKey::Backspace => {
            state.popup = Some(Popup::React {
                target,
                selected,
                query: None,
            });
            return vec![];
        }
        PopupKey::Close => None,
    };
    match chosen.and_then(|i| REACTIONS.get(i)) {
        Some(emoji) => react(state, target, emoji),
        None => vec![],
    }
}

fn on_react_search_key(
    state: &mut State,
    target: MessageRef,
    selected: usize,
    mut q: String,
    key: PopupKey,
) -> Vec<Effect> {
    match key {
        PopupKey::Previous | PopupKey::Next => {
            let len = crate::ui::shortcodes::search_shortcodes(&q, REACT_SEARCH_LIMIT).len();
            let selected = cycle(selected, len, key == PopupKey::Next);
            state.popup = Some(Popup::React {
                target,
                selected,
                query: Some(q),
            });
            vec![]
        }
        PopupKey::Choose => {
            let matches = crate::ui::shortcodes::search_shortcodes(&q, REACT_SEARCH_LIMIT);
            if let Some(&(_, emoji)) = matches.get(selected) {
                react(state, target, emoji)
            } else {
                state.popup = Some(Popup::React {
                    target,
                    selected: 0,
                    query: Some(q),
                });
                vec![]
            }
        }
        PopupKey::Backspace => {
            q.pop();
            let count = crate::ui::shortcodes::search_shortcodes(&q, REACT_SEARCH_LIMIT).len();
            let selected = selected.min(count.saturating_sub(1));
            state.popup = Some(Popup::React {
                target,
                selected,
                query: Some(q),
            });
            vec![]
        }
        PopupKey::Char(c) => {
            if c != ':' && !c.is_control() {
                q.push(c);
            }
            let count = crate::ui::shortcodes::search_shortcodes(&q, REACT_SEARCH_LIMIT).len();
            let selected = selected.min(count.saturating_sub(1));
            state.popup = Some(Popup::React {
                target,
                selected,
                query: Some(q),
            });
            vec![]
        }
        PopupKey::Close => vec![],
    }
}

fn own_reaction<'a>(state: &State, message: &'a Message) -> Option<&'a str> {
    message
        .reactions
        .iter()
        .find(|r| r.sender == state.me)
        .map(|r| r.emoji.as_str())
}

/// React with `emoji`, or take the reaction back if it's already ours (the
/// removal names the emoji, D73). Shown at once; undone if the backend fails.
fn react(state: &mut State, target: MessageRef, emoji: &str) -> Vec<Effect> {
    let Some(open) = state.open.as_ref() else {
        return vec![];
    };
    let conversation = open.id;
    let Some(message) = find(state, target) else {
        return vec![];
    };
    let previous = own_reaction(state, message).map(str::to_owned);
    let change = if previous.as_deref() == Some(emoji) {
        ReactionChange::Remove(emoji.to_owned())
    } else {
        ReactionChange::Set(emoji.to_owned())
    };
    let event = reaction(
        state,
        conversation,
        target,
        change.emoji().map(str::to_owned),
    );
    let mut effects = on_backend(state, event);
    effects.push(Effect::Backend(Command::React {
        conversation,
        target,
        change,
        previous,
    }));
    effects
}

fn reaction(
    state: &State,
    conversation: ConversationId,
    target: MessageRef,
    emoji: Option<String>,
) -> BackendEvent {
    BackendEvent::Reaction {
        conversation,
        target,
        sender: state.me,
        emoji,
    }
}

pub fn on_react_result(
    state: &mut State,
    conversation: ConversationId,
    target: MessageRef,
    previous: Option<String>,
    result: Result<(), BackendError>,
) -> Vec<Effect> {
    match result {
        Ok(()) => vec![],
        Err(error) => {
            state.notice = Some(Notice::error(format!("Reaction not sent: {error}")));
            let undo = reaction(state, conversation, target, previous);
            on_backend(state, undo)
        }
    }
}

fn copy(state: &mut State, target: MessageRef) -> Vec<Effect> {
    let text = find(state, target)
        .filter(|m| !m.deleted)
        .and_then(|m| state.readable_body(m));
    if let Some(text) = text {
        state.notice = Some(Notice::info("Copied to the clipboard"));
        vec![Effect::Copy(text)]
    } else {
        state.notice = Some(Notice::info("Nothing to copy"));
        vec![]
    }
}

/// `o`: open the message's link; with several, choose from a menu.
pub fn open_link(state: &mut State) -> Vec<Effect> {
    let Some(message) = selected(state) else {
        return vec![];
    };
    let target = message.reference();
    let links = message
        .body
        .as_ref()
        .filter(|_| !message.deleted)
        .map(|b| find_links(b.expose()))
        .unwrap_or_default();
    match links.as_slice() {
        [] => {
            state.notice = Some(Notice::info("No link in this message"));
            vec![]
        }
        [one] => open_url(state, one.clone()),
        [..] => {
            let items = links
                .into_iter()
                .take(MAX_LINK_ITEMS)
                .map(MessageAction::OpenLink)
                .collect();
            state.popup = Some(Popup::Actions {
                target,
                items,
                selected: 0,
            });
            vec![]
        }
    }
}

/// Over SSH a browser would open on the far side (if at all), so copy instead.
pub fn open_shared_url(state: &mut State, url: String) -> Vec<Effect> {
    open_url(state, url)
}

fn open_url(state: &mut State, url: String) -> Vec<Effect> {
    if state.env.over_ssh {
        state.notice = Some(Notice::info(
            "Link copied (over SSH, links are copied, not opened)",
        ));
        vec![Effect::Copy(url)]
    } else {
        state.notice = Some(Notice::info("Opening the link…"));
        vec![Effect::OpenUrl(url)]
    }
}

/// Activate a specific attachment on `target` (chosen from `Level::ChatLinks`):
/// download if remote or failed, play/stop if a downloaded voice note, view in
/// `Popup::ImageViewer` if a downloaded image, or open with the system viewer
/// (saving and copying the path over SSH).
pub fn activate_attachment(state: &mut State, target: MessageRef, index: usize) -> Vec<Effect> {
    let Some(att) = find(state, target)
        .filter(|m| !m.deleted)
        .and_then(|m| m.attachments.get(index))
    else {
        return vec![];
    };
    match &att.status {
        AttachmentStatus::Remote | AttachmentStatus::Failed => {
            download_one(state, target, index, true)
        }
        AttachmentStatus::Downloaded(_) => {
            if att.is_voice_note() {
                play_one_voice_note(state, target, index)
            } else if att.is_image() {
                state.popup = Some(Popup::ImageViewer { target, index });
                vec![]
            } else {
                open_one_with_system(state, target, index)
            }
        }
    }
}

/// Send a failed message again, with the same identity.
fn retry(state: &mut State, target: MessageRef) -> Vec<Effect> {
    let Some(open) = state.open.as_ref() else {
        return vec![];
    };
    let conversation = open.id;
    let Some(message) =
        find(state, target).filter(|m| m.direction == Direction::Outgoing(DeliveryStatus::Failed))
    else {
        return vec![];
    };
    if message.body.is_none() && message.attachments.is_empty() {
        return vec![];
    }
    let body = message.body.clone().unwrap_or_else(|| MessageBody::new(""));
    let attachments = message
        .attachments
        .iter()
        .filter_map(|att| {
            Some(faro_core::backend::OutgoingAttachment {
                path: att.local_path()?.to_path_buf(),
                content_type: att.content_type.clone(),
                file_name: att.file_name.clone(),
                size: att.size.unwrap_or(0),
                width: att.width,
                height: att.height,
            })
        })
        .collect();
    let outgoing = OutgoingMessage {
        sent_at: target.sent_at,
        body,
        mentions: message.mentions.clone(),
        quote: message.quote.clone(),
        attachments,
    };
    let mut effects = on_backend(
        state,
        BackendEvent::DeliveryUpdate {
            message: target,
            status: DeliveryStatus::Sending,
            by: None,
            updated_at: None,
        },
    );
    effects.push(Effect::Backend(Command::Send {
        conversation,
        message: outgoing,
    }));
    effects
}

/// `x`: play or stop the selected message's voice note (D81). Over SSH or mosh,
/// saves the voice note to `~/Downloads/` and copies its path instead.
pub fn play_voice_note(state: &mut State) -> Vec<Effect> {
    let Some(message) = selected(state).filter(|m| !m.deleted) else {
        return vec![];
    };
    let target = message.reference();
    let Some((index, att)) = message
        .attachments
        .iter()
        .enumerate()
        .find(|(_, att)| att.is_voice_note())
    else {
        if state.playing.is_some() {
            return stop_voice_note(state);
        }
        state.notice = Some(Notice::info("No voice note in this message"));
        return vec![];
    };
    if state.playing_voice_note(target, index).is_some() {
        return stop_voice_note(state);
    }
    if !state.can_play_audio() {
        if att.is_downloaded() {
            return save_and_copy_remote_voice_note(state, target, index);
        }
        state.notice = Some(Notice::info(if state.is_downloading(target, index) {
            "Voice note is still downloading…"
        } else {
            "Press d to download the voice note first (or s to save to Downloads)"
        }));
        return vec![];
    }
    if !att.is_downloaded() {
        state.notice = Some(Notice::info(if state.is_downloading(target, index) {
            "Voice note is still downloading…"
        } else {
            "Press d to download the voice note first"
        }));
        return vec![];
    }
    play_one_voice_note(state, target, index)
}

fn play_one_voice_note(state: &mut State, target: MessageRef, index: usize) -> Vec<Effect> {
    if state.playing_voice_note(target, index).is_some() {
        return stop_voice_note(state);
    }
    if !state.can_play_audio() {
        return save_and_copy_remote_voice_note(state, target, index);
    }
    let Some(conversation) = state.open.as_ref().map(|o| o.id) else {
        return vec![];
    };
    let Some(att) = find(state, target).and_then(|m| m.attachments.get(index)) else {
        return vec![];
    };
    let Some(path) = att.local_path().map(PathBuf::from) else {
        state.notice = Some(Notice::info("Press d to download the voice note first"));
        return vec![];
    };
    let duration_secs = att.duration_secs;
    let player = state.config.voice.player.clone();
    state.playing = Some(PlayingVoiceNote {
        conversation,
        message: target,
        index,
        started_at: Instant::now(),
        elapsed_secs: 0,
        duration_secs,
    });
    vec![Effect::PlayAudio {
        message: target,
        index,
        path,
        player,
    }]
}

fn stop_voice_note(state: &mut State) -> Vec<Effect> {
    if state.playing.take().is_some() {
        vec![Effect::StopAudio]
    } else {
        vec![]
    }
}

pub fn on_playback_finished(
    state: &mut State,
    message: MessageRef,
    index: usize,
    error: Option<String>,
) {
    if state
        .playing
        .as_ref()
        .is_some_and(|p| p.message == message && p.index == index)
    {
        state.playing = None;
    }
    if let Some(error) = error {
        state.notice = Some(Notice::warning(format!(
            "Couldn't play voice note: {error}"
        )));
    }
}

/// `t`: transcribe the selected message's voice note (D82).
pub fn transcribe_voice_note(state: &mut State) -> Vec<Effect> {
    let Some(message) = selected(state).filter(|m| !m.deleted) else {
        return vec![];
    };
    let target = message.reference();
    let Some((index, _)) = message
        .attachments
        .iter()
        .enumerate()
        .find(|(_, att)| att.is_voice_note())
    else {
        state.notice = Some(Notice::info("No voice note in this message"));
        return vec![];
    };
    transcribe_one_voice_note(state, target, index, true)
}

fn transcribe_one_voice_note(
    state: &mut State,
    target: MessageRef,
    index: usize,
    manual: bool,
) -> Vec<Effect> {
    let Some(conversation) = state.open.as_ref().map(|o| o.id) else {
        return vec![];
    };
    let Some(att) = find(state, target).and_then(|m| m.attachments.get(index)) else {
        return vec![];
    };
    if !att.is_voice_note() {
        return vec![];
    }
    let max_secs = state.config.voice.transcribe_max_secs;
    if !manual && max_secs > 0 && att.duration_secs.is_some_and(|d| d > max_secs) {
        return vec![];
    }
    if !att.is_downloaded() {
        if manual && !att.id.0.is_empty() {
            state.transcribe_when_downloaded.insert((target, index));
            let effects = download_one(state, target, index, false);
            state.notice = Some(Notice::info("Downloading voice note to transcribe…"));
            return effects;
        }
        return vec![];
    }
    let mode = match state.config.voice.transcribe {
        TranscribeMode::Remote => TranscribeMode::Remote,
        TranscribeMode::Local | TranscribeMode::Off => TranscribeMode::Local,
    };
    if mode == TranscribeMode::Remote {
        let perm = state
            .summary(conversation)
            .map_or(RemoteTranscription::Disallowed, |s| {
                s.state.remote_transcription
            });
        match perm {
            RemoteTranscription::Allowed => {
                return start_transcription(
                    state,
                    conversation,
                    target,
                    index,
                    TranscribeMode::Remote,
                    manual,
                );
            }
            RemoteTranscription::Disallowed => {
                if manual {
                    state.popup = Some(Popup::ConfirmRemoteTranscription {
                        conversation,
                        target,
                        index,
                        selected: 0,
                    });
                }
                return vec![];
            }
        }
    }
    start_transcription(
        state,
        conversation,
        target,
        index,
        TranscribeMode::Local,
        manual,
    )
}

fn on_remote_consent_key(
    state: &mut State,
    conversation: ConversationId,
    target: MessageRef,
    index: usize,
    selected: usize,
    key: PopupKey,
) -> Vec<Effect> {
    let chosen = match key {
        PopupKey::Previous | PopupKey::Next => {
            let selected = cycle(
                selected,
                REMOTE_CONSENT_CHOICES.len(),
                key == PopupKey::Next,
            );
            state.popup = Some(Popup::ConfirmRemoteTranscription {
                conversation,
                target,
                index,
                selected,
            });
            return vec![];
        }
        PopupKey::Choose => Some(selected),
        PopupKey::Char(c) => c
            .to_digit(10)
            .and_then(|d| usize::try_from(d).ok())
            .and_then(|d| d.checked_sub(1))
            .filter(|i| *i < REMOTE_CONSENT_CHOICES.len()),
        PopupKey::Backspace => {
            state.popup = Some(Popup::ConfirmRemoteTranscription {
                conversation,
                target,
                index,
                selected,
            });
            return vec![];
        }
        PopupKey::Close => None,
    };
    let Some(choice) = chosen else {
        return vec![];
    };
    match choice {
        0 => {
            let mut effects = Vec::new();
            if let Some(summary) = state
                .conversations
                .iter_mut()
                .find(|c| c.id == conversation)
            {
                summary.state.remote_transcription = RemoteTranscription::Allowed;
                effects.push(Effect::SaveConversationState(
                    conversation,
                    Box::new(summary.state.clone()),
                ));
            }
            effects.extend(start_transcription(
                state,
                conversation,
                target,
                index,
                TranscribeMode::Remote,
                true,
            ));
            effects.extend(queue_auto_transcriptions(state));
            effects
        }
        1 => start_transcription(
            state,
            conversation,
            target,
            index,
            TranscribeMode::Remote,
            true,
        ),
        2 => {
            let mut effects = Vec::new();
            if let Some(summary) = state
                .conversations
                .iter_mut()
                .find(|c| c.id == conversation)
            {
                summary.state.remote_transcription = RemoteTranscription::Disallowed;
                effects.push(Effect::SaveConversationState(
                    conversation,
                    Box::new(summary.state.clone()),
                ));
            }
            state.notice = Some(Notice::info(
                "Remote transcription disabled for this conversation",
            ));
            effects
        }
        _ => vec![],
    }
}

fn start_transcription(
    state: &mut State,
    conversation: ConversationId,
    target: MessageRef,
    index: usize,
    mode: TranscribeMode,
    announce: bool,
) -> Vec<Effect> {
    let Some(att) = find(state, target).and_then(|m| m.attachments.get(index)) else {
        return vec![];
    };
    let Some(path) = att.local_path().map(PathBuf::from) else {
        return vec![];
    };
    if !state.transcribing.insert((target, index)) {
        if announce {
            state.notice = Some(Notice::info("Voice note is already transcribing…"));
        }
        return vec![];
    }
    let mut voice = state.config.voice.clone();
    voice.transcribe = mode;
    if announce {
        state.notice = Some(Notice::info("Transcribing voice note…"));
    }
    vec![Effect::Backend(Command::Transcribe {
        conversation,
        message: target,
        index,
        path,
        voice,
    })]
}

/// Queue auto-transcriptions for downloaded, untranscribed voice notes in the
/// open conversation when `state.config.voice.transcribe` is `local` or `remote` (D82).
pub fn queue_auto_transcriptions(state: &mut State) -> Vec<Effect> {
    if state.config.voice.transcribe == TranscribeMode::Off {
        return vec![];
    }
    let max_secs = state.config.voice.transcribe_max_secs;
    let candidates: Vec<(MessageRef, usize)> = state
        .open
        .as_ref()
        .filter(|o| !o.loading)
        .map(|open| {
            let mut out = Vec::new();
            for message in &open.messages {
                if message.deleted {
                    continue;
                }
                let msg_ref = message.reference();
                for (index, att) in message.attachments.iter().enumerate() {
                    let within_limit =
                        max_secs == 0 || att.duration_secs.is_none_or(|d| d <= max_secs);
                    if att.is_voice_note()
                        && att.is_downloaded()
                        && att.transcript.is_none()
                        && within_limit
                        && !state.is_transcribing(msg_ref, index)
                    {
                        out.push((msg_ref, index));
                    }
                }
            }
            out
        })
        .unwrap_or_default();
    let mut effects = Vec::new();
    for (target, index) in candidates {
        effects.extend(transcribe_one_voice_note(state, target, index, false));
    }
    effects
}

pub fn on_transcribed(
    state: &mut State,
    conversation: ConversationId,
    message: MessageRef,
    index: usize,
    result: Result<(String, TranscriptSource), String>,
) -> Vec<Effect> {
    state.transcribing.remove(&(message, index));
    match result {
        Ok((text, source)) => {
            let mut updated = None;
            if let Some(open) = state.open.as_mut().filter(|o| o.id == conversation)
                && let Some(msg) = open.messages.iter_mut().rfind(|m| m.reference() == message)
                && let Some(att) = msg.attachments.get_mut(index)
            {
                att.transcript = Some(text);
                att.transcript_source = Some(source);
                updated = Some(att.clone());
            }
            if let Some(attachment) = updated {
                vec![Effect::SaveAttachment {
                    message,
                    index,
                    attachment,
                }]
            } else {
                vec![]
            }
        }
        Err(error) => {
            state.notice = Some(Notice::warning(format!(
                "Voice note transcription failed: {error}"
            )));
            vec![]
        }
    }
}

fn save_and_copy_remote_voice_note(
    state: &mut State,
    target: MessageRef,
    index: usize,
) -> Vec<Effect> {
    let Some(att) = find(state, target).and_then(|m| m.attachments.get(index)) else {
        return vec![];
    };
    let Some(path) = att.local_path().map(PathBuf::from) else {
        return vec![];
    };
    let file_name = att.file_name.clone();
    let content_type = att.content_type.clone();
    let display = path.display().to_string();
    state.notice = Some(Notice::info(
        "Saving voice note to Downloads and copying path (remote session: audio isn't played remotely)",
    ));
    vec![
        Effect::Copy(display),
        Effect::SaveToDownloads {
            source: path,
            file_name,
            content_type,
        },
    ]
}

/// `d`: download the selected message's attachment(s).
pub fn download_attachment(state: &mut State) -> Vec<Effect> {
    let Some(message) = selected(state).filter(|m| !m.deleted) else {
        return vec![];
    };
    if message.attachments.is_empty() {
        state.notice = Some(Notice::info("No attachment in this message"));
        return vec![];
    }
    let target = message.reference();
    let count = message.attachments.len();
    let downloadable: Vec<(usize, String)> = message
        .attachments
        .iter()
        .enumerate()
        .filter(|(index, att)| {
            matches!(
                att.status,
                AttachmentStatus::Remote | AttachmentStatus::Failed
            ) && !state.is_downloading(target, *index)
                && !att.id.0.is_empty()
        })
        .map(|(index, att)| (index, attachment_label(att)))
        .collect();
    match downloadable.as_slice() {
        [] => {
            let downloading = (0..count).any(|i| state.is_downloading(target, i));
            state.notice = Some(Notice::info(if downloading {
                "Attachment is already downloading…"
            } else {
                "Attachment is already downloaded"
            }));
            vec![]
        }
        [(index, _)] => download_one(state, target, *index, true),
        [..] => {
            let items = downloadable
                .into_iter()
                .map(|(index, name)| MessageAction::DownloadAttachment { index, name })
                .collect();
            state.popup = Some(Popup::Actions {
                target,
                items,
                selected: 0,
            });
            vec![]
        }
    }
}

/// `v`: view an image in the inline viewer popup, play/stop a voice note, open
/// a local file with the system viewer, or (for non-image files over SSH) save
/// to `~/Downloads/` and copy the path (D75, D81).
pub fn open_attachment(state: &mut State) -> Vec<Effect> {
    let Some(message) = selected(state).filter(|m| !m.deleted) else {
        return vec![];
    };
    if message.attachments.is_empty() {
        state.notice = Some(Notice::info("No attachment in this message"));
        return vec![];
    }
    let target = message.reference();
    let count = message.attachments.len();
    let over_ssh = state.env.over_ssh;
    let can_play = state.can_play_audio();
    let downloaded: Vec<(usize, bool, bool, String)> = message
        .attachments
        .iter()
        .enumerate()
        .filter(|(_, att)| att.is_downloaded())
        .map(|(index, att)| {
            (
                index,
                att.is_image(),
                att.is_voice_note(),
                attachment_label(att),
            )
        })
        .collect();
    match downloaded.as_slice() {
        [] => {
            let downloading = (0..count).any(|i| state.is_downloading(target, i));
            state.notice = Some(Notice::info(if downloading {
                "Attachment is still downloading…"
            } else {
                "Press d to download the attachment first"
            }));
            vec![]
        }
        [(index, is_image, is_voice, name)] => {
            if *is_voice {
                play_one_voice_note(state, target, *index)
            } else if *is_image {
                perform(
                    state,
                    target,
                    MessageAction::ViewAttachment {
                        index: *index,
                        name: name.clone(),
                    },
                )
            } else if !over_ssh {
                open_one_with_system(state, target, *index)
            } else {
                save_and_copy_over_ssh(state, target, *index)
            }
        }
        [..] => {
            let items = downloaded
                .into_iter()
                .map(|(index, is_image, is_voice, name)| {
                    if is_voice && can_play {
                        if state.playing_voice_note(target, index).is_some() {
                            MessageAction::StopVoiceNote { index }
                        } else {
                            MessageAction::PlayVoiceNote { index }
                        }
                    } else if is_image {
                        MessageAction::ViewAttachment { index, name }
                    } else if !over_ssh && !is_voice {
                        MessageAction::OpenAttachment { index, name }
                    } else {
                        MessageAction::SaveAttachment { index, name }
                    }
                })
                .collect();
            state.popup = Some(Popup::Actions {
                target,
                items,
                selected: 0,
            });
            vec![]
        }
    }
}

/// `s`: save the selected message's attachment to `~/Downloads/` (downloading
/// it first if needed, D75).
pub fn save_attachment(state: &mut State) -> Vec<Effect> {
    let Some(message) = selected(state).filter(|m| !m.deleted) else {
        return vec![];
    };
    let target = message.reference();
    let savable: Vec<MessageAction> = message
        .attachments
        .iter()
        .enumerate()
        .filter(|(_, att)| att.is_downloaded() || !att.id.0.is_empty())
        .map(|(index, att)| MessageAction::SaveAttachment {
            index,
            name: attachment_label(att),
        })
        .collect();
    match savable.as_slice() {
        [] => {
            state.notice = Some(Notice::info("No attachment in this message"));
            vec![]
        }
        [one] => perform(state, target, one.clone()),
        [..] => {
            state.popup = Some(Popup::Actions {
                target,
                items: savable,
                selected: 0,
            });
            vec![]
        }
    }
}

/// Copy a downloaded attachment's local path from the selected message.
pub fn copy_attachment_path(state: &mut State) -> Vec<Effect> {
    let Some(message) = selected(state).filter(|m| !m.deleted) else {
        return vec![];
    };
    let target = message.reference();
    let items: Vec<MessageAction> = message
        .attachments
        .iter()
        .enumerate()
        .filter(|(_, att)| att.is_downloaded())
        .map(|(index, att)| MessageAction::CopyAttachmentPath {
            index,
            name: attachment_label(att),
        })
        .collect();
    match items.as_slice() {
        [] => {
            state.notice = Some(Notice::info("Press d to download the attachment first"));
            vec![]
        }
        [one] => perform(state, target, one.clone()),
        [..] => {
            state.popup = Some(Popup::Actions {
                target,
                items,
                selected: 0,
            });
            vec![]
        }
    }
}

fn open_one_with_system(state: &mut State, target: MessageRef, index: usize) -> Vec<Effect> {
    if state.env.over_ssh {
        return save_and_copy_over_ssh(state, target, index);
    }
    let Some(att) = find(state, target).and_then(|m| m.attachments.get(index)) else {
        return vec![];
    };
    let Some(path) = att.local_path().map(PathBuf::from) else {
        state.notice = Some(Notice::info("Press d to download the attachment first"));
        return vec![];
    };
    let label = attachment_label(att);
    state.notice = Some(Notice::info(format!("Opening {label}…")));
    vec![Effect::OpenFile(path)]
}

fn save_and_copy_over_ssh(state: &mut State, target: MessageRef, index: usize) -> Vec<Effect> {
    let Some(att) = find(state, target).and_then(|m| m.attachments.get(index)) else {
        return vec![];
    };
    let Some(path) = att.local_path().map(PathBuf::from) else {
        return vec![];
    };
    let file_name = att.file_name.clone();
    let content_type = att.content_type.clone();
    let display = path.display().to_string();
    state.notice = Some(Notice::info(
        "Saving to Downloads and copying path (over SSH, files aren't opened remotely)",
    ));
    vec![
        Effect::Copy(display),
        Effect::SaveToDownloads {
            source: path,
            file_name,
            content_type,
        },
    ]
}

fn save_one(state: &mut State, target: MessageRef, index: usize) -> Vec<Effect> {
    let Some(att) = find(state, target).and_then(|m| m.attachments.get(index)) else {
        return vec![];
    };
    if let Some(source) = att.local_path().map(PathBuf::from) {
        let file_name = att.file_name.clone();
        let content_type = att.content_type.clone();
        return vec![Effect::SaveToDownloads {
            source,
            file_name,
            content_type,
        }];
    }
    if att.id.0.is_empty() {
        return vec![];
    }
    let label = attachment_label(att);
    state.save_when_downloaded.insert((target, index));
    let mut effects = download_one(state, target, index, false);
    state.notice = Some(Notice::info(format!(
        "Downloading {label} to save to Downloads…"
    )));
    effects.extend(start_queued_downloads(state));
    effects
}

fn copy_one_path(state: &mut State, target: MessageRef, index: usize) -> Vec<Effect> {
    let Some(path) = find(state, target)
        .and_then(|m| m.attachments.get(index))
        .and_then(AttachmentRef::local_path)
        .map(|p| p.display().to_string())
    else {
        state.notice = Some(Notice::info("Press d to download the attachment first"));
        return vec![];
    };
    state.notice = Some(Notice::info(format!("Copied path: {path}")));
    vec![Effect::Copy(path)]
}

fn download_one(
    state: &mut State,
    target: MessageRef,
    index: usize,
    announce: bool,
) -> Vec<Effect> {
    let Some(open) = state.open.as_ref() else {
        return vec![];
    };
    let conversation = open.id;
    let Some(message) = open.messages.iter().rfind(|m| m.reference() == target) else {
        return vec![];
    };
    let Some(att) = message.attachments.get(index) else {
        return vec![];
    };
    if att.id.0.is_empty() {
        return vec![];
    }
    if att.is_downloaded() {
        if announce {
            state.notice = Some(Notice::info("Attachment is already downloaded"));
        }
        return vec![];
    }
    let label = attachment_label(att);
    let job = DownloadJob {
        conversation,
        message: target,
        index,
        attachment: att.clone(),
    };
    if !state.is_downloading(target, index) {
        state.download_queue.push_front(job);
    }
    if announce {
        state.notice = Some(Notice::info(format!("Downloading {label}…")));
    }
    start_queued_downloads(state)
}

fn should_auto_download(policy: AutoDownload, attachment: &AttachmentRef) -> bool {
    if attachment.id.0.is_empty() {
        return false;
    }
    if attachment
        .size
        .is_some_and(|size| size > AUTO_DOWNLOAD_LIMIT)
    {
        return false;
    }
    match policy {
        AutoDownload::Off => false,
        AutoDownload::Images => {
            attachment.is_image() || attachment.is_sticker() || attachment.is_voice_note()
        }
        AutoDownload::All => true,
    }
}

/// Queue auto-downloads for remote attachments in `open.messages` according to
/// `state.config.media.auto_download` (D75), and start up to
/// `MAX_DOWNLOADS_IN_FLIGHT` concurrent downloads.
pub fn queue_auto_downloads(state: &mut State) -> Vec<Effect> {
    let policy = state.config.media.auto_download;
    let jobs: Vec<DownloadJob> = state
        .open
        .as_ref()
        .filter(|o| !o.loading)
        .map(|open| {
            let conversation = open.id;
            let mut jobs = Vec::new();
            for message in &open.messages {
                if message.deleted {
                    continue;
                }
                let msg_ref = message.reference();
                for (index, att) in message.attachments.iter().enumerate() {
                    if att.status == AttachmentStatus::Remote
                        && !state.is_downloading(msg_ref, index)
                        && should_auto_download(policy, att)
                    {
                        jobs.push(DownloadJob {
                            conversation,
                            message: msg_ref,
                            index,
                            attachment: att.clone(),
                        });
                    }
                }
            }
            jobs
        })
        .unwrap_or_default();
    state.download_queue.extend(jobs);
    start_queued_downloads(state)
}

fn start_queued_downloads(state: &mut State) -> Vec<Effect> {
    let mut effects = Vec::new();
    while state.downloading.len() < MAX_DOWNLOADS_IN_FLIGHT {
        let Some(job) = state.download_queue.pop_front() else {
            break;
        };
        let key = (job.message, job.index);
        if !state.downloading.insert(key) {
            continue;
        }
        effects.push(Effect::Backend(Command::Fetch {
            conversation: job.conversation,
            message: job.message,
            index: job.index,
            attachment: job.attachment,
        }));
    }
    effects
}

pub fn on_fetched(
    state: &mut State,
    conversation: ConversationId,
    message: MessageRef,
    index: usize,
    result: Result<AttachmentRef, BackendError>,
) -> Vec<Effect> {
    let key = (message, index);
    state.downloading.remove(&key);
    let save_after = state.save_when_downloaded.remove(&key);
    let transcribe_after = state.transcribe_when_downloaded.remove(&key);
    let mut effects = Vec::new();
    match result {
        Ok(fetched) => {
            let is_voice_note = fetched.is_voice_note();
            effects.push(Effect::SaveAttachment {
                message,
                index,
                attachment: fetched.clone(),
            });
            if save_after && let Some(source) = fetched.local_path().map(PathBuf::from) {
                effects.push(Effect::SaveToDownloads {
                    source,
                    file_name: fetched.file_name.clone(),
                    content_type: fetched.content_type.clone(),
                });
            }
            if let Some(open) = state.open.as_mut().filter(|o| o.id == conversation)
                && let Some(msg) = open.messages.iter_mut().rfind(|m| m.reference() == message)
                && let Some(att) = msg.attachments.get_mut(index)
            {
                *att = fetched;
            }
            if transcribe_after {
                effects.extend(transcribe_one_voice_note(state, message, index, true));
            } else if is_voice_note {
                effects.extend(queue_auto_transcriptions(state));
            }
        }
        Err(error) => {
            if let Some(open) = state.open.as_mut().filter(|o| o.id == conversation)
                && let Some(msg) = open.messages.iter_mut().rfind(|m| m.reference() == message)
                && let Some(att) = msg.attachments.get_mut(index)
            {
                att.status = AttachmentStatus::Failed;
            }
            state.notice = Some(Notice::warning(format!(
                "Attachment download failed: {error}"
            )));
        }
    }
    effects.extend(start_queued_downloads(state));
    effects
}

pub fn on_saved_to_downloads(state: &mut State, result: Result<PathBuf, String>) {
    state.notice = Some(match result {
        Ok(path) => Notice::info(format!("Saved to {}", path.display())),
        Err(error) => Notice::error(format!("Couldn't save attachment: {error}")),
    });
}

/// http(s) links in `text`, in order, without duplicates. Surrounding
/// punctuation (brackets, quotes, a full stop) isn't part of the link.
pub fn find_links(text: &str) -> Vec<String> {
    let mut links: Vec<String> = Vec::new();
    for word in text.split_whitespace() {
        let word = word.trim_start_matches(['(', '<', '[', '"', '\'']);
        let lower = word
            .get(..8)
            .map(str::to_ascii_lowercase)
            .unwrap_or_default();
        if !(lower.starts_with("http://") || lower.starts_with("https://")) {
            continue;
        }
        let mut link = word.trim_end_matches(['.', ',', ';', ':', '!', '?', '"', '\'', '>', ']']);
        while link.matches(')').count() > link.matches('(').count()
            && let Some(shorter) = link.strip_suffix(')')
        {
            link = shorter.trim_end_matches(['.', ',', ';', ':', '!', '?', '"', '\'']);
        }
        let has_host = link
            .split_once("://")
            .is_some_and(|(_, rest)| !rest.is_empty());
        if has_host && !link.chars().any(char::is_control) && !links.iter().any(|l| l == link) {
            links.push(link.to_owned());
        }
    }
    links
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_are_found_without_surrounding_punctuation() {
        assert_eq!(
            find_links("see https://example.com/a?b=1. and (http://x.org/wiki/Foo_(bar)), ok"),
            vec!["https://example.com/a?b=1", "http://x.org/wiki/Foo_(bar)"]
        );
        assert_eq!(
            find_links("<HTTPS://Example.com> https://example.com/x https://example.com/x"),
            vec!["HTTPS://Example.com", "https://example.com/x"]
        );
        assert_eq!(
            find_links("no links: http:// ftp://x.org www.x.org"),
            Vec::<String>::new()
        );
    }

    #[test]
    fn cycling_wraps() {
        assert_eq!(cycle(0, 3, false), 2);
        assert_eq!(cycle(2, 3, true), 0);
        assert_eq!(cycle(0, 0, true), 0);
    }
}
