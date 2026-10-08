//! Notifications (M6, D49): when the bell rings, and what the unread total
//! (the window title's count) includes.

use faro_core::domain::{Mention, ServiceId};
use faro_core::store::{ConversationState, NotifyLevel};

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

#[test]
fn the_window_title_shows_the_unread_total() {
    let mut state = loaded_state();
    let now = wall_clock();
    assert_eq!(state.window_title(now).as_deref(), Some("faro (1)"));
    incoming(&mut state, bob(), 1);
    assert_eq!(state.window_title(now).as_deref(), Some("faro (2)"));
    for id in [alice(), bob()] {
        state
            .conversations
            .iter_mut()
            .find(|c| c.id == id)
            .unwrap()
            .unread = 0;
    }
    assert_eq!(state.window_title(now).as_deref(), Some("faro"));
    state.config.notifications.title = false;
    assert_eq!(state.window_title(now), None);
}

#[test]
fn bell_and_title_can_be_switched_from_the_palette() {
    use crate::app::palette::SettingChoice;
    use faro_core::config::{ConfigValue, SettingEdit, key};

    let mut state = loaded_state();
    let effects = crate::app::update::settings::choose(&mut state, SettingChoice::Bell(false));
    assert!(!state.config.notifications.bell);
    assert_eq!(
        effects,
        vec![Effect::SaveSetting(SettingEdit {
            path: key::BELL,
            value: ConfigValue::Bool(false),
        })]
    );
    assert_eq!(bells(&incoming(&mut state, alice(), 1)), 0);

    let effects = crate::app::update::settings::choose(&mut state, SettingChoice::Title(false));
    assert_eq!(state.window_title(wall_clock()), None);
    assert_eq!(
        effects,
        vec![Effect::SaveSetting(SettingEdit {
            path: key::TITLE,
            value: ConfigValue::Bool(false),
        })]
    );
}

#[test]
fn mentions_only_is_quiet_until_mentions_exist() {
    use faro_core::store::NotifyLevel;

    let mut state = loaded_state();
    // `@` in the sidebar acts on the open conversation (Climbing).
    state.focus = Focus::Conversations;
    let effects = update(&mut state, key(KeyCode::Char('@'), KeyModifiers::NONE));
    let climbing_state = state.summary(climbing()).unwrap().state.clone();
    assert_eq!(climbing_state.notify, NotifyLevel::Mentions);
    assert_eq!(
        effects,
        vec![Effect::SaveConversationState(
            climbing(),
            Box::new(climbing_state)
        )]
    );

    // Not rung for, nor counted, even when not in front of the user.
    update(&mut state, AppEvent::Terminal(Event::FocusLost));
    assert_eq!(bells(&incoming(&mut state, climbing(), 1)), 0);
    assert_eq!(state.summary(climbing()).unwrap().unread, 1);
    assert_eq!(state.unread_total(wall_clock()), 1, "only Alice's");

    // And back.
    update(&mut state, AppEvent::Terminal(Event::FocusGained));
    update(&mut state, key(KeyCode::Char('@'), KeyModifiers::NONE));
    assert_eq!(
        state.summary(climbing()).unwrap().state.notify,
        NotifyLevel::All
    );
    state.last_bell = None;
    update(&mut state, AppEvent::Terminal(Event::FocusLost));
    assert_eq!(bells(&incoming(&mut state, climbing(), 2)), 1);
}

#[test]
fn alt_n_is_do_not_disturb() {
    use faro_core::config::{ConfigValue, SettingEdit, key};

    let saved = |on| {
        vec![Effect::SaveSetting(SettingEdit {
            path: key::BELL,
            value: ConfigValue::Bool(on),
        })]
    };
    let mut state = loaded_state();
    // Global: works from the input (where Alt+B/F/D edit) and elsewhere.
    let effects = update(&mut state, key(KeyCode::Char('n'), KeyModifiers::ALT));
    assert!(!state.config.notifications.bell);
    assert_eq!(effects, saved(false));
    assert_eq!(
        state.notice.as_ref().map(|n| n.text.as_str()),
        Some("Do not disturb: the bell is off")
    );
    assert_eq!(bells(&incoming(&mut state, alice(), 1)), 0);
    // The input wasn't typed into.
    assert_eq!(state.open.as_ref().unwrap().input.text(), "");

    state.focus = crate::app::state::Focus::Conversations;
    let effects = update(&mut state, key(KeyCode::Char('n'), KeyModifiers::ALT));
    assert!(state.config.notifications.bell);
    assert_eq!(effects, saved(true));
    assert_eq!(bells(&incoming(&mut state, alice(), 2)), 1);
}

#[test]
fn do_not_disturb_from_the_palette() {
    let mut state = loaded_state();
    update(&mut state, key(KeyCode::Char('p'), KeyModifiers::CONTROL));
    typed(&mut state, ">disturb");
    let item = state.palette.as_ref().unwrap().selected_item().unwrap();
    assert_eq!(item.title, "Turn do not disturb on (bell off)");
    assert_eq!(item.detail, "alt+n");
    update(&mut state, key(KeyCode::Enter, KeyModifiers::NONE));
    assert!(!state.config.notifications.bell);
    update(&mut state, key(KeyCode::Char('p'), KeyModifiers::CONTROL));
    typed(&mut state, ">disturb");
    let item = state.palette.as_ref().unwrap().selected_item().unwrap();
    assert_eq!(item.title, "Turn do not disturb off (bell on)");
}

#[test]
fn reloading_the_file_with_the_bell_off_is_do_not_disturb_too() {
    let mut state = loaded_state();
    update(
        &mut state,
        AppEvent::ConfigRead(Ok("[notifications]\nbell = false\n".to_owned())),
    );
    assert!(!state.config.notifications.bell);
    assert_eq!(bells(&incoming(&mut state, alice(), 1)), 0);
}

fn mentioning(state: &mut State, who: ServiceId, at: u64) -> Vec<Effect> {
    let mut message = text(climbing(), ALICE, fixtures::NOW_MS + at, "\u{FFFC} ping");
    message.mentions = vec![Mention {
        who,
        start: 0,
        len: '\u{FFFC}'.len_utf8(),
    }];
    update(state, AppEvent::Backend(BackendEvent::Message(message)))
}

#[test]
fn mentions_only_rings_for_mentions_of_us_unless_muted() {
    let mut state = loaded_state();
    edit_state(&mut state, climbing(), |s| s.notify = NotifyLevel::Mentions);
    update(&mut state, AppEvent::Terminal(Event::FocusLost));
    let total = state.unread_total(wall_clock());

    assert_eq!(bells(&mentioning(&mut state, BOB, 1)), 0, "someone else");
    assert_eq!(bells(&mentioning(&mut state, ME, 2)), 1, "us");
    // Counted as unread, but not in the title's total (D72).
    assert_eq!(state.summary(climbing()).unwrap().unread, 2);
    assert_eq!(state.unread_total(wall_clock()), total);

    // A mute silences mentions too.
    state.last_bell = None;
    edit_state(&mut state, climbing(), |s| {
        s.muted_until = Some(ConversationState::MUTED_FOREVER);
    });
    assert_eq!(bells(&mentioning(&mut state, ME, 3)), 0);
}
