//! What we tell others (M4 task 6, D42): typing indicators and read receipts.

use faro_core::domain::{Direction, MessageRef, Timestamp};

use super::*;
use crate::app::state::{TYPING_PAUSE, TYPING_REFRESH};

fn typing_effect(conversation: ConversationId, state: TypingState) -> Effect {
    Effect::Backend(Command::Typing {
        conversation,
        state,
    })
}

fn typing_effects(effects: &[Effect]) -> Vec<TypingState> {
    effects
        .iter()
        .filter_map(|e| {
            if let Effect::Backend(Command::Typing { state, .. }) = e {
                Some(*state)
            } else {
                None
            }
        })
        .collect()
}

fn receipts(effects: &[Effect]) -> Vec<MessageRef> {
    effects
        .iter()
        .filter_map(|e| {
            if let Effect::Backend(Command::MarkRead(refs)) = e {
                Some(refs.clone())
            } else {
                None
            }
        })
        .flatten()
        .collect()
}

fn ago(duration: Duration) -> Instant {
    Instant::now().checked_sub(duration).unwrap()
}

fn focus(state: &mut State, gained: bool) -> Vec<Effect> {
    let event = if gained {
        Event::FocusGained
    } else {
        Event::FocusLost
    };
    update(state, AppEvent::Terminal(event))
}

#[test]
fn typing_starts_once_and_refreshes() {
    let mut state = loaded_state();
    let effects = update(&mut state, key(KeyCode::Char('h'), KeyModifiers::NONE));
    assert_eq!(
        effects,
        vec![typing_effect(climbing(), TypingState::Started)]
    );
    let effects = update(&mut state, key(KeyCode::Char('i'), KeyModifiers::NONE));
    assert_eq!(effects, vec![]);
    // Cursor movement isn't an edit.
    assert_eq!(
        update(&mut state, key(KeyCode::Left, KeyModifiers::NONE)),
        vec![]
    );

    // Still typing after the refresh interval: say so again.
    let sent = state.open.as_mut().unwrap().typing_sent.as_mut().unwrap();
    sent.started = ago(TYPING_REFRESH);
    let effects = update(&mut state, key(KeyCode::Char('!'), KeyModifiers::NONE));
    assert_eq!(typing_effects(&effects), vec![TypingState::Started]);
}

#[test]
fn typing_stops_after_a_pause() {
    let mut state = loaded_state();
    typed(&mut state, "hey");
    // The pause is a deadline, so the loop wakes up for it.
    assert!(state.next_deadline().is_some());
    assert_eq!(update(&mut state, AppEvent::Tick), vec![]);

    let sent = state.open.as_mut().unwrap().typing_sent.as_mut().unwrap();
    sent.last_edit = ago(TYPING_PAUSE);
    let effects = update(&mut state, AppEvent::Tick);
    assert_eq!(typing_effects(&effects), vec![TypingState::Stopped]);
    assert_eq!(state.open.as_ref().unwrap().typing_sent, None);
    assert_eq!(typing_effects(&update(&mut state, AppEvent::Tick)), vec![]);

    // Typing again starts again.
    let effects = update(&mut state, key(KeyCode::Char('!'), KeyModifiers::NONE));
    assert_eq!(typing_effects(&effects), vec![TypingState::Started]);
}

#[test]
fn typing_stops_when_the_input_is_emptied() {
    let mut state = loaded_state();
    typed(&mut state, "ok");
    update(&mut state, key(KeyCode::Backspace, KeyModifiers::NONE));
    let effects = update(&mut state, key(KeyCode::Backspace, KeyModifiers::NONE));
    assert_eq!(typing_effects(&effects), vec![TypingState::Stopped]);
    // Nothing more to delete, nothing more to say.
    let effects = update(&mut state, key(KeyCode::Backspace, KeyModifiers::NONE));
    assert_eq!(typing_effects(&effects), vec![]);
}

#[test]
fn typing_stops_on_quit_but_not_after_sending() {
    let mut state = loaded_state();
    typed(&mut state, "sent");
    let effects = update(&mut state, key(KeyCode::Enter, KeyModifiers::NONE));
    // The message itself ends the indicator for the recipients.
    assert_eq!(typing_effects(&effects), vec![]);
    assert_eq!(state.open.as_ref().unwrap().typing_sent, None);

    typed(&mut state, "unsent");
    let effects = update(&mut state, key(KeyCode::Char('q'), KeyModifiers::CONTROL));
    assert!(state.quit);
    assert_eq!(typing_effects(&effects), vec![TypingState::Stopped]);
}

#[test]
fn opening_a_conversation_receipts_its_unread_messages() {
    let mut state = State::new(ME, fixtures::contacts(), fixtures::groups());
    update(
        &mut state,
        AppEvent::Loaded {
            conversations: fixtures::summaries(),
            last_open: Some(climbing()),
        },
    );
    update(&mut state, key(KeyCode::Enter, KeyModifiers::NONE));
    let effects = update(
        &mut state,
        AppEvent::MessagesLoaded {
            conversation: climbing(),
            messages: climbing_history(),
        },
    );
    // Climbing had 2 unread: the last two incoming messages.
    let mut expected: Vec<MessageRef> = climbing_history()
        .iter()
        .rev()
        .filter(|m| m.direction == Direction::Incoming)
        .take(2)
        .map(Message::reference)
        .collect();
    expected.reverse();
    assert_eq!(receipts(&effects), expected);
    assert_eq!(
        state.open.as_ref().unwrap().unread_marker,
        expected.first().copied()
    );
    assert_eq!(state.open.as_ref().unwrap().unreceipted, vec![]);
}

#[test]
fn receipts_wait_while_the_terminal_is_unfocused() {
    let mut state = loaded_state();
    // As if it had been opened with nothing unread.
    state.open.as_mut().unwrap().unread_marker = None;
    assert_eq!(focus(&mut state, false), vec![]);
    assert!(!state.env.focused);

    let incoming = text(climbing(), BOB, fixtures::NOW_MS + 1, "you there?");
    let effects = update(
        &mut state,
        AppEvent::Backend(BackendEvent::Message(incoming.clone())),
    );
    assert_eq!(receipts(&effects), vec![]);
    assert_eq!(state.summary(climbing()).unwrap().unread, 1);
    let open = state.open.as_ref().unwrap();
    assert_eq!(open.unread_marker, Some(incoming.reference()));
    // Our own messages are never receipted.
    let mine = text(climbing(), ME, fixtures::NOW_MS + 2, "from my phone");
    update(&mut state, AppEvent::Backend(BackendEvent::Message(mine)));
    assert_eq!(
        state.open.as_ref().unwrap().unreceipted,
        vec![incoming.reference()]
    );

    // Back in the terminal: read.
    let effects = focus(&mut state, true);
    assert_eq!(receipts(&effects), vec![incoming.reference()]);
    assert!(effects.contains(&Effect::MarkRead(climbing())));
    assert_eq!(state.summary(climbing()).unwrap().unread, 0);
    assert_eq!(receipts(&focus(&mut state, true)), vec![]);
}

#[test]
fn a_key_press_counts_as_focus() {
    let mut state = loaded_state();
    focus(&mut state, false);
    let incoming = text(climbing(), BOB, fixtures::NOW_MS + 1, "hello");
    update(
        &mut state,
        AppEvent::Backend(BackendEvent::Message(incoming.clone())),
    );
    // Terminals without focus reporting: any input means someone is there.
    let effects = update(&mut state, key(KeyCode::Char('x'), KeyModifiers::NONE));
    assert!(state.env.focused);
    assert_eq!(receipts(&effects), vec![incoming.reference()]);
}

#[test]
fn receipts_wait_until_scrolled_back_down() {
    let mut state = loaded_state();
    update(&mut state, key(KeyCode::PageUp, KeyModifiers::NONE));
    assert!(state.open.as_ref().unwrap().scroll.is_some());

    let incoming = text(climbing(), BOB, fixtures::NOW_MS + 1, "look at this");
    let effects = update(
        &mut state,
        AppEvent::Backend(BackendEvent::Message(incoming.clone())),
    );
    assert_eq!(receipts(&effects), vec![]);
    assert_eq!(state.summary(climbing()).unwrap().unread, 1);

    // A frame with the view still scrolled up changes nothing.
    let drawn = Geometry {
        scroll: state.open.as_ref().unwrap().scroll,
        ..Geometry::default()
    };
    assert_eq!(
        receipts(&update(&mut state, AppEvent::Drawn(drawn))),
        vec![]
    );
    // At the bottom again: read.
    let effects = update(&mut state, AppEvent::Drawn(Geometry::default()));
    assert_eq!(receipts(&effects), vec![incoming.reference()]);
    assert_eq!(state.summary(climbing()).unwrap().unread, 0);
}

#[test]
fn messages_elsewhere_are_not_receipted() {
    let mut state = loaded_state();
    let incoming = text(alice(), ALICE, fixtures::NOW_MS + 1, "psst");
    let effects = update(
        &mut state,
        AppEvent::Backend(BackendEvent::Message(incoming)),
    );
    assert_eq!(receipts(&effects), vec![]);
    assert_eq!(state.open.as_ref().unwrap().unreceipted, vec![]);
}

#[test]
fn reads_on_the_phone_lower_unread_counts_by_what_the_store_cleared() {
    let mut state = loaded_state();
    let read = MessageRef {
        sender: ALICE,
        sent_at: Timestamp::from_millis(fixtures::NOW_MS),
    };
    let effects = update(
        &mut state,
        AppEvent::Backend(BackendEvent::ReadElsewhere(vec![read])),
    );
    // The store finds the conversations and works out the counts.
    assert_eq!(effects, vec![Effect::ReadElsewhere(vec![read])]);

    let before = state.summary(alice()).unwrap().unread;
    let incoming = text(alice(), ALICE, fixtures::NOW_MS + 1, "and another");
    update(
        &mut state,
        AppEvent::Backend(BackendEvent::Message(incoming)),
    );
    assert_eq!(state.summary(alice()).unwrap().unread, before + 1);
    // What the store cleared comes off ours: the message that arrived
    // meanwhile still counts, and nothing goes below zero.
    update(
        &mut state,
        AppEvent::UnreadCleared(vec![(alice(), before), (bob(), 99)]),
    );
    assert_eq!(state.summary(alice()).unwrap().unread, 1);
    assert_eq!(state.summary(bob()).unwrap().unread, 0);
}

#[test]
fn reads_on_the_phone_need_no_receipts_from_faro() {
    let mut state = loaded_state();
    // As if it had been opened with nothing unread.
    state.open.as_mut().unwrap().unread_marker = None;
    focus(&mut state, false);
    let first = text(climbing(), BOB, fixtures::NOW_MS + 1, "one");
    let second = text(climbing(), CAROL, fixtures::NOW_MS + 2, "two");
    let third = text(climbing(), BOB, fixtures::NOW_MS + 3, "three");
    for message in [&first, &second, &third] {
        update(
            &mut state,
            AppEvent::Backend(BackendEvent::Message(message.clone())),
        );
    }
    assert_eq!(
        state.open.as_ref().unwrap().unread_marker,
        Some(first.reference())
    );

    // Read on the phone up to the second: the "new messages" rule above
    // them goes, and only the third still waits for a receipt.
    update(
        &mut state,
        AppEvent::Backend(BackendEvent::ReadElsewhere(vec![second.reference()])),
    );
    let open = state.open.as_ref().unwrap();
    assert_eq!(open.unread_marker, None);
    assert_eq!(open.unreceipted, vec![third.reference()]);
    let effects = focus(&mut state, true);
    assert_eq!(receipts(&effects), vec![third.reference()]);

    // Messages faro doesn't show change nothing here.
    let elsewhere = MessageRef {
        sender: ALICE,
        sent_at: Timestamp::from_millis(1),
    };
    let incoming = text(climbing(), BOB, fixtures::NOW_MS + 4, "four");
    focus(&mut state, false);
    update(
        &mut state,
        AppEvent::Backend(BackendEvent::Message(incoming.clone())),
    );
    update(
        &mut state,
        AppEvent::Backend(BackendEvent::ReadElsewhere(vec![elsewhere])),
    );
    let open = state.open.as_ref().unwrap();
    assert_eq!(open.unread_marker, Some(incoming.reference()));
    assert_eq!(open.unreceipted, vec![incoming.reference()]);
}
