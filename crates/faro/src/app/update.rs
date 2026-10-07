//! The reducer (D22): `update(state, event) -> effects`. Synchronous and free of
//! I/O, so the UI can never wait on the network or disk here. Store work is
//! returned as `Effect`s.

use crossterm::event::Event;
use faro_core::backend::BackendEvent;
use faro_core::domain::{ConversationId, Direction, Message, MessageRef, ServiceId, TypingState};
use faro_core::store::{ConversationState, ConversationSummary};
use tokio::time::Instant;

use super::event::{AppEvent, Effect};
use super::state::{
    Focus, Geometry, Notice, NoticeLevel, OpenConversation, SidebarScroll, State, TYPING_TIMEOUT,
    Typist, sort_conversations,
};
use super::timeline;

mod keys;
mod navigation;

pub use navigation::mouse_affects_view;

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
        } => {
            on_messages_loaded(state, conversation, messages);
            vec![]
        }
        AppEvent::OlderLoaded {
            conversation,
            messages,
        } => {
            on_older_loaded(state, conversation, messages);
            vec![]
        }
        AppEvent::Drawn(geometry) => on_drawn(state, geometry),
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
        Event::Key(key) => keys::key_action(*key, state.focus)
            .map_or_else(Vec::new, |a| navigation::on_action(state, a)),
        Event::Mouse(mouse) => navigation::on_mouse(state, *mouse),
        Event::FocusGained | Event::FocusLost | Event::Paste(_) | Event::Resize(_, _) => vec![],
    }
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
    state.geometry = geometry;
    effects
}

fn open_conversation(state: &mut State, id: ConversationId) -> Vec<Effect> {
    state.sidebar.reveal = true;
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
        scroll: None,
        has_older: false,
        loading_older: false,
        unseen: 0,
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
    open.has_older = full_page(&messages);
    open.messages = messages;
    for event in std::mem::take(&mut open.pending) {
        timeline::apply(&mut open.messages, &event);
    }
    open.loading = false;
    open.unread_marker = first_unread(&open.messages, open.unread_on_open);
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
        if let Some(open) = state.open.as_mut()
            && open.scroll.is_some()
        {
            open.unseen += 1;
        }
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
