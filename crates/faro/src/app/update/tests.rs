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

    let effects = update(&mut state, key(KeyCode::Down, KeyModifiers::ALT));
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
    // Already at the end of the list.
    assert_eq!(
        update(&mut state, key(KeyCode::Down, KeyModifiers::ALT)),
        vec![]
    );

    update(&mut state, key(KeyCode::Up, KeyModifiers::ALT));
    assert_eq!(open_id(&state), Some(climbing()));
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
