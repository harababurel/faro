//! Curating the conversation list (M4 task 5, D41): pin, mute, hide, archive,
//! the allowlist, and switching between the chats and the archive.

use faro_core::config::key;
use faro_core::domain::{ConversationId, Timestamp};
use faro_core::store::{ConversationState, NotifyLevel};

use super::selection::cycle;
use super::sending::wall_clock;
use super::{open_conversation, save_draft, settings};
use crate::app::event::Effect;
use crate::app::keymap::{Curate, PopupKey};
use crate::app::palette::INACTIVE_CHOICES;
use crate::app::state::{
    ConversationAction, Focus, MUTE_CHOICES, Notice, Popup, Section, SidebarScroll, SidebarView,
    State, sort_conversations,
};

/// A sidebar key. Acts on the open conversation, which is the sidebar's
/// highlight, as long as it's listed in the current view.
pub fn on_curate(state: &mut State, curate: Curate) -> Vec<Effect> {
    let target = state
        .highlighted()
        .filter(|id| state.visible_conversations().any(|c| c.id == *id));
    match (curate, target) {
        (Curate::SwitchView, _) => {
            switch_view(state);
            vec![]
        }
        (Curate::AllowlistMode, _) => set_allowlist_mode(state, !state.list.allowlist),
        (Curate::Menu, Some(target)) => {
            menu(state, target);
            vec![]
        }
        (_, Some(target)) => match toggle(state, target, curate) {
            Some(action) => perform(state, target, action),
            None => vec![],
        },
        (_, None) => vec![],
    }
}

/// What a toggle key means for `target` right now.
fn toggle(state: &State, target: ConversationId, curate: Curate) -> Option<ConversationAction> {
    let current = &state.summary(target)?.state;
    let action = match curate {
        Curate::Pin if current.pinned.is_some() => ConversationAction::Unpin,
        Curate::Pin => ConversationAction::Pin,
        Curate::MoveUp => ConversationAction::MoveUp,
        Curate::MoveDown => ConversationAction::MoveDown,
        Curate::Mute if current.muted_at(wall_clock()) => ConversationAction::Unmute,
        Curate::Mute => ConversationAction::Mute,
        Curate::Mentions => mentions_toggle(current),
        Curate::Hide if current.hidden => ConversationAction::Unhide,
        Curate::Hide => ConversationAction::Hide,
        Curate::Archive if current.archived => ConversationAction::Unarchive,
        Curate::Archive => ConversationAction::Archive,
        Curate::Allow if current.allowed => ConversationAction::Disallow,
        Curate::Allow => ConversationAction::Allow,
        Curate::AllowlistMode => ConversationAction::AllowlistMode(!state.list.allowlist),
        Curate::Menu | Curate::SwitchView => return None,
    };
    Some(action)
}

/// `Space` or right-click on a conversation: its menu.
pub fn menu(state: &mut State, target: ConversationId) {
    let Some(summary) = state.summary(target) else {
        return;
    };
    let current = &summary.state;
    let pins = state
        .conversations
        .iter()
        .filter(|c| c.state.pinned.is_some())
        .count();
    let mut items = Vec::with_capacity(8);
    if current.pinned.is_none() {
        items.push(ConversationAction::Pin);
    } else {
        items.push(ConversationAction::Unpin);
        if pins > 1 {
            items.extend([ConversationAction::MoveUp, ConversationAction::MoveDown]);
        }
    }
    items.push(if current.muted_at(wall_clock()) {
        ConversationAction::Unmute
    } else {
        ConversationAction::Mute
    });
    items.push(mentions_toggle(current));
    items.push(if current.hidden {
        ConversationAction::Unhide
    } else {
        ConversationAction::Hide
    });
    items.push(if current.archived {
        ConversationAction::Unarchive
    } else {
        ConversationAction::Archive
    });
    items.push(if current.allowed {
        ConversationAction::Disallow
    } else {
        ConversationAction::Allow
    });
    items.push(ConversationAction::AllowlistMode(!state.list.allowlist));
    state.popup = Some(Popup::Conversation {
        target,
        items,
        selected: 0,
    });
}

/// Switch between "mentions only" and "every message".
pub fn mentions_toggle(current: &ConversationState) -> ConversationAction {
    match current.notify {
        NotifyLevel::All => ConversationAction::MentionsOnly,
        NotifyLevel::Mentions => ConversationAction::AllMessages,
    }
}

/// A key in the conversation menu. Shortcuts are the sidebar keys.
pub fn on_menu_key(
    state: &mut State,
    target: ConversationId,
    items: Vec<ConversationAction>,
    selected: usize,
    key: PopupKey,
) -> Vec<Effect> {
    let chosen = match key {
        PopupKey::Previous | PopupKey::Next => {
            let selected = cycle(selected, items.len(), key == PopupKey::Next);
            state.popup = Some(Popup::Conversation {
                target,
                items,
                selected,
            });
            return vec![];
        }
        PopupKey::Choose => items.get(selected).copied(),
        PopupKey::Char(c) => items
            .iter()
            .copied()
            .find(|a| a.label().1.chars().eq(std::iter::once(c))),
        PopupKey::Close => None,
    };
    chosen.map_or_else(Vec::new, |action| perform(state, target, action))
}

/// A key in the mute picker: arrows and `Enter`, or `1`–`5`.
pub fn on_mute_key(
    state: &mut State,
    target: ConversationId,
    selected: usize,
    key: PopupKey,
) -> Vec<Effect> {
    let chosen = match key {
        PopupKey::Previous | PopupKey::Next => {
            let selected = cycle(selected, MUTE_CHOICES.len(), key == PopupKey::Next);
            state.popup = Some(Popup::Mute { target, selected });
            return vec![];
        }
        PopupKey::Choose => Some(selected),
        PopupKey::Char(c) => c
            .to_digit(10)
            .and_then(|d| usize::try_from(d).ok())
            .and_then(|d| d.checked_sub(1)),
        PopupKey::Close => None,
    };
    chosen.map_or_else(Vec::new, |choice| mute_for(state, target, choice))
}

/// Mute `target` for `MUTE_CHOICES[choice]`.
pub fn mute_for(state: &mut State, target: ConversationId, choice: usize) -> Vec<Effect> {
    let Some(&(label, length)) = MUTE_CHOICES.get(choice) else {
        return vec![];
    };
    let until = length.map_or(ConversationState::MUTED_FOREVER, |length| {
        let length = u64::try_from(length.as_millis()).unwrap_or(u64::MAX);
        Timestamp::from_millis(wall_clock().as_millis().saturating_add(length))
    });
    let notice = if length.is_some() {
        format!("Muted for {label}")
    } else {
        "Muted until you unmute it".to_owned()
    };
    change(state, target, notice, |s| s.muted_until = Some(until))
}

/// Hide every conversation with no message for `INACTIVE_CHOICES[choice]`
/// (DESIGN §7.7), except pinned ones; like `h`, each comes back with a new
/// message.
pub fn hide_inactive(state: &mut State, choice: usize) -> Vec<Effect> {
    const DAY_MS: u64 = 24 * 60 * 60 * 1000;
    let Some(&(days, label)) = INACTIVE_CHOICES.get(choice) else {
        return vec![];
    };
    let cutoff = wall_clock()
        .as_millis()
        .saturating_sub(u64::from(days) * DAY_MS);
    let before = visible_ids(state);
    let mut effects = Vec::new();
    for summary in &mut state.conversations {
        let current = &summary.state;
        let inactive = summary
            .last_activity
            .is_some_and(|t| t.as_millis() < cutoff);
        if inactive && current.pinned.is_none() && !current.hidden && !current.archived {
            summary.state.hidden = true;
            effects.push(Effect::SaveConversationState(
                summary.id,
                Box::new(summary.state.clone()),
            ));
        }
    }
    let notice = match effects.len() {
        0 => format!("No conversations inactive for {label}"),
        1 => format!("Hid 1 conversation inactive for {label}"),
        n => format!("Hid {n} conversations inactive for {label}"),
    };
    state.notice = Some(Notice::info(notice));
    state.sidebar.reveal = true;
    if let Some(open) = state.open.as_ref().map(|o| o.id) {
        effects.extend(follow(state, open, &before));
    }
    effects
}

pub fn perform(
    state: &mut State,
    target: ConversationId,
    action: ConversationAction,
) -> Vec<Effect> {
    match action {
        ConversationAction::Mute => {
            state.popup = Some(Popup::Mute {
                target,
                selected: 0,
            });
            vec![]
        }
        ConversationAction::MoveUp => move_pin(state, target, true),
        ConversationAction::MoveDown => move_pin(state, target, false),
        ConversationAction::AllowlistMode(on) => set_allowlist_mode(state, on),
        ConversationAction::Pin => {
            let next = state
                .conversations
                .iter()
                .filter_map(|c| c.state.pinned)
                .max()
                .map_or(0, |p| p.saturating_add(1));
            let effects = change(state, target, String::new(), |s| {
                s.pinned = Some(next);
                s.hidden = false;
                s.archived = false;
            });
            state.notice = Some(Notice::info(pin_notice(state, target)));
            effects
        }
        ConversationAction::Unpin => change(state, target, "Unpinned".into(), |s| s.pinned = None),
        ConversationAction::Unmute => {
            change(state, target, "Unmuted".into(), |s| s.muted_until = None)
        }
        ConversationAction::MentionsOnly => {
            change(state, target, "Notifying for mentions only".into(), |s| {
                s.notify = NotifyLevel::Mentions;
            })
        }
        ConversationAction::AllMessages => {
            change(state, target, "Notifying for every message".into(), |s| {
                s.notify = NotifyLevel::All;
            })
        }
        ConversationAction::Hide => {
            let notice = format!(
                "Hid {} until a new message arrives",
                state.conversation_title(target)
            );
            change(state, target, notice, |s| {
                s.hidden = true;
                s.pinned = None;
            })
        }
        ConversationAction::Unhide => {
            change(state, target, "Unhidden".into(), |s| s.hidden = false)
        }
        ConversationAction::Archive => {
            let notice = format!(
                "Archived {} (A shows the archive)",
                state.conversation_title(target)
            );
            change(state, target, notice, |s| {
                s.archived = true;
                s.hidden = false;
                s.pinned = None;
            })
        }
        ConversationAction::Unarchive => {
            change(state, target, "Unarchived".into(), |s| s.archived = false)
        }
        ConversationAction::Allow => change(state, target, "Added to the allowlist".into(), |s| {
            s.allowed = true;
        }),
        ConversationAction::Disallow => {
            change(state, target, "Removed from the allowlist".into(), |s| {
                s.allowed = false;
            })
        }
    }
}

/// Change `target`'s state, store it, re-sort the list, show `notice`, and
/// move on if the open conversation left the current view.
fn change(
    state: &mut State,
    target: ConversationId,
    notice: String,
    edit: impl FnOnce(&mut ConversationState),
) -> Vec<Effect> {
    let before = visible_ids(state);
    let Some(summary) = state.conversations.iter_mut().find(|c| c.id == target) else {
        return vec![];
    };
    edit(&mut summary.state);
    let mut effects = vec![Effect::SaveConversationState(
        target,
        Box::new(summary.state.clone()),
    )];
    sort_conversations(&mut state.conversations);
    state.sidebar.reveal = true;
    if !notice.is_empty() {
        state.notice = Some(Notice::info(notice));
    }
    effects.extend(follow(state, target, &before));
    effects
}

fn visible_ids(state: &State) -> Vec<ConversationId> {
    state.visible_conversations().map(|c| c.id).collect()
}

/// "Pinned (Alt+N)" while it has a shortcut.
fn pin_notice(state: &State, target: ConversationId) -> String {
    let place = pinned(state).iter().position(|id| *id == target);
    match place {
        Some(i) if i < 9 => format!("Pinned (Alt+{} opens it)", i + 1),
        Some(_) | None => "Pinned".to_owned(),
    }
}

/// Pinned conversations in order.
fn pinned(state: &State) -> Vec<ConversationId> {
    state
        .listed_in(SidebarView::Chats)
        .filter(|c| state.section(c) == Section::Pinned)
        .map(|c| c.id)
        .collect()
}

/// If the open conversation `target` is no longer listed in the current view
/// (hidden, archived, unarchived…), open the one that took its place in
/// `before`. With nothing left: from the archive, go back to the chats (where
/// it is now); from the chats, close it.
fn follow(state: &mut State, target: ConversationId, before: &[ConversationId]) -> Vec<Effect> {
    let is_open = state.open.as_ref().is_some_and(|o| o.id == target);
    if !is_open || state.visible_conversations().any(|c| c.id == target) {
        return vec![];
    }
    let at = before.iter().position(|id| *id == target).unwrap_or(0);
    let now = visible_ids(state);
    match now.get(at).or_else(|| now.last()).copied() {
        Some(next) => open_conversation(state, next),
        None if state.list.view == SidebarView::Archive => {
            state.list.view = SidebarView::Chats;
            state.sidebar.reveal = true;
            vec![]
        }
        None => close(state),
    }
}

/// Close the open conversation (`Esc`, or nothing left to show): its draft
/// is saved and the list, shown and focused, highlights it (D60).
pub fn close(state: &mut State) -> Vec<Effect> {
    let mut effects = super::presence::stop_typing(state);
    effects.extend(save_draft(state));
    state.list.cursor = state.open.take().map(|o| o.id);
    state.popup = None;
    state.panels.left = true;
    state.sidebar.reveal = true;
    state.focus = Focus::Conversations;
    effects
}

/// Swap a pinned conversation with its neighbour (`up`: the one before it),
/// numbering the pins 0, 1, 2… again; stores those whose position changed.
fn move_pin(state: &mut State, target: ConversationId, up: bool) -> Vec<Effect> {
    let mut order = pinned(state);
    let Some(at) = order.iter().position(|id| *id == target) else {
        return vec![];
    };
    let other = if up {
        at.checked_sub(1)
    } else {
        Some(at + 1).filter(|i| *i < order.len())
    };
    let Some(other) = other else {
        return vec![];
    };
    order.swap(at, other);
    let mut effects = Vec::new();
    for (position, id) in order.into_iter().enumerate() {
        let position = u32::try_from(position).ok();
        if let Some(summary) = state.conversations.iter_mut().find(|c| c.id == id)
            && summary.state.pinned != position
        {
            summary.state.pinned = position;
            effects.push(Effect::SaveConversationState(
                id,
                Box::new(summary.state.clone()),
            ));
        }
    }
    sort_conversations(&mut state.conversations);
    state.sidebar.reveal = true;
    effects
}

/// Turn allowlist mode on or off and save it to the config file (D48).
pub fn set_allowlist_mode(state: &mut State, on: bool) -> Vec<Effect> {
    state.notice = Some(Notice::info(if on {
        "Allowlist mode on: only allowed and pinned conversations are listed"
    } else {
        "Allowlist mode off"
    }));
    state.config.conversations.allowlist = on;
    let mut effects = apply_allowlist_mode(state, on);
    effects.extend(settings::save(state, key::ALLOWLIST));
    effects
}

/// Turn allowlist mode on or off (also when the config file changes),
/// moving on if the open conversation leaves the list.
pub fn apply_allowlist_mode(state: &mut State, on: bool) -> Vec<Effect> {
    let before = visible_ids(state);
    state.list.allowlist = on;
    state.sidebar.reveal = true;
    match state.open.as_ref().map(|o| o.id) {
        Some(open) => follow(state, open, &before),
        None => vec![],
    }
}

/// `A`, or a click on the switch row: chats ⇄ archive.
pub fn switch_view(state: &mut State) {
    state.list.view = match state.list.view {
        SidebarView::Chats => SidebarView::Archive,
        SidebarView::Archive => SidebarView::Chats,
    };
    state.sidebar = SidebarScroll {
        offset: 0,
        reveal: true,
    };
}

/// `Alt+N`: open the Nth pinned conversation.
pub fn jump_to_pinned(state: &mut State, n: u8) -> Vec<Effect> {
    let Some(id) = pinned(state).get(usize::from(n).saturating_sub(1)).copied() else {
        state.notice = Some(Notice::info(format!("Nothing pinned at Alt+{n}")));
        return vec![];
    };
    state.list.view = SidebarView::Chats;
    state.sidebar.reveal = true;
    if state.open.as_ref().is_some_and(|o| o.id == id) {
        vec![]
    } else {
        open_conversation(state, id)
    }
}
