//! Reducer tests: event in → state and effects out.

use std::time::Duration;

use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use faro_core::backend::{BackendError, BackendEvent, OutgoingMessage, ReactionChange};
use faro_core::domain::{DeliveryStatus, MessageBody, Quote, Timestamp, TypingState};

use super::*;
use crate::app::commands::Command;
use crate::app::state::{MessageAction, NoticeLevel, Popup};
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
fn startup_opens_nothing_and_highlights_the_remembered_conversation() {
    let mut state = State::new(ME, fixtures::contacts(), fixtures::groups());
    assert!(state.loading);
    let effects = update(
        &mut state,
        AppEvent::Loaded {
            conversations: fixtures::summaries(),
            last_open: Some(climbing()),
        },
    );
    // Nothing is opened, so nothing is read (D60).
    assert_eq!(effects, vec![]);
    assert!(!state.loading);
    assert_eq!(open_id(&state), None);
    assert_eq!(state.highlighted(), Some(climbing()));
    assert_eq!(state.focus, Focus::Conversations);
    assert_eq!(state.summary(climbing()).map(|s| s.unread), Some(2));
    // Enter opens it and reads it.
    let effects = update(&mut state, key(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(
        effects,
        vec![
            Effect::LoadMessages(climbing()),
            Effect::RememberOpen(climbing()),
            Effect::MarkRead(climbing()),
        ]
    );
    assert_eq!(open_id(&state), Some(climbing()));
    assert_eq!(state.focus, Focus::Input);
    assert_eq!(state.summary(climbing()).map(|s| s.unread), Some(0));
    // Pinned first, then by activity.
    let order: Vec<_> = state.visible_conversations().map(|c| c.id).collect();
    assert_eq!(order, vec![bob(), climbing(), alice()]);
}

#[test]
fn startup_highlights_the_first_conversation_without_a_remembered_one() {
    let mut state = State::new(ME, vec![], vec![]);
    update(
        &mut state,
        AppEvent::Loaded {
            conversations: fixtures::summaries(),
            last_open: None,
        },
    );
    assert_eq!(open_id(&state), None);
    assert_eq!(state.highlighted(), Some(bob()));
}

#[test]
fn a_first_message_on_a_fresh_start_opens_nothing() {
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
    // It's listed, highlighted and unread; it doesn't open by itself (D60).
    assert_eq!(open_id(&state), None);
    assert_eq!(state.highlighted(), Some(alice()));
    assert_eq!(state.summary(alice()).map(|s| s.unread), Some(1));
    assert_eq!(
        effects,
        vec![
            Effect::Persist(Box::new(BackendEvent::Message(first))),
            Effect::Bell,
            Effect::DesktopNotify {
                title: "faro".to_owned(),
                body: "New message from Alice 🌸".to_owned(),
            },
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
    update(&mut state, key(KeyCode::Enter, KeyModifiers::NONE));
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
            mentions: vec![],
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
        vec![
            Effect::Persist(Box::new(BackendEvent::Message(to_alice))),
            Effect::Bell,
            Effect::DesktopNotify {
                title: "faro".to_owned(),
                body: "New message from Alice 🌸".to_owned(),
            },
        ]
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
            Effect::Backend(Command::MarkRead(vec![in_climbing.reference()])),
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
    assert_eq!(state.open.as_ref().unwrap().input.text(), "half a th");

    let effects = update(&mut state, key(KeyCode::Char('j'), KeyModifiers::ALT));
    assert_eq!(open_id(&state), Some(alice()));
    // The draft is stored before switching.
    let saved = ConversationState {
        draft: Some(MessageBody::new("half a th")),
        ..ConversationState::default()
    };
    assert_eq!(
        effects,
        vec![
            Effect::Backend(Command::Typing {
                conversation: climbing(),
                state: TypingState::Stopped,
            }),
            Effect::SaveConversationState(climbing(), Box::new(saved)),
            Effect::LoadMessages(alice()),
            Effect::RememberOpen(alice()),
            Effect::MarkRead(alice()),
        ]
    );
    assert_eq!(state.open.as_ref().unwrap().input.text(), "");
    // Already at the end of the list; Alt+↓ is the same action.
    assert_eq!(
        update(&mut state, key(KeyCode::Down, KeyModifiers::ALT)),
        vec![]
    );

    update(&mut state, key(KeyCode::Char('k'), KeyModifiers::ALT));
    assert_eq!(open_id(&state), Some(climbing()));
    assert_eq!(state.open.as_ref().unwrap().input.text(), "half a th");

    update(&mut state, key(KeyCode::Down, KeyModifiers::ALT));
    assert_eq!(open_id(&state), Some(alice()));
    update(&mut state, key(KeyCode::Up, KeyModifiers::ALT));
    assert_eq!(open_id(&state), Some(climbing()));
    // Alt+letters are commands, never text.
    assert_eq!(state.open.as_ref().unwrap().input.text(), "half a th");
}

#[test]
fn shifted_characters_are_typed_and_releases_ignored() {
    let mut state = loaded_state();
    update(&mut state, key(KeyCode::Char('H'), KeyModifiers::SHIFT));
    let mut release = KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE);
    release.kind = KeyEventKind::Release;
    update(&mut state, AppEvent::Terminal(Event::Key(release)));
    typed(&mut state, "i 🧗");
    assert_eq!(state.open.as_ref().unwrap().input.text(), "Hi 🧗");
    update(&mut state, key(KeyCode::Backspace, KeyModifiers::NONE));
    assert_eq!(state.open.as_ref().unwrap().input.text(), "Hi ");
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
fn new_contacts_and_groups_are_listed_at_once() {
    let mut state = loaded_state();
    let dave = ServiceId::Aci(uuid::Uuid::from_u128(99));
    let effects = update(
        &mut state,
        AppEvent::Backend(BackendEvent::ContactUpdated(faro_core::domain::Contact {
            service_id: dave,
            name: Some("Dave".to_owned()),
            profile_name: None,
            phone: None,
        })),
    );
    let summary = state.summary(ConversationId::Direct(dave)).unwrap();
    assert_eq!(summary.title.as_deref(), Some("Dave"));
    assert!(matches!(effects.first(), Some(Effect::Persist(_))));
    // Sorted: never above conversations with messages.
    assert_eq!(
        state.conversations.last().map(|c| c.id),
        Some(ConversationId::Direct(dave))
    );
    let group = faro_core::domain::Group {
        id: faro_core::domain::GroupId::from_bytes([9; faro_core::domain::GroupId::LEN]),
        title: "Book club".to_owned(),
        members: vec![ME, dave],
    };
    update(
        &mut state,
        AppEvent::Backend(BackendEvent::GroupUpdated(group.clone())),
    );
    assert_eq!(
        state.conversation_title(ConversationId::Group(group.id)),
        "Book club"
    );
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

use crate::app::state::{Anchor, Focus, Geometry, SidebarRow};

/// A frame's geometry: sidebar rows at y=1.., history at x=31.., input below.
fn geometry(state: &State) -> Geometry {
    Geometry {
        sidebar: Some(Rect::new(1, 1, 28, 20)),
        sidebar_rows: state
            .visible_conversations()
            .map(|c| SidebarRow::Conversation(c.id))
            .collect(),
        sidebar_offset: 0,
        history: Some(Rect::new(31, 1, 60, 12)),
        input: Some(Rect::new(31, 14, 60, 3)),
        scroll: state.open.as_ref().and_then(|o| o.scroll),
        at_top: false,
        history_rows: vec![],
        palette: None,
        logo_animated: false,
        images_wanted: vec![],
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
    assert_eq!(state.open.as_ref().unwrap().input.text(), "");
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

    update(&mut state, mouse(MouseEventKind::ScrollDown, 40, 5));
    assert_eq!(
        scroll_of(&state).map(|a| a.offset),
        Some(10 - crate::app::keymap::WHEEL_ROWS)
    );
    // Back at the bottom (the renderer reports no anchor), the count clears.
    let mut drawn = geometry(&state);
    drawn.scroll = None;
    update(&mut state, AppEvent::Drawn(drawn));
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

// --- M4 task 2: editing and drafts ---

#[test]
fn editing_keys_change_the_input() {
    let mut state = loaded_state();
    typed(&mut state, "see you");
    update(&mut state, key(KeyCode::Enter, KeyModifiers::ALT));
    typed(&mut state, "there");
    update(&mut state, key(KeyCode::Char('w'), KeyModifiers::CONTROL));
    typed(&mut state, "soon");
    update(&mut state, key(KeyCode::Up, KeyModifiers::NONE));
    update(&mut state, key(KeyCode::Char('a'), KeyModifiers::CONTROL));
    typed(&mut state, "OK ");
    assert_eq!(
        state.open.as_ref().unwrap().input.text(),
        "OK see you\nsoon"
    );
}

#[test]
fn paste_keeps_newlines_and_focuses_the_input() {
    let mut state = loaded_state();
    state.focus = Focus::Messages;
    update(
        &mut state,
        AppEvent::Terminal(Event::Paste("line one\r\nline two".to_owned())),
    );
    assert_eq!(state.focus, Focus::Input);
    assert_eq!(
        state.open.as_ref().unwrap().input.text(),
        "line one\nline two"
    );
}

#[test]
fn drafts_are_saved_after_a_pause_and_on_quit_and_restored() {
    let mut state = loaded_state();
    // Moving the cursor isn't an edit.
    update(&mut state, key(KeyCode::Left, KeyModifiers::NONE));
    assert_eq!(state.next_deadline(), None);
    typed(&mut state, "draft");
    let due = state.next_deadline().unwrap();
    assert!(due > Instant::now());

    // Due: a Tick saves it once.
    state.open.as_mut().unwrap().draft_edited =
        Some(Instant::now().checked_sub(Duration::from_secs(5)).unwrap());
    let effects = update(&mut state, AppEvent::Tick);
    let [Effect::SaveConversationState(id, saved)] = effects.as_slice() else {
        panic!("expected a save, got {effects:?}");
    };
    assert_eq!(*id, climbing());
    assert_eq!(saved.draft.as_ref().map(MessageBody::expose), Some("draft"));
    assert_eq!(update(&mut state, AppEvent::Tick), vec![]);

    // Clearing it saves "no draft" on quit.
    for _ in 0.."draft".len() {
        update(&mut state, key(KeyCode::Backspace, KeyModifiers::NONE));
    }
    let effects = update(&mut state, key(KeyCode::Char('q'), KeyModifiers::CONTROL));
    assert!(state.quit);
    let [Effect::SaveConversationState(_, saved)] = effects.as_slice() else {
        panic!("expected a save, got {effects:?}");
    };
    assert_eq!(saved.draft, None);

    // A stored draft comes back when the conversation opens.
    let mut state = State::new(ME, fixtures::contacts(), fixtures::groups());
    let mut summaries = fixtures::summaries();
    for s in &mut summaries {
        if s.id == alice() {
            s.state.draft = Some(MessageBody::new("unfinished"));
        }
    }
    update(
        &mut state,
        AppEvent::Loaded {
            conversations: summaries,
            last_open: Some(alice()),
        },
    );
    update(&mut state, key(KeyCode::Enter, KeyModifiers::NONE));
    let input = &state.open.as_ref().unwrap().input;
    assert_eq!((input.text(), input.cursor()), ("unfinished", 10));
}

// --- M4 task 3: sending ---

fn last_message(state: &State) -> Message {
    state
        .open
        .as_ref()
        .unwrap()
        .messages
        .last()
        .cloned()
        .unwrap()
}

#[test]
fn enter_sends_shows_and_stores_the_message() {
    let mut state = loaded_state();
    typed(&mut state, "  on my way  ");
    update(&mut state, key(KeyCode::Enter, KeyModifiers::ALT));
    typed(&mut state, "see you");
    let effects = update(&mut state, key(KeyCode::Enter, KeyModifiers::NONE));

    let sent = last_message(&state);
    assert_eq!(sent.sender, ME);
    assert_eq!(sent.direction, Direction::Outgoing(DeliveryStatus::Sending));
    let body = "on my way  \nsee you";
    assert_eq!(sent.body.as_ref().map(MessageBody::expose), Some(body));
    assert_eq!(state.last_sent_at, sent.sent_at);
    assert!(state.open.as_ref().unwrap().input.is_empty());
    assert_eq!(
        state.summary(climbing()).and_then(|s| s.last_activity),
        Some(sent.sent_at)
    );
    assert_eq!(
        effects,
        vec![
            Effect::Persist(Box::new(BackendEvent::Message(sent.clone()))),
            Effect::Backend(Command::Send {
                conversation: climbing(),
                message: OutgoingMessage {
                    sent_at: sent.sent_at,
                    body: MessageBody::new(body),
                    mentions: vec![],
                    quote: None,
                    attachments: vec![],
                },
            }),
        ]
    );
}

#[test]
fn blank_input_sends_nothing() {
    let mut state = loaded_state();
    let before = state.open.as_ref().unwrap().messages.len();
    typed(&mut state, "   ");
    let effects = update(&mut state, key(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(effects, vec![]);
    assert_eq!(state.open.as_ref().unwrap().messages.len(), before);
}

#[test]
fn sending_clears_a_stored_draft_and_jumps_to_the_bottom() {
    let mut state = loaded_state();
    if let Some(summary) = summary_entry(&mut state, climbing()) {
        summary.state.draft = Some(MessageBody::new("old"));
    }
    update(&mut state, key(KeyCode::PageUp, KeyModifiers::NONE));
    typed(&mut state, "hi");
    let effects = update(&mut state, key(KeyCode::Enter, KeyModifiers::NONE));
    let Some(Effect::SaveConversationState(id, saved)) = effects.first() else {
        panic!("expected the cleared draft to be saved first: {effects:?}");
    };
    assert_eq!((*id, saved.draft.clone()), (climbing(), None));
    let open = state.open.as_ref().unwrap();
    assert_eq!(
        (open.scroll, open.unseen, open.draft_edited),
        (None, 0, None)
    );
}

#[test]
fn send_results_become_delivery_updates() {
    let mut state = loaded_state();
    typed(&mut state, "one");
    update(&mut state, key(KeyCode::Enter, KeyModifiers::NONE));
    let first = last_message(&state).sent_at;
    typed(&mut state, "two");
    update(&mut state, key(KeyCode::Enter, KeyModifiers::NONE));
    let second = last_message(&state).sent_at;
    assert!(second > first);

    let effects = update(
        &mut state,
        AppEvent::SendResult {
            sent_at: first,
            result: Ok(()),
        },
    );
    let sent = BackendEvent::DeliveryUpdate {
        message: MessageRef {
            sender: ME,
            sent_at: first,
        },
        status: DeliveryStatus::Sent,
        by: None,
        updated_at: None,
    };
    assert_eq!(effects, vec![Effect::Persist(Box::new(sent))]);
    update(
        &mut state,
        AppEvent::SendResult {
            sent_at: second,
            result: Err(BackendError::Network("offline".into())),
        },
    );
    let statuses: Vec<_> = state
        .open
        .as_ref()
        .unwrap()
        .messages
        .iter()
        .rev()
        .take(2)
        .map(|m| m.direction)
        .collect();
    assert_eq!(
        statuses,
        vec![
            Direction::Outgoing(DeliveryStatus::Failed),
            Direction::Outgoing(DeliveryStatus::Sent)
        ]
    );
    assert_eq!(
        state.notice.as_ref().map(|n| n.level),
        Some(NoticeLevel::Error)
    );
}

// --- M4 task 4: message selection ---

fn selected_ref(state: &State) -> Option<MessageRef> {
    state
        .open
        .as_ref()
        .and_then(|o| o.selected)
        .map(|s| s.message)
}

fn press(state: &mut State, c: char) -> Vec<Effect> {
    update(state, key(KeyCode::Char(c), KeyModifiers::NONE))
}

fn select_last_but(state: &mut State, older: usize) {
    update(state, key(KeyCode::Up, KeyModifiers::NONE));
    for _ in 0..older {
        press(state, 'k');
    }
}

#[test]
fn up_on_an_empty_input_selects_and_down_past_the_newest_returns() {
    let mut state = loaded_state();
    let history = climbing_history();
    let newest = history.last().unwrap().reference();
    update(&mut state, key(KeyCode::Up, KeyModifiers::NONE));
    assert_eq!(state.focus, Focus::Messages);
    assert_eq!(selected_ref(&state), Some(newest));
    assert!(state.open.as_ref().unwrap().selected.unwrap().reveal);
    press(&mut state, 'k');
    assert_eq!(
        selected_ref(&state),
        history.get(history.len() - 2).map(Message::reference)
    );
    press(&mut state, 'g');
    assert_eq!(
        selected_ref(&state),
        history.first().map(Message::reference)
    );
    press(&mut state, 'k');
    assert_eq!(
        selected_ref(&state),
        history.first().map(Message::reference),
        "stays at the top"
    );
    update(&mut state, key(KeyCode::Char('G'), KeyModifiers::SHIFT));
    press(&mut state, 'j');
    assert_eq!(state.focus, Focus::Input);

    // A frame shows the selection: no more revealing.
    update(&mut state, AppEvent::Drawn(Geometry::default()));
    assert!(!state.open.as_ref().unwrap().selected.unwrap().reveal);

    // With text in the input, ↑ stays in the input.
    typed(&mut state, "hi");
    update(&mut state, key(KeyCode::Up, KeyModifiers::NONE));
    assert_eq!(state.focus, Focus::Input);
}

#[test]
fn replies_quote_the_selected_message() {
    let mut state = loaded_state();
    let late = climbing_history().last().cloned().unwrap();
    update(&mut state, key(KeyCode::Up, KeyModifiers::NONE));
    press(&mut state, 'r');
    assert_eq!(state.focus, Focus::Input);
    let quote = Quote {
        target: late.reference(),
        preview: late.body.clone(),
    };
    assert_eq!(state.open.as_ref().unwrap().reply_to, Some(quote.clone()));
    // Esc cancels the reply first, then works as usual.
    update(&mut state, key(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(state.open.as_ref().unwrap().reply_to, None);

    update(&mut state, key(KeyCode::Up, KeyModifiers::NONE));
    press(&mut state, 'r');
    typed(&mut state, "no worries");
    let effects = update(&mut state, key(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(last_message(&state).quote, Some(quote.clone()));
    assert!(effects.iter().any(|e| matches!(
        e,
        Effect::Backend(Command::Send { message, .. }) if message.quote == Some(quote.clone())
    )));
    assert_eq!(state.open.as_ref().unwrap().reply_to, None);
}

#[test]
fn reactions_toggle_and_are_undone_on_failure() {
    let mut state = loaded_state();
    let target = climbing_history().last().unwrap().reference();
    let reaction = |emoji: Option<&str>| BackendEvent::Reaction {
        conversation: climbing(),
        target,
        sender: ME,
        emoji: emoji.map(str::to_owned),
    };
    update(&mut state, key(KeyCode::Up, KeyModifiers::NONE));
    press(&mut state, '+');
    assert_eq!(
        state.popup,
        Some(Popup::React {
            target,
            selected: 0,
            query: None,
        })
    );
    let effects = press(&mut state, '2');
    assert_eq!(state.popup, None);
    assert_eq!(
        effects,
        vec![
            Effect::Persist(Box::new(reaction(Some("👍")))),
            Effect::Backend(Command::React {
                conversation: climbing(),
                target,
                change: ReactionChange::Set("👍".into()),
                previous: None,
            }),
        ]
    );
    let mine = |state: &State| {
        let message = last_message(state);
        message
            .reactions
            .iter()
            .find(|r| r.sender == ME)
            .map(|r| r.emoji.clone())
    };
    assert_eq!(mine(&state).as_deref(), Some("👍"));

    // Picking it again takes it back, naming it (D73); the picker starts on it.
    press(&mut state, '+');
    assert_eq!(
        state.popup,
        Some(Popup::React {
            target,
            selected: 1,
            query: None,
        })
    );
    let effects = update(&mut state, key(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(mine(&state), None);
    assert!(effects.contains(&Effect::Backend(Command::React {
        conversation: climbing(),
        target,
        change: ReactionChange::Remove("👍".into()),
        previous: Some("👍".into()),
    })));

    // Another emoji replaces ours: a plain `Set`.
    press(&mut state, '+');
    press(&mut state, '2');
    press(&mut state, '+');
    let effects = press(&mut state, '3');
    assert!(effects.iter().any(|e| matches!(
        e,
        Effect::Backend(Command::React { change: ReactionChange::Set(_), previous: Some(p), .. })
            if p == "👍"
    )));

    // The backend refused: restore what was there before.
    update(
        &mut state,
        AppEvent::ReactResult {
            conversation: climbing(),
            target,
            previous: Some("👍".into()),
            result: Err(BackendError::Network("offline".into())),
        },
    );
    assert_eq!(mine(&state).as_deref(), Some("👍"));
    assert_eq!(
        state.notice.as_ref().map(|n| n.level),
        Some(NoticeLevel::Error)
    );
}

#[test]
fn copy_puts_the_text_on_the_clipboard_and_the_notice_expires() {
    let mut state = loaded_state();
    update(&mut state, key(KeyCode::Up, KeyModifiers::NONE));
    let effects = press(&mut state, 'y');
    assert_eq!(effects, vec![Effect::Copy("Running 10 min late 🙈".into())]);
    let notice = state.notice.clone().unwrap();
    assert_eq!(notice.level, NoticeLevel::Info);
    assert_eq!(state.next_deadline(), notice.until);
    state.notice.as_mut().unwrap().until = Some(Instant::now());
    update(&mut state, AppEvent::Tick);
    assert_eq!(state.notice, None);
}

#[test]
fn copies_and_replies_write_mentions_as_names() {
    let mut state = loaded_state();
    let mut message = text(climbing(), BOB, fixtures::NOW_MS, "hi \u{FFFC}!");
    message.mentions = vec![faro_core::domain::Mention {
        who: ALICE,
        start: 3,
        len: '\u{FFFC}'.len_utf8(),
    }];
    update(
        &mut state,
        AppEvent::Backend(BackendEvent::Message(message)),
    );
    update(&mut state, key(KeyCode::Up, KeyModifiers::NONE));
    let readable = format!("hi {}!", state.mention_name(ALICE));
    assert_eq!(press(&mut state, 'y'), vec![Effect::Copy(readable.clone())]);
    press(&mut state, 'r');
    let quote = state.open.as_ref().unwrap().reply_to.clone().unwrap();
    assert_eq!(quote.preview, Some(MessageBody::new(readable)));
}

#[test]
fn links_open_locally_and_are_copied_over_ssh() {
    let mut state = loaded_state();
    let one = text(
        climbing(),
        BOB,
        fixtures::NOW_MS,
        "map: https://example.com/map.",
    );
    update(&mut state, AppEvent::Backend(BackendEvent::Message(one)));
    update(&mut state, key(KeyCode::Up, KeyModifiers::NONE));
    assert_eq!(
        press(&mut state, 'o'),
        vec![Effect::OpenUrl("https://example.com/map".into())]
    );
    state.env.over_ssh = true;
    assert_eq!(
        press(&mut state, 'o'),
        vec![Effect::Copy("https://example.com/map".into())]
    );

    // Several links: choose from a menu.
    let two = text(
        climbing(),
        BOB,
        fixtures::NOW_MS + 1,
        "https://a.org and https://b.org",
    );
    update(
        &mut state,
        AppEvent::Backend(BackendEvent::Message(two.clone())),
    );
    update(&mut state, key(KeyCode::Char('G'), KeyModifiers::SHIFT));
    press(&mut state, 'o');
    let Some(Popup::Actions { items, .. }) = &state.popup else {
        panic!("expected a link menu");
    };
    assert_eq!(
        items,
        &vec![
            MessageAction::OpenLink("https://a.org".into()),
            MessageAction::OpenLink("https://b.org".into())
        ]
    );
    press(&mut state, 'j');
    assert_eq!(
        update(&mut state, key(KeyCode::Enter, KeyModifiers::NONE)),
        vec![Effect::Copy("https://b.org".into())]
    );
    assert_eq!(state.popup, None);
}

#[test]
fn the_menu_lists_what_a_message_offers() {
    let mut state = loaded_state();
    // "On my way" (own, sent), then the deleted message further up.
    select_last_but(&mut state, 1);
    update(&mut state, key(KeyCode::Enter, KeyModifiers::NONE));
    let Some(Popup::Actions {
        items, selected, ..
    }) = &state.popup
    else {
        panic!("expected the action menu");
    };
    assert_eq!(*selected, 0);
    assert_eq!(
        items,
        &vec![
            MessageAction::Reply,
            MessageAction::React,
            MessageAction::Copy,
            MessageAction::Forward,
            MessageAction::Info,
            MessageAction::Edit,
            MessageAction::Delete,
        ]
    );
    // Shortcut letters work inside the menu; Esc closes it.
    press(&mut state, '+');
    assert!(matches!(state.popup, Some(Popup::React { .. })));
    update(&mut state, key(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(state.popup, None);
    assert_eq!(state.focus, Focus::Messages);

    // Deleted messages offer nothing, so no menu.
    for _ in 0..3 {
        press(&mut state, 'k');
    }
    let deleted = climbing_history().get(4).map(Message::reference);
    assert_eq!(selected_ref(&state), deleted);
    update(&mut state, key(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(state.popup, None);
}

#[test]
fn failed_messages_can_be_retried_from_the_menu() {
    let mut state = loaded_state();
    typed(&mut state, "lost");
    update(&mut state, key(KeyCode::Enter, KeyModifiers::NONE));
    let sent = last_message(&state);
    update(
        &mut state,
        AppEvent::SendResult {
            sent_at: sent.sent_at,
            result: Err(BackendError::Network("offline".into())),
        },
    );
    update(&mut state, key(KeyCode::Up, KeyModifiers::NONE));
    update(&mut state, key(KeyCode::Enter, KeyModifiers::NONE));
    let effects = update(&mut state, key(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(
        effects,
        vec![
            Effect::Persist(Box::new(BackendEvent::DeliveryUpdate {
                message: sent.reference(),
                status: DeliveryStatus::Sending,
                by: None,
                updated_at: None,
            })),
            Effect::Backend(Command::Send {
                conversation: climbing(),
                message: OutgoingMessage {
                    sent_at: sent.sent_at,
                    body: MessageBody::new("lost"),
                    mentions: vec![],
                    quote: None,
                    attachments: vec![],
                },
            }),
        ]
    );
    assert_eq!(
        last_message(&state).direction,
        Direction::Outgoing(DeliveryStatus::Sending)
    );
}

#[test]
fn clicking_a_message_selects_it_and_right_click_opens_the_menu() {
    let mut state = loaded_state();
    let target = climbing_history().last().unwrap().reference();
    let mut drawn = geometry(&state);
    drawn.history_rows = vec![None, Some(target)];
    update(&mut state, AppEvent::Drawn(drawn));
    update(
        &mut state,
        mouse(MouseEventKind::Down(MouseButton::Left), 40, 2),
    );
    assert_eq!(state.focus, Focus::Messages);
    assert_eq!(selected_ref(&state), Some(target));
    update(
        &mut state,
        mouse(MouseEventKind::Down(MouseButton::Right), 40, 2),
    );
    assert!(matches!(state.popup, Some(Popup::Actions { .. })));
    // Any click closes it.
    update(
        &mut state,
        mouse(MouseEventKind::Down(MouseButton::Left), 40, 9),
    );
    assert_eq!(state.popup, None);
}

#[test]
fn auto_downloads_respect_policy_size_and_concurrency_cap() {
    use faro_core::config::AutoDownload;
    use faro_core::domain::{AttachmentRef, AttachmentStatus};
    use std::path::PathBuf;

    let mut state = loaded_state();
    // Default is AutoDownload::Images: remote images under 25 MB are queued,
    // non-images and >25 MB images are left Remote.
    let mut msg = text(climbing(), BOB, fixtures::NOW_MS + 10, "files");
    msg.attachments = vec![
        AttachmentRef {
            file_name: Some("small.jpg".into()),
            size: Some(1024),
            width: Some(100),
            height: Some(80),
            ..AttachmentRef::remote("att-1", "image/jpeg")
        },
        AttachmentRef {
            file_name: Some("doc.pdf".into()),
            size: Some(1024),
            ..AttachmentRef::remote("att-2", "application/pdf")
        },
        AttachmentRef {
            file_name: Some("huge.png".into()),
            size: Some(30 * 1024 * 1024),
            width: Some(4000),
            height: Some(3000),
            ..AttachmentRef::remote("att-3", "image/png")
        },
        AttachmentRef {
            file_name: Some("voice-1.wav".into()),
            size: Some(2048),
            voice_note: true,
            ..AttachmentRef::remote("att-vn", "audio/wav")
        },
    ];
    let target = msg.reference();
    let effects = update(
        &mut state,
        AppEvent::Backend(BackendEvent::Message(msg.clone())),
    );
    assert!(effects.contains(&Effect::Backend(Command::Fetch {
        conversation: climbing(),
        message: target,
        index: 0,
        attachment: msg.attachments[0].clone(),
    })));
    assert!(state.is_downloading(target, 0));
    assert!(!state.is_downloading(target, 1));
    assert!(!state.is_downloading(target, 2));
    assert!(state.is_downloading(target, 3));

    // Completing the download updates state and emits SaveAttachment.
    let local_path = PathBuf::from("/tmp/faro-test/small.jpg");
    let fetched = AttachmentRef {
        status: AttachmentStatus::Downloaded(local_path.clone()),
        ..msg.attachments[0].clone()
    };
    let effects = update(
        &mut state,
        AppEvent::Fetched {
            conversation: climbing(),
            message: target,
            index: 0,
            result: Ok(fetched.clone()),
        },
    );
    assert!(!state.is_downloading(target, 0));
    assert!(effects.contains(&Effect::SaveAttachment {
        message: target,
        index: 0,
        attachment: fetched,
    }));

    // AutoDownload::Off queues nothing.
    state.config.media.auto_download = AutoDownload::Off;
    let mut msg2 = text(climbing(), BOB, fixtures::NOW_MS + 11, "more");
    msg2.attachments = vec![AttachmentRef {
        file_name: Some("off.jpg".into()),
        size: Some(512),
        ..AttachmentRef::remote("att-4", "image/jpeg")
    }];
    let effects = update(&mut state, AppEvent::Backend(BackendEvent::Message(msg2)));
    assert!(
        !effects
            .iter()
            .any(|e| matches!(e, Effect::Backend(Command::Fetch { .. })))
    );
}

#[test]
fn attachment_keys_download_view_open_and_save() {
    use faro_core::config::AutoDownload;
    use faro_core::domain::{AttachmentRef, AttachmentStatus};
    use std::path::PathBuf;

    let mut state = loaded_state();
    state.config.media.auto_download = AutoDownload::Off;
    let mut msg = text(climbing(), BOB, fixtures::NOW_MS + 20, "photo");
    msg.attachments = vec![AttachmentRef {
        file_name: Some("wall.png".into()),
        size: Some(2048),
        width: Some(640),
        height: Some(480),
        ..AttachmentRef::remote("att-wall", "image/png")
    }];
    let target = msg.reference();
    update(
        &mut state,
        AppEvent::Backend(BackendEvent::Message(msg.clone())),
    );
    // Select the newest message (`Up` from Input enters Messages on the last message).
    update(&mut state, key(KeyCode::Up, KeyModifiers::NONE));
    assert_eq!(selected_ref(&state), Some(target));

    // Pressing `s` on a remote attachment queues a download and remembers to save on completion.
    let effects = press(&mut state, 's');
    assert_eq!(effects.len(), 1);
    assert!(matches!(
        &effects[0],
        Effect::Backend(Command::Fetch { index: 0, .. })
    ));

    let local_path = PathBuf::from("/tmp/faro-test/wall.png");
    let fetched = AttachmentRef {
        status: AttachmentStatus::Downloaded(local_path.clone()),
        ..msg.attachments[0].clone()
    };
    let effects = update(
        &mut state,
        AppEvent::Fetched {
            conversation: climbing(),
            message: target,
            index: 0,
            result: Ok(fetched),
        },
    );
    assert!(effects.contains(&Effect::SaveToDownloads {
        source: local_path.clone(),
        file_name: Some("wall.png".into()),
        content_type: "image/png".into(),
    }));

    // Pressing `v` on a downloaded image opens the inline ImageViewer popup.
    let effects = press(&mut state, 'v');
    assert_eq!(effects, vec![]);
    assert_eq!(state.popup, Some(Popup::ImageViewer { target, index: 0 }));
    // Inside ImageViewer, `y` copies the local path and `Esc` closes.
    assert_eq!(
        press(&mut state, 'y'),
        vec![Effect::Copy(local_path.display().to_string())]
    );
    update(&mut state, key(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(state.popup, None);

    // For a non-image attachment, `v` opens locally with OpenFile, or saves + copies path over SSH.
    let pdf_path = PathBuf::from("/tmp/faro-test/guide.pdf");
    let mut pdf_msg = text(climbing(), BOB, fixtures::NOW_MS + 21, "guide");
    pdf_msg.attachments = vec![AttachmentRef {
        file_name: Some("guide.pdf".into()),
        size: Some(4096),
        status: AttachmentStatus::Downloaded(pdf_path.clone()),
        ..AttachmentRef::remote("att-pdf", "application/pdf")
    }];
    update(
        &mut state,
        AppEvent::Backend(BackendEvent::Message(pdf_msg)),
    );
    update(&mut state, key(KeyCode::Char('G'), KeyModifiers::SHIFT));
    assert_eq!(
        press(&mut state, 'v'),
        vec![Effect::OpenFile(pdf_path.clone())]
    );
    state.env.over_ssh = true;
    assert_eq!(
        press(&mut state, 'v'),
        vec![
            Effect::Copy(pdf_path.display().to_string()),
            Effect::SaveToDownloads {
                source: pdf_path,
                file_name: Some("guide.pdf".into()),
                content_type: "application/pdf".into(),
            }
        ]
    );
}

#[test]
fn staging_sending_and_pasting_attachments() {
    use crate::app::state::StagedAttachment;
    use faro_core::backend::OutgoingAttachment;
    use faro_core::domain::AttachmentStatus;
    use std::path::PathBuf;

    let mut state = loaded_state();
    state.attachments_dir = PathBuf::from("/tmp/faro-test/attachments");

    // Staging an oversized file shows an error notice without staging.
    update(
        &mut state,
        AppEvent::FileStaged(Err("file exceeds 100 MB limit (120.0 MB)".into())),
    );
    assert!(
        state
            .notice
            .as_ref()
            .is_some_and(|n| n.text.contains("exceeds 100 MB limit"))
    );
    assert!(state.open.as_ref().unwrap().staged_attachment.is_none());

    // Staging a valid file attaches it to the open conversation and focuses Input.
    let staged = StagedAttachment {
        path: PathBuf::from("/home/user/Downloads/photo.png"),
        name: "photo.png".into(),
        content_type: "image/png".into(),
        size: 16_384,
        width: Some(800),
        height: Some(600),
    };
    update(&mut state, AppEvent::FileStaged(Ok(staged.clone())));
    assert_eq!(
        state.open.as_ref().unwrap().staged_attachment,
        Some(staged.clone())
    );
    assert_eq!(state.focus, Focus::Input);

    // Esc in Input removes the staged attachment before closing the conversation.
    update(&mut state, key(KeyCode::Esc, KeyModifiers::NONE));
    assert!(state.open.as_ref().unwrap().staged_attachment.is_none());
    assert!(state.open.is_some());

    // Pasting a single path checks it off-thread; if it's a file, it stages it.
    let effects = update(
        &mut state,
        AppEvent::Terminal(Event::Paste("'/home/user/Downloads/photo.png'\n".into())),
    );
    assert_eq!(
        effects,
        vec![Effect::CheckPaste {
            raw: "'/home/user/Downloads/photo.png'\n".into(),
            path: PathBuf::from("/home/user/Downloads/photo.png"),
        }]
    );
    assert_eq!(state.open.as_ref().unwrap().input.text(), "");

    // When PasteChecked reports None (not a regular file), the raw text is inserted.
    update(
        &mut state,
        AppEvent::PasteChecked {
            raw: "/not/a/real/file".into(),
            staged: None,
        },
    );
    assert_eq!(
        state.open.as_ref().unwrap().input.text(),
        "/not/a/real/file"
    );
    state.open.as_mut().unwrap().input = crate::app::input::Input::default();

    // When PasteChecked reports Some(staged), the file is staged and input stays empty.
    update(
        &mut state,
        AppEvent::PasteChecked {
            raw: "/home/user/Downloads/photo.png".into(),
            staged: Some(staged.clone()),
        },
    );
    assert_eq!(
        state.open.as_ref().unwrap().staged_attachment,
        Some(staged.clone())
    );
    assert_eq!(state.open.as_ref().unwrap().input.text(), "");

    // Sending with no text sends the attachment with body = None on the local echo.
    let effects = update(&mut state, key(KeyCode::Enter, KeyModifiers::NONE));
    assert!(state.open.as_ref().unwrap().staged_attachment.is_none());
    let last_msg = state.open.as_ref().unwrap().messages.last().unwrap();
    assert_eq!(last_msg.body, None);
    assert_eq!(last_msg.attachments.len(), 1);
    assert_eq!(
        last_msg.attachments[0].file_name.as_deref(),
        Some("photo.png")
    );
    assert!(matches!(
        &last_msg.attachments[0].status,
        AttachmentStatus::Downloaded(p) if p.starts_with("/tmp/faro-test/attachments")
    ));
    assert!(effects.iter().any(|e| matches!(
        e,
        Effect::Backend(Command::Send { message, .. })
            if message.body.expose().is_empty()
                && message.attachments == vec![OutgoingAttachment {
                    path: PathBuf::from("/home/user/Downloads/photo.png"),
                    content_type: "image/png".into(),
                    file_name: Some("photo.png".into()),
                    size: 16_384,
                    width: Some(800),
                    height: Some(600),
                }]
    )));
}

#[test]
fn voice_note_playback_and_remote_session_guard() {
    use faro_core::config::AutoDownload;
    use faro_core::domain::{AttachmentRef, AttachmentStatus};
    use std::path::PathBuf;

    let mut state = loaded_state();
    state.config.media.auto_download = AutoDownload::Off;
    state.config.voice.player = "mpv".into();
    let voice_path = PathBuf::from("/tmp/faro-test/voice.wav");
    let mut msg = text(climbing(), BOB, fixtures::NOW_MS + 30, "");
    msg.body = None;
    msg.attachments = vec![AttachmentRef {
        file_name: Some("voice.wav".into()),
        size: Some(16_000),
        status: AttachmentStatus::Downloaded(voice_path.clone()),
        voice_note: true,
        duration_secs: Some(8),
        waveform: Some(vec![1, 2, 3, 4]),
        transcript: Some("Heading to the boulders".into()),
        transcript_source: Some(faro_core::domain::TranscriptSource::Local),
        ..AttachmentRef::remote("att-vn", "audio/wav")
    }];
    let target = msg.reference();
    update(&mut state, AppEvent::Backend(BackendEvent::Message(msg)));
    update(&mut state, key(KeyCode::Up, KeyModifiers::NONE));
    assert_eq!(selected_ref(&state), Some(target));

    // Copying a voice note with a transcript copies the transcript text.
    assert_eq!(
        press(&mut state, 'y'),
        vec![Effect::Copy("Heading to the boulders".into())]
    );

    // Local session: `x` starts playback; pressing `x` again stops it.
    let effects = press(&mut state, 'x');
    assert_eq!(
        effects,
        vec![Effect::PlayAudio {
            message: target,
            index: 0,
            path: voice_path.clone(),
            player: "mpv".into(),
        }]
    );
    assert_eq!(state.playing_voice_note(target, 0), Some(0));

    // Message menu while playing offers `StopVoiceNote`.
    update(&mut state, key(KeyCode::Enter, KeyModifiers::NONE));
    assert!(matches!(
        &state.popup,
        Some(Popup::Actions { items, .. })
            if items.contains(&MessageAction::StopVoiceNote { index: 0 })
    ));
    update(&mut state, key(KeyCode::Esc, KeyModifiers::NONE));

    let effects = press(&mut state, 'x');
    assert_eq!(effects, vec![Effect::StopAudio]);
    assert!(state.playing.is_none());

    // Starting again via `v`, then finishing via PlaybackFinished clears `playing`.
    let effects = press(&mut state, 'v');
    assert_eq!(
        effects,
        vec![Effect::PlayAudio {
            message: target,
            index: 0,
            path: voice_path.clone(),
            player: "mpv".into(),
        }]
    );
    update(
        &mut state,
        AppEvent::PlaybackFinished {
            message: target,
            index: 0,
            error: Some("mpv exited with 1".into()),
        },
    );
    assert!(state.playing.is_none());
    assert!(
        state
            .notice
            .as_ref()
            .is_some_and(|n| n.text.contains("Couldn't play voice note"))
    );

    // Remote session (mosh or SSH): PlayVoiceNote is omitted from the menu, and `x` saves + copies path.
    state.env.emoji.mosh = true;
    assert!(!state.can_play_audio());
    update(&mut state, key(KeyCode::Enter, KeyModifiers::NONE));
    assert!(matches!(
        &state.popup,
        Some(Popup::Actions { items, .. })
            if !items.iter().any(|a| matches!(
                a,
                MessageAction::PlayVoiceNote { .. } | MessageAction::StopVoiceNote { .. }
            ))
    ));
    update(&mut state, key(KeyCode::Esc, KeyModifiers::NONE));

    assert_eq!(
        press(&mut state, 'x'),
        vec![
            Effect::Copy(voice_path.display().to_string()),
            Effect::SaveToDownloads {
                source: voice_path,
                file_name: Some("voice.wav".into()),
                content_type: "audio/wav".into(),
            }
        ]
    );
}

#[test]
fn voice_note_local_transcription() {
    use faro_core::config::{AutoDownload, TranscribeMode};
    use faro_core::domain::{AttachmentRef, AttachmentStatus, TranscriptSource};
    use std::path::PathBuf;

    let mut state = loaded_state();
    state.config.media.auto_download = AutoDownload::Off;
    let voice_path = PathBuf::from("/tmp/faro-test/voice-t.wav");
    let mut msg = text(climbing(), BOB, fixtures::NOW_MS + 40, "");
    msg.body = None;
    msg.attachments = vec![AttachmentRef {
        file_name: Some("voice-t.wav".into()),
        size: Some(12_000),
        status: AttachmentStatus::Downloaded(voice_path.clone()),
        voice_note: true,
        duration_secs: Some(6),
        waveform: Some(vec![2, 4, 6, 4]),
        ..AttachmentRef::remote("att-vnt", "audio/wav")
    }];
    let target = msg.reference();
    update(&mut state, AppEvent::Backend(BackendEvent::Message(msg)));
    update(&mut state, key(KeyCode::Up, KeyModifiers::NONE));
    assert_eq!(selected_ref(&state), Some(target));

    let effects = press(&mut state, 't');
    assert!(state.is_transcribing(target, 0));
    assert!(effects.iter().any(|e| matches!(
        e,
        Effect::Backend(Command::Transcribe {
            conversation,
            message,
            index: 0,
            path,
            voice,
        }) if *conversation == climbing()
            && *message == target
            && *path == voice_path
            && voice.transcribe == TranscribeMode::Local
    )));

    let effects = update(
        &mut state,
        AppEvent::Transcribed {
            conversation: climbing(),
            message: target,
            index: 0,
            result: Ok(("Meet at the crag at five".into(), TranscriptSource::Local)),
        },
    );
    assert!(!state.is_transcribing(target, 0));
    assert!(effects.iter().any(|e| matches!(
        e,
        Effect::SaveAttachment {
            message,
            index: 0,
            attachment,
        } if *message == target
            && attachment.transcript.as_deref() == Some("Meet at the crag at five")
            && attachment.transcript_source == Some(TranscriptSource::Local)
    )));
}

#[test]
fn voice_note_remote_consent_flow() {
    use faro_core::config::{AutoDownload, TranscribeMode};
    use faro_core::domain::{AttachmentRef, AttachmentStatus};
    use faro_core::store::RemoteTranscription;
    use std::path::PathBuf;

    let mut state = loaded_state();
    state.config.media.auto_download = AutoDownload::Off;
    state.config.voice.transcribe = TranscribeMode::Remote;
    let voice_path = PathBuf::from("/tmp/faro-test/voice-r.wav");
    let mut msg = text(climbing(), BOB, fixtures::NOW_MS + 50, "");
    msg.body = None;
    msg.attachments = vec![AttachmentRef {
        file_name: Some("voice-r.wav".into()),
        size: Some(10_000),
        status: AttachmentStatus::Downloaded(voice_path),
        voice_note: true,
        duration_secs: Some(5),
        ..AttachmentRef::remote("att-vnr", "audio/wav")
    }];
    let target = msg.reference();
    let effects = update(&mut state, AppEvent::Backend(BackendEvent::Message(msg)));
    assert!(
        !effects
            .iter()
            .any(|e| matches!(e, Effect::Backend(Command::Transcribe { .. }))),
        "remote mode must not auto-transcribe when conversation is Disallowed"
    );
    update(&mut state, key(KeyCode::Up, KeyModifiers::NONE));
    assert_eq!(selected_ref(&state), Some(target));

    let effects = press(&mut state, 't');
    assert_eq!(effects, vec![]);
    assert!(matches!(
        state.popup,
        Some(Popup::ConfirmRemoteTranscription {
            conversation,
            target: t,
            index: 0,
            selected: 0,
        }) if conversation == climbing() && t == target
    ));

    let effects = press(&mut state, '1');
    assert_eq!(
        state
            .summary(climbing())
            .unwrap()
            .state
            .remote_transcription,
        RemoteTranscription::Allowed
    );
    assert!(effects.iter().any(|e| matches!(
        e,
        Effect::SaveConversationState(id, st)
            if *id == climbing() && st.remote_transcription == RemoteTranscription::Allowed
    )));
    assert!(effects.iter().any(|e| matches!(
        e,
        Effect::Backend(Command::Transcribe {
            message,
            voice,
            ..
        }) if *message == target && voice.transcribe == TranscribeMode::Remote
    )));
}

#[test]
fn editing_own_message_updates_timeline_and_sends_edit_command() {
    let mut state = loaded_state();
    // Newest message is Bob's incoming message: cannot be edited.
    update(&mut state, key(KeyCode::Up, KeyModifiers::NONE));
    assert_eq!(press(&mut state, 'e'), vec![]);
    assert_eq!(state.focus, Focus::Messages);
    assert!(
        state
            .notice
            .as_ref()
            .is_some_and(|n| n.text.contains("Only your own messages"))
    );

    // Step up to our own message ("On my way") and press `e`.
    press(&mut state, 'k');
    let own_ref = selected_ref(&state).unwrap();
    assert_eq!(own_ref.sender, ME);
    press(&mut state, 'e');
    assert_eq!(state.focus, Focus::Input);
    assert_eq!(state.open.as_ref().unwrap().editing, Some(own_ref.sent_at));
    assert_eq!(state.open.as_ref().unwrap().input.text(), "On my way");

    // Esc cancels the edit without sending or closing the conversation.
    update(&mut state, key(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(state.open.as_ref().unwrap().editing, None);
    assert_eq!(state.open.as_ref().unwrap().input.text(), "");
    assert!(state.open.is_some());

    // Start editing again, append text, and press Enter.
    select_last_but(&mut state, 1);
    press(&mut state, 'e');
    typed(&mut state, " — 5 min out");
    let effects = update(&mut state, key(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(state.open.as_ref().unwrap().editing, None);
    assert_eq!(state.open.as_ref().unwrap().input.text(), "");
    let edited = state
        .open
        .as_ref()
        .unwrap()
        .messages
        .iter()
        .find(|m| m.reference() == own_ref)
        .unwrap();
    assert_eq!(
        edited.body.as_ref().map(MessageBody::expose),
        Some("On my way — 5 min out")
    );
    assert!(edited.edited_at.is_some());
    assert!(effects.iter().any(|e| matches!(
        e,
        Effect::Persist(ev) if matches!(
            ev.as_ref(),
            BackendEvent::Edited { conversation, target, body, .. }
                if *conversation == climbing()
                    && *target == own_ref
                    && body.expose() == "On my way — 5 min out"
        )
    )));
    assert!(effects.iter().any(|e| matches!(
        e,
        Effect::Backend(Command::Edit { conversation, target, edit })
            if *conversation == climbing()
                && *target == own_ref
                && edit.body.expose() == "On my way — 5 min out"
    )));
}

#[test]
fn deleting_own_message_for_everyone_confirms_and_sends_delete_command() {
    let mut state = loaded_state();
    // Incoming message cannot be deleted for everyone.
    update(&mut state, key(KeyCode::Up, KeyModifiers::NONE));
    assert_eq!(
        update(&mut state, key(KeyCode::Char('D'), KeyModifiers::SHIFT)),
        vec![]
    );
    assert_eq!(state.popup, None);

    // Select our own message ("On my way") and press `D`.
    press(&mut state, 'k');
    let own_ref = selected_ref(&state).unwrap();
    update(&mut state, key(KeyCode::Char('D'), KeyModifiers::SHIFT));
    assert_eq!(state.popup, Some(Popup::ConfirmDelete { target: own_ref }));

    // Pressing `n` cancels without deleting.
    assert_eq!(press(&mut state, 'n'), vec![]);
    assert_eq!(state.popup, None);
    let msg = state
        .open
        .as_ref()
        .unwrap()
        .messages
        .iter()
        .find(|m| m.reference() == own_ref)
        .unwrap();
    assert!(!msg.deleted);

    // Pressing `D` then `y` confirms deletion.
    update(&mut state, key(KeyCode::Char('D'), KeyModifiers::SHIFT));
    let effects = press(&mut state, 'y');
    assert_eq!(state.popup, None);
    let msg = state
        .open
        .as_ref()
        .unwrap()
        .messages
        .iter()
        .find(|m| m.reference() == own_ref)
        .unwrap();
    assert!(msg.deleted);
    assert_eq!(
        effects,
        vec![
            Effect::Persist(Box::new(BackendEvent::Deleted {
                conversation: climbing(),
                target: own_ref,
            })),
            Effect::Backend(Command::DeleteForEveryone {
                conversation: climbing(),
                target: own_ref,
            }),
        ]
    );
}

#[test]
fn typing_at_opens_mention_autocomplete_and_sends_mentions() {
    use faro_core::domain::Mention;

    let mut state = loaded_state();
    assert_eq!(state.mention_candidates(), vec![]);

    // Type "cc @al" in the Climbing group chat: Alice matches.
    typed(&mut state, "cc @al");
    let candidates = state.mention_candidates();
    assert_eq!(candidates, vec![(ALICE, "Alice 🌸".to_owned())]);

    // Pressing Tab accepts the highlighted candidate and inserts "@Alice 🌸 ".
    update(&mut state, key(KeyCode::Tab, KeyModifiers::NONE));
    assert_eq!(state.open.as_ref().unwrap().input.text(), "cc @Alice 🌸 ");
    assert_eq!(state.mention_candidates(), vec![]);

    // Add more text and send.
    typed(&mut state, "for beta");
    let effects = update(&mut state, key(KeyCode::Enter, KeyModifiers::NONE));
    let sent = last_message(&state);
    assert_eq!(
        sent.body.as_ref().map(MessageBody::expose),
        Some("cc \u{FFFC} for beta")
    );
    let expected_mentions = vec![Mention {
        who: ALICE,
        start: 3,
        len: '\u{FFFC}'.len_utf8(),
    }];
    assert_eq!(sent.mentions, expected_mentions);
    assert_eq!(
        state.readable_body(&sent).as_deref(),
        Some("cc @Alice 🌸 for beta")
    );
    assert!(effects.iter().any(|e| matches!(
        e,
        Effect::Backend(Command::Send { message, .. })
            if message.body.expose() == "cc \u{FFFC} for beta"
                && message.mentions == expected_mentions
    )));
}

#[test]
fn message_info_popup_loads_and_updates_receipts() {
    use faro_core::store::RecipientReceipt;

    let mut state = loaded_state();
    // Select the last outgoing message in climbing_history (index 7, `sent`, from ME).
    select_last_but(&mut state, 1);
    let target = selected_ref(&state).unwrap();
    assert_eq!(target.sender, ME);

    let effects = press(&mut state, 'i');
    assert_eq!(
        effects,
        vec![Effect::LoadMessageReceipts {
            conversation: climbing(),
            message: target,
        }]
    );
    assert_eq!(
        state.popup,
        Some(Popup::MessageInfo {
            target,
            receipts: vec![],
        })
    );

    let r1 = RecipientReceipt {
        recipient: ALICE,
        status: DeliveryStatus::Delivered,
        updated_at: Timestamp::from_millis(fixtures::NOW_MS + 100),
    };
    update(
        &mut state,
        AppEvent::MessageReceiptsLoaded {
            conversation: climbing(),
            message: target,
            receipts: vec![r1],
        },
    );
    assert_eq!(
        state.popup,
        Some(Popup::MessageInfo {
            target,
            receipts: vec![r1],
        })
    );

    // Live DeliveryUpdate with provenance updates the open popup's receipts in place.
    update(
        &mut state,
        AppEvent::Backend(BackendEvent::DeliveryUpdate {
            message: target,
            status: DeliveryStatus::Read,
            by: Some(ALICE),
            updated_at: Some(Timestamp::from_millis(fixtures::NOW_MS + 200)),
        }),
    );
    assert_eq!(
        state.popup,
        Some(Popup::MessageInfo {
            target,
            receipts: vec![RecipientReceipt {
                recipient: ALICE,
                status: DeliveryStatus::Read,
                updated_at: Timestamp::from_millis(fixtures::NOW_MS + 200),
            }],
        })
    );

    // Esc closes the popup.
    update(&mut state, key(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(state.popup, None);
}

#[test]
fn shortcode_autocomplete_completes_on_tab_enter_and_closing_colon() {
    let mut state = loaded_state();
    assert_eq!(state.focus, Focus::Input);
    let type_str = |state: &mut State, s: &str| {
        for c in s.chars() {
            press(state, c);
        }
    };

    // Mid-word colons like `10:30` never trigger shortcode completion.
    type_str(&mut state, "meet at 10:30 ");
    assert_eq!(state.shortcode_candidates(), [] as [(&str, &str); 0]);

    // Typing `:thu` after whitespace opens shortcode candidates; Tab completes `👍`.
    type_str(&mut state, ":thu");
    assert_eq!(
        state.shortcode_candidates().first().copied(),
        Some(("thumbsup", "👍"))
    );
    update(&mut state, key(KeyCode::Tab, KeyModifiers::NONE));
    assert_eq!(
        state.open.as_ref().unwrap().input.text(),
        "meet at 10:30 👍"
    );
    assert_eq!(state.shortcode_candidates(), [] as [(&str, &str); 0]);

    // Typing an exact `:fire:` with a closing colon auto-replaces without Enter/Tab.
    type_str(&mut state, " :fire:");
    assert_eq!(
        state.open.as_ref().unwrap().input.text(),
        "meet at 10:30 👍 🔥"
    );

    // Navigating with Down/Up and pressing Enter completes the shortcode instead of sending.
    type_str(&mut state, " :ha");
    let candidates = state.shortcode_candidates();
    assert!(candidates.len() >= 2);
    let second_emoji = candidates[1].1;
    update(&mut state, key(KeyCode::Down, KeyModifiers::NONE));
    assert_eq!(state.open.as_ref().unwrap().shortcode_selected, 1);
    let effects = update(&mut state, key(KeyCode::Enter, KeyModifiers::NONE));
    assert!(
        !effects
            .iter()
            .any(|e| matches!(e, Effect::Backend(Command::Send { .. }))),
        "Enter while shortcode popup is active should complete, not send"
    );
    assert_eq!(
        state.open.as_ref().unwrap().input.text(),
        format!("meet at 10:30 👍 🔥 {second_emoji}")
    );

    // Esc dismisses the shortcode popup without clearing input or closing the conversation.
    type_str(&mut state, " :ro");
    assert_ne!(state.shortcode_candidates(), [] as [(&str, &str); 0]);
    update(&mut state, key(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(state.shortcode_candidates(), [] as [(&str, &str); 0]);
    assert!(state.open.is_some());
    assert!(state.open.as_ref().unwrap().input.text().ends_with(":ro"));
}

#[test]
fn reaction_picker_search_mode_filters_and_reacts() {
    let mut state = loaded_state();
    let target = climbing_history().last().unwrap().reference();
    update(&mut state, key(KeyCode::Up, KeyModifiers::NONE));
    press(&mut state, '+');
    assert_eq!(
        state.popup,
        Some(Popup::React {
            target,
            selected: 0,
            query: None,
        })
    );

    // Pressing `/` enters full shortcode search mode.
    press(&mut state, '/');
    assert_eq!(
        state.popup,
        Some(Popup::React {
            target,
            selected: 0,
            query: Some(String::new()),
        })
    );

    // Typing letters (including `h`/`j`/`k`/`l`/`q`) edits the query in search mode.
    for c in "firex".chars() {
        press(&mut state, c);
    }
    assert_eq!(
        state.popup,
        Some(Popup::React {
            target,
            selected: 0,
            query: Some("firex".into()),
        })
    );
    update(&mut state, key(KeyCode::Backspace, KeyModifiers::NONE));
    assert_eq!(
        state.popup,
        Some(Popup::React {
            target,
            selected: 0,
            query: Some("fire".into()),
        })
    );

    let effects = update(&mut state, key(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(state.popup, None);
    assert!(effects.contains(&Effect::Backend(Command::React {
        conversation: climbing(),
        target,
        change: ReactionChange::Set("🔥".into()),
        previous: None,
    })));
}

#[test]
fn forward_message_opens_destination_picker_and_sends_copy() {
    let mut state = loaded_state();
    let source = climbing_history().last().unwrap().clone();
    let target = source.reference();

    // Select the newest message and press `f`.
    update(&mut state, key(KeyCode::Up, KeyModifiers::NONE));
    assert_eq!(state.focus, Focus::Messages);
    let effects = press(&mut state, 'f');
    assert_eq!(effects, vec![]);
    assert_eq!(
        state
            .palette
            .as_ref()
            .map(crate::app::palette::Palette::level),
        Some(crate::app::palette::Level::ForwardTo(target))
    );

    // Filter to Bob and press Enter to forward.
    typed(&mut state, "Bob");
    let effects = update(&mut state, key(KeyCode::Enter, KeyModifiers::NONE));
    assert!(state.palette.is_none());
    assert_eq!(open_id(&state), Some(bob()));
    assert_eq!(
        state.notice.as_ref().map(|n| n.text.as_str()),
        Some("Forwarded to Bob")
    );

    let sent_cmd = effects.iter().find_map(|e| {
        if let Effect::Backend(Command::Send {
            conversation,
            message,
        }) = e
        {
            Some((*conversation, message.clone()))
        } else {
            None
        }
    });
    let (dest, outgoing) = sent_cmd.expect("expected Command::Send");
    assert_eq!(dest, bob());
    assert_eq!(Some(&outgoing.body), source.body.as_ref());
    assert!(outgoing.quote.is_none());
}

#[test]
fn forward_attachment_only_message_requires_download_first() {
    let mut state = loaded_state();
    let mut msg = text(climbing(), ALICE, fixtures::NOW_MS + 500, "");
    msg.attachments = vec![faro_core::domain::AttachmentRef {
        file_name: Some("climb.jpg".to_owned()),
        size: Some(1024),
        ..faro_core::domain::AttachmentRef::remote("att-remote", "image/jpeg")
    }];
    update(&mut state, AppEvent::Backend(BackendEvent::Message(msg)));

    update(&mut state, key(KeyCode::Up, KeyModifiers::NONE));
    press(&mut state, 'f');
    assert!(state.palette.is_none());
    assert_eq!(
        state.notice.as_ref().map(|n| n.text.as_str()),
        Some("Press d to download the attachment before forwarding")
    );

    // Forwarding the downloaded photo message (`Route map from last time`) attaches its file.
    let mut state = loaded_state();
    // Select `photo` (index 5 out of 9 messages: 4 steps up from the newest).
    update(&mut state, key(KeyCode::Up, KeyModifiers::NONE));
    for _ in 0..3 {
        press(&mut state, 'k');
    }
    press(&mut state, 'f');
    typed(&mut state, "Bob");
    let effects = update(&mut state, key(KeyCode::Enter, KeyModifiers::NONE));
    let outgoing = effects
        .iter()
        .find_map(|e| {
            if let Effect::Backend(Command::Send { message, .. }) = e {
                Some(message.clone())
            } else {
                None
            }
        })
        .expect("expected Command::Send");
    assert_eq!(outgoing.body.expose(), "Route map from last time");
    assert_eq!(outgoing.attachments.len(), 1);
    assert_eq!(
        outgoing.attachments[0].file_name.as_deref(),
        Some("route-map.jpg")
    );
}

mod curation;
mod notify;
mod palette;
mod people;
mod presence;
mod settings;
mod terminal;
