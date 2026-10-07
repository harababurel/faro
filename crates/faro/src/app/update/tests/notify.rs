//! Notifications (M6, D49): when the bell rings, and what the unread total
//! (the window title's count) includes.

use faro_core::store::ConversationState;

use super::*;
use crate::app::state::BELL_INTERVAL;

fn incoming(state: &mut State, conversation: ConversationId, at: u64) -> Vec<Effect> {
    let message = text(conversation, ALICE, fixtures::NOW_MS + at, "ping");
    update(state, AppEvent::Backend(BackendEvent::Message(message)))
}

fn bells(effects: &[Effect]) -> usize {
    effects.iter().filter(|e| **e == Effect::Bell).count()
}

fn edit_state(state: &mut State, id: ConversationId, change: impl FnOnce(&mut ConversationState)) {
    let summary = state.conversations.iter_mut().find(|c| c.id == id).unwrap();
    change(&mut summary.state);
}

#[test]
fn a_message_in_a_background_conversation_rings() {
    let mut state = loaded_state();
    assert_eq!(bells(&incoming(&mut state, alice(), 1)), 1);
    assert!(state.last_bell.is_some());
}

#[test]
fn a_message_already_in_front_of_the_user_doesnt_ring() {
    let mut state = loaded_state();
    assert!(state.seeing_open());
    assert_eq!(bells(&incoming(&mut state, climbing(), 1)), 0);
}

#[test]
fn the_open_conversation_rings_when_the_terminal_is_unfocused_or_scrolled_up() {
    let mut state = loaded_state();
    update(&mut state, AppEvent::Terminal(Event::FocusLost));
    assert_eq!(bells(&incoming(&mut state, climbing(), 1)), 1);

    let mut state = loaded_state();
    let last = climbing_history().last().unwrap().reference();
    state.open.as_mut().unwrap().scroll = Some(crate::app::state::Anchor {
        message: last,
        offset: 3,
    });
    assert_eq!(bells(&incoming(&mut state, climbing(), 1)), 1);
}

#[test]
fn muted_archived_and_not_allowed_conversations_dont_ring() {
    let mut state = loaded_state();
    edit_state(&mut state, alice(), |s| {
        s.muted_until = Some(ConversationState::MUTED_FOREVER);
    });
    assert_eq!(bells(&incoming(&mut state, alice(), 1)), 0);

    let mut state = loaded_state();
    edit_state(&mut state, alice(), |s| s.archived = true);
    assert_eq!(bells(&incoming(&mut state, alice(), 1)), 0);

    let mut state = loaded_state();
    state.list.allowlist = true;
    assert_eq!(bells(&incoming(&mut state, alice(), 1)), 0);
    edit_state(&mut state, alice(), |s| s.allowed = true);
    assert_eq!(bells(&incoming(&mut state, alice(), 2)), 1);
}

#[test]
fn a_hidden_conversation_comes_back_and_rings() {
    let mut state = loaded_state();
    edit_state(&mut state, alice(), |s| s.hidden = true);
    assert_eq!(bells(&incoming(&mut state, alice(), 1)), 1);
}

#[test]
fn a_flood_rings_once_per_interval() {
    let mut state = loaded_state();
    assert_eq!(bells(&incoming(&mut state, alice(), 1)), 1);
    assert_eq!(bells(&incoming(&mut state, alice(), 2)), 0);
    assert_eq!(bells(&incoming(&mut state, bob(), 3)), 0);
    state.last_bell = Instant::now().checked_sub(BELL_INTERVAL);
    assert_eq!(bells(&incoming(&mut state, bob(), 4)), 1);
}

#[test]
fn the_bell_setting_turns_it_off() {
    let mut state = loaded_state();
    state.config.notifications.bell = false;
    assert_eq!(bells(&incoming(&mut state, alice(), 1)), 0);
}

#[test]
fn own_deleted_and_early_messages_dont_ring() {
    let mut state = loaded_state();
    // Sent from our phone (a sync message).
    let mine = text(alice(), ME, fixtures::NOW_MS + 1, "on my way");
    let effects = update(&mut state, AppEvent::Backend(BackendEvent::Message(mine)));
    assert_eq!(bells(&effects), 0);
    let mut gone = text(alice(), ALICE, fixtures::NOW_MS + 2, "");
    gone.deleted = true;
    let effects = update(&mut state, AppEvent::Backend(BackendEvent::Message(gone)));
    assert_eq!(bells(&effects), 0);

    // Before the conversation list has loaded, mutes aren't known.
    let mut state = State::new(ME, fixtures::contacts(), fixtures::groups());
    assert_eq!(bells(&incoming(&mut state, alice(), 1)), 0);
}

#[test]
fn the_unread_total_counts_conversations_that_notify() {
    let mut state = loaded_state();
    let now = wall_clock();
    // Alice has 1 unread; Climbing is open and read.
    assert_eq!(state.unread_total(now), 1);
    incoming(&mut state, bob(), 1);
    assert_eq!(state.unread_total(now), 2);
    edit_state(&mut state, bob(), |s| {
        s.muted_until = Some(ConversationState::MUTED_FOREVER);
    });
    assert_eq!(state.unread_total(now), 1);
    edit_state(&mut state, alice(), |s| s.archived = true);
    assert_eq!(state.unread_total(now), 0);
}
