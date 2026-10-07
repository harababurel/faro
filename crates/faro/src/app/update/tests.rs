//! Reducer tests: event in → state and effects out.

use std::time::Duration;

use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use faro_core::backend::BackendEvent;
use faro_core::domain::{MessageBody, Timestamp, TypingState};

use super::*;
use crate::fixtures::{
    self, ALICE, BOB, CAROL, ME, alice, bob, climbing, climbing_history, loaded_state, text,
};

fn key(code: KeyCode, modifiers: KeyModifiers) -> AppEvent {
    AppEvent::Terminal(Event::Key(KeyEvent::new(code, modifiers)))
}

fn typed(state: &mut State, text: &str) {
    for c in text.chars() {
        update(state, key(KeyCode::Char(c), KeyModifiers::NONE));
    }
}

fn open_id(state: &State) -> Option<ConversationId> {
    state.open.as_ref().map(|o| o.id)
}

#[test]
fn startup_opens_the_remembered_conversation() {
    let mut state = State::new(ME, fixtures::contacts(), fixtures::groups());
    assert!(state.loading);
    let effects = update(
        &mut state,
        AppEvent::Loaded {
            conversations: fixtures::summaries(),
            last_open: Some(climbing()),
        },
    );
    assert_eq!(
        effects,
        vec![
            Effect::LoadMessages(climbing()),
            Effect::RememberOpen(climbing()),
            Effect::MarkRead(climbing()),
        ]
    );
    assert!(!state.loading);
    assert_eq!(open_id(&state), Some(climbing()));
    assert_eq!(state.summary(climbing()).map(|s| s.unread), Some(0));
    // Pinned first, then by activity.
    let order: Vec<_> = state.visible_conversations().map(|c| c.id).collect();
    assert_eq!(order, vec![bob(), climbing(), alice()]);
}

#[test]
fn startup_falls_back_to_the_first_conversation() {
    let mut state = State::new(ME, vec![], vec![]);
    update(
        &mut state,
        AppEvent::Loaded {
            conversations: fixtures::summaries(),
            last_open: None,
        },
    );
    assert_eq!(open_id(&state), Some(bob()));
}

#[test]
fn first_message_opens_a_conversation_on_a_fresh_start() {
    let mut state = State::new(ME, fixtures::contacts(), vec![]);
    // Contacts exist but have no messages yet, so nothing is visible.
    let mut empty = fixtures::summary(alice(), "Alice", 0, 0);
    empty.last_activity = None;
    update(
        &mut state,
        AppEvent::Loaded {
            conversations: vec![empty],
            last_open: None,
        },
    );
    assert_eq!(open_id(&state), None);

    let first = text(alice(), ALICE, fixtures::NOW_MS, "hello?");
    let effects = update(
        &mut state,
        AppEvent::Backend(BackendEvent::Message(first.clone())),
    );
    assert_eq!(open_id(&state), Some(alice()));
    assert_eq!(
        effects,
        vec![
            Effect::Persist(Box::new(BackendEvent::Message(first))),
            Effect::LoadMessages(alice()),
            Effect::RememberOpen(alice()),
            Effect::MarkRead(alice()),
        ]
    );
}

#[test]
fn events_during_loading_are_replayed_on_the_loaded_page() {
    let mut state = State::new(ME, vec![], vec![]);
    update(
        &mut state,
        AppEvent::Loaded {
            conversations: fixtures::summaries(),
            last_open: Some(climbing()),
        },
    );
    let history = climbing_history();
    let last = history.last().cloned().unwrap();
    let new = text(
        climbing(),
        BOB,
        last.sent_at.as_millis() + 1,
        "new while loading",
    );
    update(
        &mut state,
        AppEvent::Backend(BackendEvent::Message(new.clone())),
    );
    update(
        &mut state,
        AppEvent::Backend(BackendEvent::Edited {
            conversation: climbing(),
            target: last.reference(),
            body: MessageBody::new("edited while loading"),
            edited_at: Timestamp::from_millis(last.sent_at.as_millis() + 2),
        }),
    );
    update(
        &mut state,
        AppEvent::MessagesLoaded {
            conversation: climbing(),
            messages: history.clone(),
        },
    );
    let open = state.open.as_ref().unwrap();
    assert!(!open.loading);
    assert_eq!(open.messages.len(), history.len() + 1);
    assert_eq!(open.messages.last(), Some(&new));
    let edited = &open.messages[history.len() - 1];
    assert_eq!(edited.body, Some(MessageBody::new("edited while loading")));
}

#[test]
fn unread_marker_points_at_the_first_unread_message() {
    let state = loaded_state();
    let open = state.open.as_ref().unwrap();
    // Two unread: Carol's long message (Me's "On my way" is outgoing) and Alice's.
    let history = climbing_history();
    assert_eq!(open.unread_marker, Some(history[6].reference()));
    assert_eq!(history[6].sender, CAROL);
}

#[test]
fn messages_update_the_list_and_the_open_conversation() {
    let mut state = loaded_state();
    let to_alice = text(alice(), ALICE, fixtures::NOW_MS + 1, "hi");
    let effects = update(
        &mut state,
        AppEvent::Backend(BackendEvent::Message(to_alice.clone())),
    );
    assert_eq!(
        effects,
        vec![Effect::Persist(Box::new(BackendEvent::Message(to_alice)))]
    );
    let alice_summary = state.summary(alice()).unwrap();
    assert_eq!(alice_summary.unread, 2);
    // Most recent first after the pinned one.
    let order: Vec<_> = state.visible_conversations().map(|c| c.id).collect();
    assert_eq!(order, vec![bob(), alice(), climbing()]);

    let in_climbing = text(climbing(), BOB, fixtures::NOW_MS + 2, "here");
    let effects = update(
        &mut state,
        AppEvent::Backend(BackendEvent::Message(in_climbing.clone())),
    );
    assert_eq!(
        effects,
        vec![
            Effect::Persist(Box::new(BackendEvent::Message(in_climbing.clone()))),
            Effect::MarkRead(climbing()),
        ]
    );
    assert_eq!(state.summary(climbing()).unwrap().unread, 0);
    assert_eq!(
        state.open.as_ref().unwrap().messages.last(),
        Some(&in_climbing)
    );
}

#[test]
fn switching_conversations_keeps_drafts() {
    let mut state = loaded_state();
    typed(&mut state, "half a tho");
    update(&mut state, key(KeyCode::Backspace, KeyModifiers::NONE));
    assert_eq!(state.open.as_ref().unwrap().input, "half a th");

    let effects = update(&mut state, key(KeyCode::Char('j'), KeyModifiers::ALT));
    assert_eq!(open_id(&state), Some(alice()));
    assert_eq!(
        effects,
        vec![
            Effect::LoadMessages(alice()),
            Effect::RememberOpen(alice()),
            Effect::MarkRead(alice()),
        ]
    );
    assert_eq!(state.open.as_ref().unwrap().input, "");
    // Already at the end of the list; Alt+↓ is the same action.
    assert_eq!(
        update(&mut state, key(KeyCode::Down, KeyModifiers::ALT)),
        vec![]
    );

    update(&mut state, key(KeyCode::Char('k'), KeyModifiers::ALT));
    assert_eq!(open_id(&state), Some(climbing()));
    assert_eq!(state.open.as_ref().unwrap().input, "half a th");

    update(&mut state, key(KeyCode::Down, KeyModifiers::ALT));
    assert_eq!(open_id(&state), Some(alice()));
    update(&mut state, key(KeyCode::Up, KeyModifiers::ALT));
    assert_eq!(open_id(&state), Some(climbing()));
    // Alt+letters are commands, never text.
    assert_eq!(state.open.as_ref().unwrap().input, "half a th");
}

#[test]
fn shifted_characters_are_typed_and_releases_ignored() {
    let mut state = loaded_state();
    update(&mut state, key(KeyCode::Char('H'), KeyModifiers::SHIFT));
    let mut release = KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE);
    release.kind = KeyEventKind::Release;
    update(&mut state, AppEvent::Terminal(Event::Key(release)));
    typed(&mut state, "i 🧗");
    assert_eq!(state.open.as_ref().unwrap().input, "Hi 🧗");
    update(&mut state, key(KeyCode::Backspace, KeyModifiers::NONE));
    assert_eq!(state.open.as_ref().unwrap().input, "Hi ");
}

#[test]
fn ctrl_c_needs_confirmation() {
    let mut state = loaded_state();
    update(&mut state, key(KeyCode::Char('c'), KeyModifiers::CONTROL));
    assert!(!state.quit);
    assert!(state.notice.is_some());
    // Any other key cancels.
    typed(&mut state, "x");
    assert!(state.notice.is_none());
    update(&mut state, key(KeyCode::Char('c'), KeyModifiers::CONTROL));
    update(&mut state, key(KeyCode::Char('c'), KeyModifiers::CONTROL));
    assert!(state.quit);

    let mut state = loaded_state();
    update(&mut state, key(KeyCode::Char('q'), KeyModifiers::CONTROL));
    assert!(state.quit);
}

#[test]
fn sidebars_toggle() {
    let mut state = loaded_state();
    update(&mut state, key(KeyCode::Char('s'), KeyModifiers::ALT));
    update(&mut state, key(KeyCode::Char('i'), KeyModifiers::ALT));
    assert!(!state.panels.left);
    assert!(state.panels.right);
}

#[test]
fn typing_indicators_start_stop_and_expire() {
    let mut state = loaded_state();
    let typing = |sender, typing| {
        AppEvent::Backend(BackendEvent::Typing {
            conversation: climbing(),
            sender,
            state: typing,
        })
    };
    update(&mut state, typing(ALICE, TypingState::Started));
    update(&mut state, typing(BOB, TypingState::Started));
    assert_eq!(state.typists(climbing()).len(), 2);
    assert!(state.next_deadline().is_some());

    update(&mut state, typing(BOB, TypingState::Stopped));
    let from_alice = text(climbing(), ALICE, fixtures::NOW_MS + 5, "done typing");
    update(
        &mut state,
        AppEvent::Backend(BackendEvent::Message(from_alice)),
    );
    assert_eq!(state.typists(climbing()), &[]);

    update(&mut state, typing(CAROL, TypingState::Started));
    expire_typing(&mut state, Instant::now() + Duration::from_secs(16));
    assert_eq!(state.typists(climbing()), &[]);
    assert_eq!(state.next_deadline(), None);
}

#[test]
fn new_contacts_and_groups_update_titles() {
    let mut state = loaded_state();
    let mut contact = fixtures::contacts().remove(0);
    contact.name = Some("Alice Renamed".to_owned());
    update(
        &mut state,
        AppEvent::Backend(BackendEvent::ContactUpdated(contact)),
    );
    assert_eq!(state.conversation_title(alice()), "Alice Renamed");
}

#[test]
fn shutdown_and_store_failures() {
    let mut state = loaded_state();
    update(&mut state, AppEvent::StoreFailed("disk full".to_owned()));
    assert_eq!(
        state.notice.as_ref().map(|n| n.level),
        Some(NoticeLevel::Error)
    );
    update(&mut state, AppEvent::Shutdown);
    assert!(state.quit);
}

// --- M4 task 1: focus, sidebar navigation, scrolling, mouse ---

use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;

use crate::app::state::{Anchor, Focus, Geometry};

/// A frame's geometry: sidebar rows at y=1.., history at x=31.., input below.
fn geometry(state: &State) -> Geometry {
    Geometry {
        sidebar: Some(Rect::new(1, 1, 28, 20)),
        sidebar_rows: state.visible_conversations().map(|c| Some(c.id)).collect(),
        sidebar_offset: 0,
        history: Some(Rect::new(31, 1, 60, 12)),
        input: Some(Rect::new(31, 14, 60, 3)),
        scroll: state.open.as_ref().and_then(|o| o.scroll),
        at_top: false,
    }
}

fn mouse(kind: MouseEventKind, column: u16, row: u16) -> AppEvent {
    AppEvent::Terminal(Event::Mouse(MouseEvent {
        kind,
        column,
        row,
        modifiers: KeyModifiers::NONE,
    }))
}

fn scroll_of(state: &State) -> Option<Anchor> {
    state.open.as_ref().and_then(|o| o.scroll)
}

#[test]
fn tab_cycles_focus_and_skips_a_hidden_sidebar() {
    let mut state = loaded_state();
    state.geometry = geometry(&state);
    assert_eq!(state.focus, Focus::Input);
    let tab = || key(KeyCode::Tab, KeyModifiers::NONE);
    update(&mut state, tab());
    assert_eq!(state.focus, Focus::Conversations);
    update(&mut state, tab());
    assert_eq!(state.focus, Focus::Messages);
    update(&mut state, tab());
    assert_eq!(state.focus, Focus::Input);
    update(&mut state, key(KeyCode::BackTab, KeyModifiers::SHIFT));
    assert_eq!(state.focus, Focus::Messages);
    update(&mut state, key(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(state.focus, Focus::Input);

    state.geometry.sidebar = None;
    update(&mut state, tab());
    assert_eq!(state.focus, Focus::Messages);
    update(&mut state, tab());
    assert_eq!(state.focus, Focus::Input);
}

#[test]
fn hiding_the_focused_sidebar_moves_focus_to_the_input() {
    let mut state = loaded_state();
    state.focus = Focus::Conversations;
    update(&mut state, key(KeyCode::Char('s'), KeyModifiers::ALT));
    assert!(!state.panels.left);
    assert_eq!(state.focus, Focus::Input);
    // A frame without the sidebar (narrow terminal) does the same.
    state.focus = Focus::Conversations;
    update(&mut state, AppEvent::Drawn(Geometry::default()));
    assert_eq!(state.focus, Focus::Input);
}

#[test]
fn sidebar_keys_switch_conversations() {
    let mut state = loaded_state();
    state.focus = Focus::Conversations;
    // Order: Bob (pinned), Climbing (open), Alice.
    update(&mut state, key(KeyCode::Char('j'), KeyModifiers::NONE));
    assert_eq!(open_id(&state), Some(alice()));
    update(&mut state, key(KeyCode::Char('g'), KeyModifiers::NONE));
    assert_eq!(open_id(&state), Some(bob()));
    update(&mut state, key(KeyCode::Char('G'), KeyModifiers::SHIFT));
    assert_eq!(open_id(&state), Some(alice()));
    update(&mut state, key(KeyCode::Up, KeyModifiers::NONE));
    assert_eq!(open_id(&state), Some(climbing()));
    // Letters navigate here instead of typing.
    assert_eq!(state.open.as_ref().unwrap().input, "");
    update(&mut state, key(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(state.focus, Focus::Input);
}

#[test]
fn scrolling_pins_the_view_and_counts_new_messages() {
    let mut state = loaded_state();
    state.geometry = geometry(&state);
    let last = climbing_history().last().unwrap().reference();
    update(&mut state, key(KeyCode::PageUp, KeyModifiers::NONE));
    // A page is the history height minus two rows of context.
    assert_eq!(
        scroll_of(&state),
        Some(Anchor {
            message: last,
            offset: 10
        })
    );

    let news = text(climbing(), BOB, fixtures::NOW_MS, "Parking is full");
    update(&mut state, AppEvent::Backend(BackendEvent::Message(news)));
    let open = state.open.as_ref().unwrap();
    assert_eq!(open.unseen, 1);
    assert_eq!(open.scroll.map(|a| a.message), Some(last), "view stays put");

    state.focus = Focus::Messages;
    update(&mut state, key(KeyCode::Char('j'), KeyModifiers::NONE));
    assert_eq!(scroll_of(&state).map(|a| a.offset), Some(9));
    update(&mut state, key(KeyCode::Char('G'), KeyModifiers::SHIFT));
    let open = state.open.as_ref().unwrap();
    assert_eq!((open.scroll, open.unseen), (None, 0));
    // Scrolling down while at the bottom does nothing.
    update(&mut state, key(KeyCode::PageDown, KeyModifiers::NONE));
    assert_eq!(scroll_of(&state), None);
}

#[test]
fn reaching_the_top_loads_older_messages_once() {
    let mut state = loaded_state();
    let first = climbing_history().first().unwrap().reference();
    state.open.as_mut().unwrap().has_older = true;
    update(&mut state, key(KeyCode::PageUp, KeyModifiers::NONE));
    let mut drawn = geometry(&state);
    drawn.at_top = true;
    assert_eq!(
        update(&mut state, AppEvent::Drawn(drawn.clone())),
        vec![Effect::LoadOlder {
            conversation: climbing(),
            before: first,
        }]
    );
    assert_eq!(update(&mut state, AppEvent::Drawn(drawn)), vec![]);

    let older = text(climbing(), CAROL, 1_000, "Welcome to the group");
    update(
        &mut state,
        AppEvent::OlderLoaded {
            conversation: climbing(),
            messages: vec![older.clone()],
        },
    );
    let open = state.open.as_ref().unwrap();
    assert_eq!(open.messages.first(), Some(&older));
    assert_eq!(open.messages.len(), climbing_history().len() + 1);
    assert!(!open.loading_older);
    assert!(!open.has_older, "a short page is the last one");
}

#[test]
fn a_frame_at_the_bottom_clears_the_new_message_count() {
    let mut state = loaded_state();
    update(&mut state, key(KeyCode::PageUp, KeyModifiers::NONE));
    state.open.as_mut().unwrap().unseen = 3;
    let mut drawn = geometry(&state);
    drawn.scroll = None;
    update(&mut state, AppEvent::Drawn(drawn));
    let open = state.open.as_ref().unwrap();
    assert_eq!((open.scroll, open.unseen), (None, 0));
}

#[test]
fn mouse_clicks_and_wheel() {
    let mut state = loaded_state();
    state.geometry = geometry(&state);
    let left = MouseEventKind::Down(MouseButton::Left);

    // Third row: Alice.
    let effects = update(&mut state, mouse(left, 5, 3));
    assert_eq!(open_id(&state), Some(alice()));
    assert_eq!(effects.first(), Some(&Effect::LoadMessages(alice())));
    assert_eq!(state.focus, Focus::Input);

    update(&mut state, mouse(left, 40, 5));
    assert_eq!(state.focus, Focus::Messages);
    update(&mut state, mouse(left, 40, 15));
    assert_eq!(state.focus, Focus::Input);

    let mut state = loaded_state();
    state.geometry = geometry(&state);
    update(&mut state, mouse(MouseEventKind::ScrollUp, 40, 5));
    assert_eq!(scroll_of(&state).map(|a| a.offset), Some(3));
    update(&mut state, mouse(MouseEventKind::ScrollDown, 40, 5));
    assert_eq!(scroll_of(&state).map(|a| a.offset), Some(0));

    update(&mut state, mouse(MouseEventKind::ScrollDown, 5, 5));
    assert_eq!(state.sidebar.offset, 3);
    assert!(!state.sidebar.reveal);
    // Moving the mouse changes nothing and isn't worth a frame.
    let moved = MouseEvent {
        kind: MouseEventKind::Moved,
        column: 5,
        row: 5,
        modifiers: KeyModifiers::NONE,
    };
    assert!(!mouse_affects_view(moved));
}
