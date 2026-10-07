//! Message selection and what can be done with the selected message (M4 task 4,
//! D40): reply, react, copy, open links, retry a failed send.

use faro_core::backend::{BackendError, BackendEvent, OutgoingMessage};
use faro_core::domain::{ConversationId, DeliveryStatus, Direction, Message, MessageRef, Quote};

use super::{curation, on_backend};
use crate::app::commands::Command;
use crate::app::event::Effect;
use crate::app::keymap::PopupKey;
use crate::app::state::{Focus, MessageAction, Notice, Popup, REACTIONS, Selection, State};

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

/// What `message` offers, in menu order.
fn actions(message: &Message) -> Vec<MessageAction> {
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
    items
}

/// `Enter` on a message: the action menu.
pub fn menu(state: &mut State) {
    let Some(message) = selected(state) else {
        return;
    };
    let target = message.reference();
    let items = actions(message);
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
    let quote = Quote {
        target,
        preview: message.body.clone(),
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

/// React with `emoji`, or take the reaction back if it's already ours. Shown
/// at once; undone if the backend fails.
fn react(state: &mut State, target: MessageRef, emoji: &str) -> Vec<Effect> {
    let Some(open) = state.open.as_ref() else {
        return vec![];
    };
    let conversation = open.id;
    let Some(message) = find(state, target) else {
        return vec![];
    };
    let previous = own_reaction(state, message).map(str::to_owned);
    let emoji = (previous.as_deref() != Some(emoji)).then(|| emoji.to_owned());
    let event = reaction(state, conversation, target, emoji.clone());
    let mut effects = on_backend(state, event);
    effects.push(Effect::Backend(Command::React {
        conversation,
        target,
        emoji,
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
        .and_then(|m| m.body.as_ref())
        .map(|b| b.expose().to_owned());
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
    let Some(body) = message.body.clone() else {
        return vec![];
    };
    let outgoing = OutgoingMessage {
        sent_at: target.sent_at,
        body,
        quote: message.quote.clone(),
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
