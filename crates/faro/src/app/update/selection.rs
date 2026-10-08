//! Message selection and what can be done with the selected message (M4 task 4,
//! D40; attachments, D75): reply, react, copy, open links, download/view/open/
//! save attachments, retry a failed send.

use std::path::PathBuf;

use faro_core::backend::{BackendError, BackendEvent, OutgoingMessage, ReactionChange};
use faro_core::config::AutoDownload;
use faro_core::domain::{
    AttachmentRef, AttachmentStatus, ConversationId, DeliveryStatus, Direction, Message,
    MessageBody, MessageRef, Quote,
};

use super::{curation, on_backend};
use crate::app::commands::Command;
use crate::app::event::Effect;
use crate::app::keymap::PopupKey;
use crate::app::state::{
    AUTO_DOWNLOAD_LIMIT, DownloadJob, Focus, MAX_DOWNLOADS_IN_FLIGHT, MessageAction, Notice, Popup,
    REACTIONS, Selection, State,
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
    if let Some(body) = message.body.as_ref().filter(|_| !message.deleted) {
        items.push(MessageAction::Copy);
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
                    items.push(MessageAction::SaveAttachment {
                        index,
                        name: name.clone(),
                    });
                    items.push(MessageAction::CopyAttachmentPath { index, name });
                }
            }
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
        'o' => Some(|a| matches!(a, MessageAction::OpenLink(_))),
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
                PopupKey::Close => None,
            };
            chosen.map_or_else(Vec::new, |action| perform(state, target, action))
        }
        Popup::React { target, selected } => {
            let chosen = match key {
                PopupKey::Previous | PopupKey::Next => {
                    let selected = cycle(selected, REACTIONS.len(), key == PopupKey::Next);
                    state.popup = Some(Popup::React { target, selected });
                    return vec![];
                }
                PopupKey::Choose => Some(selected),
                PopupKey::Char(c) => c
                    .to_digit(10)
                    .and_then(|d| usize::try_from(d).ok())
                    .and_then(|d| d.checked_sub(1))
                    .filter(|i| *i < REACTIONS.len()),
                PopupKey::Close => None,
            };
            match chosen.and_then(|i| REACTIONS.get(i)) {
                Some(emoji) => react(state, target, emoji),
                None => vec![],
            }
        }
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
            PopupKey::Previous | PopupKey::Next | PopupKey::Char(_) => {
                state.popup = Some(Popup::ImageViewer { target, index });
                vec![]
            }
        },
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
        MessageAction::OpenLink(url) => open_url(state, url),
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

/// Run an action on the selected message (its key in the message pane).
pub fn on_selected(state: &mut State, action: MessageAction) -> Vec<Effect> {
    match selected(state).map(Message::reference) {
        Some(target) => perform(state, target, action),
        None => vec![],
    }
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

pub fn react_picker(state: &mut State, target: MessageRef) {
    let Some(message) = find(state, target).filter(|m| !m.deleted) else {
        return;
    };
    let mine = own_reaction(state, message);
    let selected = mine
        .and_then(|emoji| REACTIONS.iter().position(|r| *r == emoji))
        .unwrap_or(0);
    state.popup = Some(Popup::React { target, selected });
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
        quote: message.quote.clone(),
        attachments,
    };
    let mut effects = on_backend(
        state,
        BackendEvent::DeliveryUpdate {
            message: target,
            status: DeliveryStatus::Sending,
        },
    );
    effects.push(Effect::Backend(Command::Send {
        conversation,
        message: outgoing,
    }));
    effects
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

/// `v`: view an image in the inline viewer popup, open a local file with the
/// system viewer, or (for non-image files over SSH) save to `~/Downloads/` and
/// copy the path (D75).
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
    let downloaded: Vec<(usize, bool, String)> = message
        .attachments
        .iter()
        .enumerate()
        .filter(|(_, att)| att.is_downloaded())
        .map(|(index, att)| (index, att.is_image(), attachment_label(att)))
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
        [(index, is_image, name)] => {
            if *is_image {
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
                .map(|(index, is_image, name)| {
                    if is_image {
                        MessageAction::ViewAttachment { index, name }
                    } else if !over_ssh {
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
    let mut effects = Vec::new();
    match result {
        Ok(fetched) => {
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
