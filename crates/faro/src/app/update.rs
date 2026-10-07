//! The reducer (D22): `update(state, event) -> effects`. Synchronous and free of
//! I/O, so the UI can never wait on the network or disk here. Store work is
//! returned as `Effect`s.

use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use faro_core::backend::BackendEvent;
use faro_core::domain::{ConversationId, Direction, Message, MessageRef, ServiceId, TypingState};
use faro_core::store::{ConversationState, ConversationSummary};
use tokio::time::Instant;
use unicode_segmentation::UnicodeSegmentation as _;

use super::event::{AppEvent, Effect};
use super::state::{
    Notice, NoticeLevel, OpenConversation, State, TYPING_TIMEOUT, Typist, sort_conversations,
};
use super::timeline;

/// How many messages to load when a conversation opens.
pub const PAGE: u32 = 200;

/// What a key does. The keybinding system (M4) will map config to these.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    Quit,
    /// Ctrl+C: quits on the second press (D13).
    Interrupt,
    PreviousConversation,
    NextConversation,
    ToggleLeftSidebar,
    ToggleRightSidebar,
    Dismiss,
    DeleteBackward,
    Insert(char),
}

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
        } => {
            on_messages_loaded(state, conversation, messages);
            vec![]
        }
        AppEvent::StoreFailed(error) => {
            state.notice = Some(Notice {
                text: format!("Storage problem: {error}"),
                level: NoticeLevel::Error,
            });
            vec![]
        }
        AppEvent::Tick => {
            expire_typing(state, Instant::now());
            vec![]
        }
        AppEvent::Shutdown => {
            state.quit = true;
            vec![]
        }
    }
}

fn on_terminal(state: &mut State, event: &Event) -> Vec<Effect> {
    match event {
        Event::Key(key) => key_action(*key).map_or_else(Vec::new, |a| on_action(state, a)),
        Event::FocusGained
        | Event::FocusLost
        | Event::Mouse(_)
        | Event::Paste(_)
        | Event::Resize(_, _) => vec![],
    }
}

/// Default bindings (DESIGN.md §7.6) for what M3 supports.
fn key_action(key: KeyEvent) -> Option<Action> {
    if key.kind == KeyEventKind::Release {
        return None;
    }
    // Shift only changes which character arrives.
    let modifiers = key.modifiers.difference(KeyModifiers::SHIFT);
    // Most keys do nothing, hence the catch-all (on a tuple, not an enum of ours).
    match (key.code, modifiers) {
        (KeyCode::Char('q'), KeyModifiers::CONTROL) => Some(Action::Quit),
        (KeyCode::Char('c'), KeyModifiers::CONTROL) => Some(Action::Interrupt),
        (KeyCode::Char('k' | 'K'), KeyModifiers::ALT) | (KeyCode::Up, KeyModifiers::ALT) => {
            Some(Action::PreviousConversation)
        }
        (KeyCode::Char('j' | 'J'), KeyModifiers::ALT) | (KeyCode::Down, KeyModifiers::ALT) => {
            Some(Action::NextConversation)
        }
        (KeyCode::Char('s' | 'S'), KeyModifiers::ALT) => Some(Action::ToggleLeftSidebar),
        (KeyCode::Char('i' | 'I'), KeyModifiers::ALT) => Some(Action::ToggleRightSidebar),
        (KeyCode::Esc, KeyModifiers::NONE) => Some(Action::Dismiss),
        (KeyCode::Backspace, KeyModifiers::NONE) => Some(Action::DeleteBackward),
        (KeyCode::Char(c), KeyModifiers::NONE) => Some(Action::Insert(c)),
        _ => None,
    }
}

fn on_action(state: &mut State, action: Action) -> Vec<Effect> {
    if action != Action::Interrupt && state.quit_armed {
        state.quit_armed = false;
        state.notice = None;
    }
    match action {
        Action::Quit => state.quit = true,
        Action::Interrupt => {
            if state.quit_armed {
                state.quit = true;
            } else {
                state.quit_armed = true;
                state.notice = Some(Notice {
                    text: "Press Ctrl+C again to quit (Ctrl+Q quits at once)".to_owned(),
                    level: NoticeLevel::Warning,
                });
            }
        }
        Action::PreviousConversation => return step_conversation(state, -1),
        Action::NextConversation => return step_conversation(state, 1),
        Action::ToggleLeftSidebar => state.panels.left = !state.panels.left,
        Action::ToggleRightSidebar => state.panels.right = !state.panels.right,
        Action::Dismiss => state.notice = None,
        Action::DeleteBackward => {
            if let Some(open) = &mut state.open
                && let Some((at, _)) = open.input.grapheme_indices(true).next_back()
            {
                open.input.truncate(at);
            }
        }
        Action::Insert(c) => {
            if let Some(open) = &mut state.open {
                open.input.push(c);
            }
        }
    }
    vec![]
}

fn step_conversation(state: &mut State, step: isize) -> Vec<Effect> {
    let ids: Vec<ConversationId> = state.visible_conversations().map(|c| c.id).collect();
    let current = state
        .open
        .as_ref()
        .and_then(|o| ids.iter().position(|id| *id == o.id));
    let next = match current {
        Some(i) => i
            .saturating_add_signed(step)
            .min(ids.len().saturating_sub(1)),
        None => 0,
    };
    match ids.get(next) {
        Some(&id) if Some(next) != current => open_conversation(state, id),
        Some(_) | None => vec![],
    }
}

fn open_conversation(state: &mut State, id: ConversationId) -> Vec<Effect> {
    if let Some(previous) = state.open.take()
        && !previous.input.is_empty()
    {
        state.drafts.insert(previous.id, previous.input);
    }
    let mut effects = vec![Effect::LoadMessages(id), Effect::RememberOpen(id)];
    let mut unread = 0;
    if let Some(summary) = state.conversations.iter_mut().find(|c| c.id == id) {
        unread = std::mem::take(&mut summary.unread);
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
        input: state.drafts.remove(&id).unwrap_or_default(),
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
    match remembered.or(first) {
        Some(id) => open_conversation(state, id),
        None => vec![],
    }
}

fn on_messages_loaded(state: &mut State, conversation: ConversationId, messages: Vec<Message>) {
    let Some(open) = state
        .open
        .as_mut()
        .filter(|o| o.id == conversation && o.loading)
    else {
        // The user moved on before the page arrived.
        return;
    };
    open.messages = messages;
    for event in std::mem::take(&mut open.pending) {
        timeline::apply(&mut open.messages, &event);
    }
    open.loading = false;
    open.unread_marker = first_unread(&open.messages, open.unread_on_open);
}

/// The `unread`-th incoming message from the end.
fn first_unread(messages: &[Message], unread: u32) -> Option<MessageRef> {
    let n = usize::try_from(unread).ok().filter(|n| *n > 0)?;
    messages
        .iter()
        .rev()
        .filter(|m| m.direction == Direction::Incoming && !m.deleted)
        .take(n)
        .last()
        .map(Message::reference)
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
            state.notice = Some(Notice {
                text: format!("Signal: {error}"),
                level: NoticeLevel::Warning,
            });
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
    let incoming = message.direction == Direction::Incoming && !message.deleted;
    if let Some(summary) = summary_entry(state, id) {
        summary.last_activity = summary.last_activity.max(Some(message.sent_at));
        if incoming && !is_open {
            summary.unread += 1;
        }
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
        if incoming {
            // Persisted first (effects run in order), so this clears the count
            // the store just raised.
            effects.push(Effect::MarkRead(id));
        }
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
