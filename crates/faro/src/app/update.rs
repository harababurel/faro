//! The reducer (D22): `update(state, event) -> effects`. Synchronous and free of
//! I/O, so the UI can never wait on the network or disk here. Store work is
//! returned as `Effect`s.

use crossterm::event::Event;
use faro_core::backend::BackendEvent;
use faro_core::config::BubbleStyle;
use faro_core::domain::{
    ConversationId, Direction, Message, MessageBody, MessageRef, ServiceId, TypingState,
};
use faro_core::store::{ConversationState, ConversationSummary};
use std::time::Duration;

use tokio::time::Instant;

use super::event::{AppEvent, Effect};
use super::input::Input;
use super::keymap;
use super::state::{
    DRAFT_SAVE_DELAY, EmojiMode, Focus, Geometry, Notice, OpenConversation, Popup, SidebarScroll,
    State, TYPING_TIMEOUT, Typist, sort_conversations,
};
use super::system::Terminal;
use super::timeline;

mod curation;
mod navigation;
mod notify;
mod palette;
mod presence;
mod selection;
mod sending;
mod settings;

pub use curation::mentions_toggle;
pub use navigation::mouse_affects_view;
pub use sending::wall_clock;
pub use settings::configure;

/// How many messages to load when a conversation opens.
pub const PAGE: u32 = 200;

pub fn update(state: &mut State, event: AppEvent) -> Vec<Effect> {
    match event {
        AppEvent::Terminal(event) => on_terminal(state, &event),
        AppEvent::Backend(event) => on_backend(state, event),
        AppEvent::Loaded {
            conversations,
            last_open,
        } => on_loaded(state, conversations, last_open),
        AppEvent::MessagesLoaded {
            conversation,
            messages,
        } => on_messages_loaded(state, conversation, messages),
        AppEvent::OlderLoaded {
            conversation,
            messages,
        } => {
            on_older_loaded(state, conversation, messages);
            vec![]
        }
        AppEvent::SendResult { sent_at, result } => sending::on_send_result(state, sent_at, result),
        AppEvent::ReactResult {
            conversation,
            target,
            previous,
            result,
        } => selection::on_react_result(state, conversation, target, previous, result),
        AppEvent::TerminalChecked(terminal) => {
            terminal_checked(state, terminal);
            vec![]
        }
        AppEvent::OpenFailed(error) => {
            state.notice = Some(Notice::error(format!("Couldn't open the link: {error}")));
            vec![]
        }
        AppEvent::ConfigRead(read) => settings::on_config_read(state, read),
        AppEvent::ThemesRead(files) => settings::on_themes_read(state, files),
        AppEvent::SettingNotSaved(error) => {
            state.notice = Some(Notice::error(format!("Couldn't save the setting: {error}")));
            vec![]
        }
        AppEvent::Drawn(geometry) => on_drawn(state, geometry),
        AppEvent::StoreFailed(error) => {
            state.notice = Some(Notice::error(format!("Storage problem: {error}")));
            vec![]
        }
        AppEvent::Tick => {
            let now = Instant::now();
            expire_typing(state, now);
            if state
                .notice
                .as_ref()
                .and_then(|n| n.until)
                .is_some_and(|t| now >= t)
            {
                state.notice = None;
            }
            let draft_due = state
                .open
                .as_ref()
                .and_then(|o| o.draft_edited)
                .is_some_and(|t| now >= t + DRAFT_SAVE_DELAY);
            let mut effects = presence::on_tick(state, now);
            if draft_due {
                effects.extend(save_draft(state));
            }
            effects
        }
        AppEvent::Shutdown => quit(state),
    }
}

fn on_terminal(state: &mut State, event: &Event) -> Vec<Effect> {
    // Keys and clicks only come from a focused terminal, whatever it reported.
    state.env.focused = !matches!(event, Event::FocusLost);
    let mut effects = match event {
        Event::Key(key) => {
            let action = if state.palette.is_some() {
                keymap::palette_action(&state.keymap, *key)
            } else if let Some(popup) = &state.popup {
                keymap::popup_action(*key, matches!(popup, Popup::React { .. }))
            } else {
                state.keymap.lookup(*key, state.focus)
            };
            action.map_or_else(Vec::new, |a| navigation::on_action(state, a))
        }
        Event::Mouse(mouse) if state.palette.is_some() => palette::on_mouse(state, *mouse),
        Event::Mouse(mouse) => navigation::on_mouse(state, *mouse),
        Event::Paste(text) if state.palette.is_some() => {
            palette::paste(state, text);
            vec![]
        }
        Event::Paste(text) => {
            state.popup = None;
            navigation::paste(state, text);
            presence::on_edited(state)
        }
        // A tmux client may have attached from somewhere else (D47).
        Event::FocusGained | Event::Resize(_, _) => check_terminal(state),
        Event::FocusLost => vec![],
    };
    effects.extend(presence::catch_up(state));
    effects
}

/// Take in what the last frame looked like: the normalised scroll position and
/// sidebar offset (which don't change the picture), and load older messages
/// once the top of the loaded history is on screen.
fn on_drawn(state: &mut State, geometry: Geometry) -> Vec<Effect> {
    state.sidebar = SidebarScroll {
        offset: geometry.sidebar_offset,
        reveal: false,
    };
    if geometry.sidebar.is_none() && state.focus == Focus::Conversations {
        // Drawn as input focus already (the sidebar is hidden).
        state.focus = Focus::Input;
    }
    let mut effects = Vec::new();
    if let Some(open) = state.open.as_mut() {
        open.scroll = geometry.scroll;
        if let Some(selection) = open.selected.as_mut() {
            selection.reveal = false;
        }
        if open.scroll.is_none() {
            open.unseen = 0;
        }
        let wants_older = geometry.at_top
            && open.scroll.is_some()
            && open.has_older
            && !open.loading
            && !open.loading_older;
        if wants_older && let Some(first) = open.messages.first() {
            open.loading_older = true;
            effects.push(Effect::LoadOlder {
                conversation: open.id,
                before: first.reference(),
            });
        }
    }
    if let (Some(palette), Some(drawn)) = (state.palette.as_mut(), geometry.palette) {
        palette.offset = drawn.offset;
    }
    state.geometry = geometry;
    // Scrolled back down: what arrived meanwhile has now been seen.
    effects.extend(presence::catch_up(state));
    effects
}

/// Stop: save the draft first (effects still run before the store closes).
fn quit(state: &mut State) -> Vec<Effect> {
    state.quit = true;
    let mut effects = presence::stop_typing(state);
    effects.extend(save_draft(state));
    effects
}

/// Copy the open conversation's input into its summary as the draft and store
/// it, if it changed since the last save.
fn save_draft(state: &mut State) -> Vec<Effect> {
    let Some(open) = state.open.as_mut() else {
        return vec![];
    };
    if open.draft_edited.take().is_none() {
        return vec![];
    }
    let id = open.id;
    let draft = (!open.input.is_empty()).then(|| MessageBody::new(open.input.text()));
    match summary_entry(state, id) {
        Some(summary) => {
            summary.state.draft = draft;
            vec![Effect::SaveConversationState(
                id,
                Box::new(summary.state.clone()),
            )]
        }
        None => vec![],
    }
}

fn open_conversation(state: &mut State, id: ConversationId) -> Vec<Effect> {
    state.sidebar.reveal = true;
    state.popup = None;
    if state.focus == Focus::Messages {
        state.focus = Focus::Input;
    }
    let mut effects = presence::stop_typing(state);
    effects.extend(save_draft(state));
    effects.extend([Effect::LoadMessages(id), Effect::RememberOpen(id)]);
    let mut unread = 0;
    let mut draft = None;
    if let Some(summary) = state.conversations.iter_mut().find(|c| c.id == id) {
        unread = std::mem::take(&mut summary.unread);
        draft = summary.state.draft.as_ref().map(|d| Input::new(d.expose()));
    }
    if unread > 0 {
        effects.push(Effect::MarkRead(id));
    }
    state.open = Some(OpenConversation {
        id,
        messages: Vec::new(),
        loading: true,
        pending: Vec::new(),
        unread_on_open: unread,
        unread_marker: None,
        input: draft.unwrap_or_default(),
        draft_edited: None,
        scroll: None,
        has_older: false,
        loading_older: false,
        unseen: 0,
        selected: None,
        reply_to: None,
        unreceipted: Vec::new(),
        typing_sent: None,
    });
    effects
}

fn on_loaded(
    state: &mut State,
    mut conversations: Vec<ConversationSummary>,
    last_open: Option<ConversationId>,
) -> Vec<Effect> {
    sort_conversations(&mut conversations);
    state.conversations = conversations;
    state.loading = false;
    let first = state.visible_conversations().next().map(|c| c.id);
    let remembered = last_open.filter(|id| state.summary(*id).is_some());
    let Some(id) = remembered.or(first) else {
        return vec![];
    };
    // Show the list the conversation is in (it may be archived).
    if let Some(summary) = state.summary(id) {
        state.list.view = state.section(summary).view();
    }
    open_conversation(state, id)
}

fn on_messages_loaded(
    state: &mut State,
    conversation: ConversationId,
    messages: Vec<Message>,
) -> Vec<Effect> {
    let Some(open) = state
        .open
        .as_mut()
        .filter(|o| o.id == conversation && o.loading)
    else {
        // The user moved on before the page arrived.
        return vec![];
    };
    open.has_older = full_page(&messages);
    open.messages = messages;
    for event in std::mem::take(&mut open.pending) {
        timeline::apply(&mut open.messages, &event);
    }
    open.loading = false;
    let unread = last_unread(&open.messages, open.unread_on_open);
    open.unread_marker = unread.first().copied().or(open.unread_marker);
    presence::on_history(state, &unread);
    presence::catch_up(state)
}

fn on_older_loaded(state: &mut State, conversation: ConversationId, messages: Vec<Message>) {
    let Some(open) = state
        .open
        .as_mut()
        .filter(|o| o.id == conversation && o.loading_older)
    else {
        return;
    };
    open.loading_older = false;
    open.has_older = full_page(&messages);
    timeline::prepend(&mut open.messages, messages);
}

/// A full page means there may be more before it.
fn full_page(messages: &[Message]) -> bool {
    u32::try_from(messages.len()).is_ok_and(|n| n >= PAGE)
}

/// The last `unread` incoming messages, oldest first.
fn last_unread(messages: &[Message], unread: u32) -> Vec<MessageRef> {
    let n = usize::try_from(unread).unwrap_or(usize::MAX);
    let mut refs: Vec<MessageRef> = messages
        .iter()
        .rev()
        .filter(|m| m.direction == Direction::Incoming && !m.deleted)
        .take(n)
        .map(Message::reference)
        .collect();
    refs.reverse();
    refs
}

fn on_backend(state: &mut State, event: BackendEvent) -> Vec<Effect> {
    let mut effects = Vec::new();
    match &event {
        BackendEvent::Connection(connection) => state.connection = *connection,
        BackendEvent::Message(message) => on_message(state, message, &mut effects),
        BackendEvent::DeliveryUpdate { .. } => apply_to_open(state, None, &event),
        BackendEvent::Reaction { conversation, .. }
        | BackendEvent::Edited { conversation, .. }
        | BackendEvent::Deleted { conversation, .. } => {
            apply_to_open(state, Some(*conversation), &event);
        }
        BackendEvent::Typing {
            conversation,
            sender,
            state: typing,
        } => on_typing(state, *conversation, *sender, *typing),
        BackendEvent::ContactUpdated(contact) => {
            let id = ConversationId::Direct(contact.service_id);
            let title = contact.display_name().map(str::to_owned);
            let phone = contact.phone.clone();
            state.contacts.insert(contact.service_id, contact.clone());
            if let Some(summary) = state.conversations.iter_mut().find(|c| c.id == id) {
                summary.title = title;
                summary.phone = phone;
            }
        }
        BackendEvent::GroupUpdated(group) => {
            let id = ConversationId::Group(group.id);
            state.groups.insert(group.id, group.clone());
            if let Some(summary) = summary_entry(state, id) {
                summary.title = Some(group.title.clone());
            }
        }
        BackendEvent::Error(error) => {
            state.notice = Some(Notice::warning(format!("Signal: {error}")));
        }
    }
    if persistent(&event) {
        effects.insert(0, Effect::Persist(Box::new(event)));
    }
    effects
}

/// Whether the store keeps anything from this event.
fn persistent(event: &BackendEvent) -> bool {
    match event {
        BackendEvent::Message(_)
        | BackendEvent::DeliveryUpdate { .. }
        | BackendEvent::Reaction { .. }
        | BackendEvent::Edited { .. }
        | BackendEvent::Deleted { .. }
        | BackendEvent::ContactUpdated(_)
        | BackendEvent::GroupUpdated(_) => true,
        BackendEvent::Connection(_) | BackendEvent::Typing { .. } | BackendEvent::Error(_) => false,
    }
}

fn on_message(state: &mut State, message: &Message, effects: &mut Vec<Effect>) {
    let id = message.conversation;
    if let Some(typists) = state.typing.get_mut(&id) {
        typists.retain(|t| t.who != message.sender);
    }
    let is_open = state.open.as_ref().is_some_and(|o| o.id == id);
    // Unread unless it's in front of the user (D42).
    let seen = is_open && state.seeing_open();
    let incoming = message.direction == Direction::Incoming && !message.deleted;
    if let Some(summary) = summary_entry(state, id) {
        summary.last_activity = summary.last_activity.max(Some(message.sent_at));
        if incoming && !seen {
            summary.unread += 1;
        }
        // Hidden until a new message arrives (DESIGN.md §7.7).
        if summary.state.hidden && !message.deleted {
            summary.state.hidden = false;
            effects.push(Effect::SaveConversationState(
                id,
                Box::new(summary.state.clone()),
            ));
        }
    }
    if incoming && !seen {
        effects.extend(notify::on_unseen_message(state, id));
    }
    sort_conversations(&mut state.conversations);
    if state.open.is_none() && !state.loading {
        // First run: the list was empty at startup, so open where things happen.
        // `Persist` goes first, so the loaded page includes this message.
        effects.extend(open_conversation(state, id));
        return;
    }
    if is_open {
        apply_to_open(state, Some(id), &BackendEvent::Message(message.clone()));
        if let Some(open) = state.open.as_mut()
            && open.scroll.is_some()
        {
            open.unseen += 1;
        }
        presence::on_new_message(state, message);
        // Persisted first (effects run in order), so a `MarkRead` here clears
        // the count the store just raised.
        effects.extend(presence::catch_up(state));
    }
}

/// The summary for `id`, created if this is a new conversation. (Always `Some`;
/// an `Option` only because getting it back out of the `Vec` can't be proven.)
fn summary_entry(state: &mut State, id: ConversationId) -> Option<&mut ConversationSummary> {
    if let Some(at) = state.conversations.iter().position(|c| c.id == id) {
        return state.conversations.get_mut(at);
    }
    let (title, phone) = match id {
        ConversationId::Direct(peer) => state.contacts.get(&peer).map_or((None, None), |c| {
            (c.display_name().map(str::to_owned), c.phone.clone())
        }),
        ConversationId::Group(group) => (state.groups.get(&group).map(|g| g.title.clone()), None),
    };
    state.conversations.push(ConversationSummary {
        id,
        title,
        phone,
        last_activity: None,
        unread: 0,
        state: ConversationState::default(),
    });
    state.conversations.last_mut()
}

/// Apply to the open conversation if it's `conversation` (or, for receipts, which
/// don't say, whichever is open). While its history loads, the event waits.
fn apply_to_open(state: &mut State, conversation: Option<ConversationId>, event: &BackendEvent) {
    let Some(open) = state.open.as_mut() else {
        return;
    };
    if conversation.is_some_and(|c| c != open.id) {
        return;
    }
    if open.loading {
        open.pending.push(event.clone());
    } else {
        timeline::apply(&mut open.messages, event);
    }
}

fn on_typing(state: &mut State, conversation: ConversationId, who: ServiceId, typing: TypingState) {
    let typists = state.typing.entry(conversation).or_default();
    typists.retain(|t| t.who != who);
    match typing {
        TypingState::Started => typists.push(Typist {
            who,
            since: Instant::now(),
        }),
        TypingState::Stopped => {}
    }
    if typists.is_empty() {
        state.typing.remove(&conversation);
    }
}

fn expire_typing(state: &mut State, now: Instant) {
    state.typing.retain(|_, typists| {
        typists.retain(|t| now < t.since + TYPING_TIMEOUT);
        !typists.is_empty()
    });
}

#[cfg(test)]
mod tests;

/// Don't look for mosh more often than this (resizes come in bursts).
const TERMINAL_RECHECK: Duration = Duration::from_secs(2);

/// Look at the terminal again, if the emoji mode follows mosh (D47) or the
/// bubble style follows kitty (D58).
fn check_terminal(state: &mut State) -> Vec<Effect> {
    let follows = state.env.emoji.auto || state.config.messages.bubble_style == BubbleStyle::Auto;
    let emoji = &mut state.env.emoji;
    let now = Instant::now();
    if !follows || emoji.checked_at.is_some_and(|t| now - t < TERMINAL_RECHECK) {
        return vec![];
    }
    emoji.checked_at = Some(now);
    vec![Effect::CheckTerminal]
}

/// Switch the emoji mode to match, redrawing everything if it changed, and
/// note who draws block glyphs (the next frame picks the bubble style).
fn terminal_checked(state: &mut State, terminal: Terminal) {
    let Terminal { mosh, glyphs } = terminal;
    state.env.glyphs = glyphs;
    let emoji = &mut state.env.emoji;
    let mode = if mosh {
        EmojiMode::Compatible
    } else {
        EmojiMode::Full
    };
    if !emoji.auto || emoji.mode == mode {
        return;
    }
    emoji.mode = mode;
    state.env.repaint = true;
    tracing::info!(?mode, "emoji mode changed");
    state.notice = Some(Notice::info(if mosh {
        "mosh detected: drawing emoji in compatible form"
    } else {
        "No mosh: drawing emoji in full"
    }));
}
