//! Curation (M4 task 5, D41): pin, mute, hide, archive, allowlist, views.

use crossterm::event::{MouseButton, MouseEventKind};
use faro_core::config::{Config, ConfigValue, SettingEdit, key};
use faro_core::store::ConversationState;

use super::*;
use crate::app::state::{ConversationAction, Section, SidebarRow, SidebarView};
use crate::app::update::configure;

fn sidebar_key(state: &mut State, c: char) -> Vec<Effect> {
    state.focus = Focus::Conversations;
    update(state, key(KeyCode::Char(c), KeyModifiers::NONE))
}

fn listed(state: &State) -> Vec<ConversationId> {
    state.visible_conversations().map(|c| c.id).collect()
}

fn conversation_state(state: &State, id: ConversationId) -> ConversationState {
    state.summary(id).map(|s| s.state.clone()).unwrap()
}

fn saved(effects: &[Effect], id: ConversationId) -> Option<ConversationState> {
    effects.iter().find_map(|e| match e {
        Effect::SaveConversationState(saved, state) if *saved == id => Some((**state).clone()),
        Effect::SaveConversationState(..)
        | Effect::LoadConversations
        | Effect::LoadMessages(_)
        | Effect::LoadOlder { .. }
        | Effect::Persist(_)
        | Effect::SaveAttachment { .. }
        | Effect::MarkRead(_)
        | Effect::ReadElsewhere(_)
        | Effect::RememberOpen(_)
        | Effect::SaveNickname(..)
        | Effect::SaveSetting(_)
        | Effect::Backend(_)
        | Effect::Copy(_)
        | Effect::OpenUrl(_)
        | Effect::OpenFile(_)
        | Effect::SaveToDownloads { .. }
        | Effect::CheckTerminal
        | Effect::Bell => None,
    })
}

fn notice(state: &State) -> &str {
    state.notice.as_ref().map_or("", |n| n.text.as_str())
}

#[test]
fn pinning_adds_to_the_end_and_alt_digits_jump() {
    let mut state = loaded_state();
    // Bob is pinned at 0; Climbing is open.
    let effects = sidebar_key(&mut state, 'p');
    assert_eq!(saved(&effects, climbing()).and_then(|s| s.pinned), Some(1));
    assert_eq!(listed(&state), vec![bob(), climbing(), alice()]);
    assert_eq!(notice(&state), "Pinned (Alt+2 opens it)");

    update(&mut state, key(KeyCode::Char('1'), KeyModifiers::ALT));
    assert_eq!(open_id(&state), Some(bob()));
    update(&mut state, key(KeyCode::Char('2'), KeyModifiers::ALT));
    assert_eq!(open_id(&state), Some(climbing()));
    let effects = update(&mut state, key(KeyCode::Char('5'), KeyModifiers::ALT));
    assert_eq!(effects, vec![]);
    assert_eq!(open_id(&state), Some(climbing()));
    assert_eq!(notice(&state), "Nothing pinned at Alt+5");

    // Unpin: back among the recent ones.
    let effects = sidebar_key(&mut state, 'p');
    assert_eq!(saved(&effects, climbing()).map(|s| s.pinned), Some(None));
    assert_eq!(listed(&state), vec![bob(), climbing(), alice()]);
    assert_eq!(
        state.section(state.summary(climbing()).unwrap()),
        Section::Recent
    );
}

#[test]
fn pins_move_up_and_down() {
    let mut state = loaded_state();
    sidebar_key(&mut state, 'p');
    let effects = sidebar_key(&mut state, 'K');
    assert_eq!(listed(&state), vec![climbing(), bob(), alice()]);
    assert_eq!(saved(&effects, climbing()).and_then(|s| s.pinned), Some(0));
    assert_eq!(saved(&effects, bob()).and_then(|s| s.pinned), Some(1));
    // Already first: nothing to do.
    assert_eq!(sidebar_key(&mut state, 'K'), vec![]);
    sidebar_key(&mut state, 'J');
    assert_eq!(listed(&state), vec![bob(), climbing(), alice()]);
}

#[test]
fn hiding_moves_on_and_a_new_message_brings_it_back() {
    let mut state = loaded_state();
    let effects = sidebar_key(&mut state, 'h');
    assert_eq!(saved(&effects, climbing()).map(|s| s.hidden), Some(true));
    // Alice took Climbing's place, so she opens.
    assert_eq!(open_id(&state), Some(alice()));
    assert!(effects.contains(&Effect::LoadMessages(alice())));
    assert_eq!(listed(&state), vec![bob(), alice()]);
    assert!(notice(&state).starts_with("Hid Climbing"));

    let news = text(climbing(), BOB, fixtures::NOW_MS + 1, "you there?");
    let effects = update(
        &mut state,
        AppEvent::Backend(BackendEvent::Message(news.clone())),
    );
    assert_eq!(
        effects.first(),
        Some(&Effect::Persist(Box::new(BackendEvent::Message(news))))
    );
    assert_eq!(saved(&effects, climbing()).map(|s| s.hidden), Some(false));
    assert_eq!(listed(&state), vec![bob(), climbing(), alice()]);
}

#[test]
fn hiding_a_pinned_conversation_unpins_it() {
    let mut state = loaded_state();
    update(&mut state, key(KeyCode::Char('1'), KeyModifiers::ALT));
    sidebar_key(&mut state, 'h');
    let bob_state = conversation_state(&state, bob());
    assert!(bob_state.hidden);
    assert_eq!(bob_state.pinned, None);
}

#[test]
fn archive_view_lists_archived_and_hidden_conversations() {
    let mut state = loaded_state();
    sidebar_key(&mut state, 'a');
    assert!(conversation_state(&state, climbing()).archived);
    assert_eq!(open_id(&state), Some(alice()));
    sidebar_key(&mut state, 'h');
    assert_eq!(open_id(&state), Some(bob()));
    assert_eq!(listed(&state), vec![bob()]);

    // A: the archive. The open conversation (Bob) isn't in it.
    sidebar_key(&mut state, 'A');
    assert_eq!(state.list.view, SidebarView::Archive);
    assert_eq!(listed(&state), vec![climbing(), alice()]);
    // Keys act only on a listed conversation.
    assert_eq!(sidebar_key(&mut state, 'a'), vec![]);
    assert!(!conversation_state(&state, bob()).archived);
    sidebar_key(&mut state, 'j');
    assert_eq!(open_id(&state), Some(climbing()));

    // A new message doesn't unarchive.
    let news = text(climbing(), BOB, fixtures::NOW_MS + 1, "still archived");
    update(&mut state, AppEvent::Backend(BackendEvent::Message(news)));
    assert!(conversation_state(&state, climbing()).archived);

    // Unarchiving moves on to the next one in the archive...
    sidebar_key(&mut state, 'a');
    assert_eq!(open_id(&state), Some(alice()));
    assert_eq!(listed(&state), vec![alice()]);
    // ...and with the archive empty, back to the chats with it still open.
    sidebar_key(&mut state, 'h');
    assert_eq!(state.list.view, SidebarView::Chats);
    assert_eq!(open_id(&state), Some(alice()));
    assert_eq!(listed(&state), vec![bob(), climbing(), alice()]);
}

#[test]
fn hiding_the_last_conversation_closes_it() {
    let mut state = State::new(ME, fixtures::contacts(), vec![]);
    update(
        &mut state,
        AppEvent::Loaded {
            conversations: vec![fixtures::summary(alice(), "Alice", 1, 0)],
            last_open: None,
        },
    );
    update(&mut state, key(KeyCode::Enter, KeyModifiers::NONE));
    typed(&mut state, "draft");
    let effects = sidebar_key(&mut state, 'h');
    assert_eq!(open_id(&state), None);
    // The draft is saved along with the hidden flag.
    let last = effects.iter().rev().find_map(|e| {
        if let Effect::SaveConversationState(_, s) = e {
            Some(s)
        } else {
            None
        }
    });
    assert_eq!(
        last.map(|s| (s.hidden, s.draft.clone())),
        Some((true, Some(MessageBody::new("draft"))))
    );
}

fn allowlist_saved(on: bool) -> Effect {
    Effect::SaveSetting(SettingEdit {
        path: key::ALLOWLIST,
        value: ConfigValue::Bool(on),
    })
}

#[test]
fn allowlist_mode_lists_only_allowed_and_pinned_conversations() {
    let mut state = loaded_state();
    let effects = sidebar_key(&mut state, 'l');
    assert_eq!(saved(&effects, climbing()).map(|s| s.allowed), Some(true));
    let effects = sidebar_key(&mut state, 'L');
    assert_eq!(effects, vec![allowlist_saved(true)]);
    assert!(state.config.conversations.allowlist);
    assert!(state.list.allowlist);
    assert_eq!(listed(&state), vec![bob(), climbing()]);
    assert_eq!(
        state
            .listed_in(SidebarView::Archive)
            .map(|c| c.id)
            .collect::<Vec<_>>(),
        vec![alice()]
    );
    assert_eq!(
        state.section(state.summary(alice()).unwrap()),
        Section::NotAllowed
    );

    // Taking the open conversation off the list moves on.
    sidebar_key(&mut state, 'l');
    assert_eq!(open_id(&state), Some(bob()));
    let effects = sidebar_key(&mut state, 'L');
    assert_eq!(effects, vec![allowlist_saved(false)]);
    assert_eq!(listed(&state), vec![bob(), climbing(), alice()]);
}

#[test]
fn startup_restores_allowlist_mode_and_the_archive_view() {
    let mut state = State::new(ME, fixtures::contacts(), fixtures::groups());
    let mut config = Config::default();
    config.conversations.allowlist = true;
    assert_eq!(configure(&mut state, config, vec![]), vec![]);
    let mut summaries = fixtures::summaries();
    for summary in &mut summaries {
        if summary.id == climbing() {
            summary.state.archived = true;
        }
    }
    update(
        &mut state,
        AppEvent::Loaded {
            conversations: summaries,
            last_open: Some(climbing()),
        },
    );
    assert!(state.list.allowlist);
    assert_eq!(open_id(&state), None);
    assert_eq!(state.highlighted(), Some(climbing()));
    assert_eq!(state.list.view, SidebarView::Archive);
}

#[test]
fn muting_picks_a_length_and_unmuting_is_one_key() {
    let mut state = loaded_state();
    sidebar_key(&mut state, 'm');
    assert_eq!(
        state.popup,
        Some(Popup::Mute {
            target: climbing(),
            selected: 0
        })
    );
    let effects = update(&mut state, key(KeyCode::Char('2'), KeyModifiers::NONE));
    assert_eq!(state.popup, None);
    let until = saved(&effects, climbing())
        .and_then(|s| s.muted_until)
        .unwrap();
    let eight_hours = 8 * 60 * 60 * 1000;
    let wall = super::sending::wall_clock().as_millis();
    assert!(until.as_millis().abs_diff(wall + eight_hours) < 60_000);
    assert_eq!(notice(&state), "Muted for 8 hours");
    assert!(conversation_state(&state, climbing()).muted_at(super::sending::wall_clock()));

    let effects = sidebar_key(&mut state, 'm');
    assert_eq!(
        saved(&effects, climbing()).map(|s| s.muted_until),
        Some(None)
    );
    assert_eq!(state.popup, None);

    // "Until unmuted".
    sidebar_key(&mut state, 'm');
    update(&mut state, key(KeyCode::Up, KeyModifiers::NONE));
    let effects = update(&mut state, key(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(
        saved(&effects, climbing()).and_then(|s| s.muted_until),
        Some(ConversationState::MUTED_FOREVER)
    );
}

#[test]
fn muted_conversations_dont_count_towards_the_unread_total() {
    let mut state = loaded_state();
    let now = Timestamp::from_millis(fixtures::NOW_MS);
    assert_eq!(state.unread_total(now), 1);
    if let Some(alice) = state.conversations.iter_mut().find(|c| c.id == alice()) {
        alice.state.muted_until = Some(Timestamp::from_millis(fixtures::NOW_MS + 1));
    }
    assert_eq!(state.unread_total(now), 0);
}

#[test]
fn the_conversation_menu_offers_what_applies() {
    let mut state = loaded_state();
    sidebar_key(&mut state, ' ');
    let Some(Popup::Conversation { target, items, .. }) = &state.popup else {
        panic!("no conversation menu: {:?}", state.popup);
    };
    assert_eq!(*target, climbing());
    assert_eq!(
        items,
        &vec![
            ConversationAction::Pin,
            ConversationAction::Mute,
            ConversationAction::MentionsOnly,
            ConversationAction::Hide,
            ConversationAction::Archive,
            ConversationAction::Allow,
            ConversationAction::AllowlistMode(true),
        ]
    );
    // `h` is a shortcut in menus, not a move.
    update(&mut state, key(KeyCode::Char('h'), KeyModifiers::NONE));
    assert_eq!(state.popup, None);
    assert!(conversation_state(&state, climbing()).hidden);

    // One pin: nothing to move it past.
    update(&mut state, key(KeyCode::Char('1'), KeyModifiers::ALT));
    sidebar_key(&mut state, ' ');
    let Some(Popup::Conversation { items, .. }) = &state.popup else {
        panic!("no conversation menu");
    };
    assert_eq!(items.first(), Some(&ConversationAction::Unpin));
    assert!(!items.contains(&ConversationAction::MoveUp));
}

#[test]
fn right_click_on_a_conversation_opens_its_menu_without_switching() {
    let mut state = loaded_state();
    state.geometry = geometry(&state);
    // Rows: Bob, Climbing, Alice from y=1.
    update(
        &mut state,
        mouse(MouseEventKind::Down(MouseButton::Right), 5, 3),
    );
    assert_eq!(open_id(&state), Some(climbing()));
    let Some(Popup::Conversation { target, .. }) = &state.popup else {
        panic!("no conversation menu");
    };
    assert_eq!(*target, alice());
    update(&mut state, key(KeyCode::Char('a'), KeyModifiers::NONE));
    assert!(conversation_state(&state, alice()).archived);
    assert_eq!(open_id(&state), Some(climbing()));
}

#[test]
fn clicking_the_switch_row_changes_the_view() {
    let mut state = loaded_state();
    let mut geometry = geometry(&state);
    geometry.sidebar_rows = vec![SidebarRow::SwitchView];
    state.geometry = geometry;
    update(
        &mut state,
        mouse(MouseEventKind::Down(MouseButton::Left), 5, 1),
    );
    assert_eq!(state.list.view, SidebarView::Archive);
    assert_eq!(state.focus, Focus::Conversations);
}

#[test]
fn empty_conversations_show_when_pinned_or_allowed() {
    let mut state = loaded_state();
    let mut empty = fixtures::summary(ConversationId::Direct(CAROL), "Carol", 0, 0);
    empty.last_activity = None;
    state.conversations.push(empty);
    assert!(!listed(&state).contains(&ConversationId::Direct(CAROL)));
    if let Some(carol) = state
        .conversations
        .iter_mut()
        .find(|c| c.id == ConversationId::Direct(CAROL))
    {
        carol.state.allowed = true;
    }
    assert!(!listed(&state).contains(&ConversationId::Direct(CAROL)));
    state.list.allowlist = true;
    assert!(listed(&state).contains(&ConversationId::Direct(CAROL)));
}

fn esc(state: &mut State) -> Vec<Effect> {
    update(state, key(KeyCode::Esc, KeyModifiers::NONE))
}

#[test]
fn esc_steps_back_then_closes_the_conversation() {
    let mut state = loaded_state();
    typed(&mut state, "half a thought");
    // A notice goes first.
    state.notice = Some(crate::app::state::Notice::warning("hello"));
    assert_eq!(esc(&mut state), vec![]);
    assert_eq!(state.notice, None);
    assert_eq!(open_id(&state), Some(climbing()));
    // From the list (or the messages), back to the input.
    state.focus = Focus::Conversations;
    assert_eq!(esc(&mut state), vec![]);
    assert_eq!(state.focus, Focus::Input);
    assert_eq!(open_id(&state), Some(climbing()));
    // Then it closes, keeping the draft; the list highlights it (D60).
    state.panels.left = false;
    let effects = esc(&mut state);
    assert_eq!(open_id(&state), None);
    assert_eq!(state.focus, Focus::Conversations);
    assert!(state.panels.left, "the list is shown");
    assert_eq!(state.highlighted(), Some(climbing()));
    let draft = effects.iter().find_map(|e| {
        let Effect::SaveConversationState(id, s) = e else {
            return None;
        };
        (*id == climbing()).then(|| s.draft.clone()).flatten()
    });
    assert_eq!(draft, Some(MessageBody::new("half a thought")));
    // Nothing more to close.
    assert_eq!(esc(&mut state), vec![]);
    // Opening it again brings the draft back.
    update(&mut state, key(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(
        state.open.as_ref().map(|o| o.input.text().to_owned()),
        Some("half a thought".to_owned())
    );
}

#[test]
fn with_nothing_open_the_list_moves_without_reading() {
    let mut state = loaded_state();
    while state.open.is_some() {
        esc(&mut state);
    }
    assert_eq!(listed(&state), vec![bob(), climbing(), alice()]);
    // A message to the conversation just closed stays unread.
    let unread = |state: &State, id| state.summary(id).map(|s| s.unread);
    let before = unread(&state, climbing());
    let new = text(climbing(), BOB, fixtures::NOW_MS + 60_000, "psst");
    update(&mut state, AppEvent::Backend(BackendEvent::Message(new)));
    assert_eq!(open_id(&state), None);
    assert_eq!(unread(&state, climbing()), before.map(|n| n + 1));
    // j/k and Alt+J/K move the highlight; nothing opens or is read.
    assert_eq!(sidebar_key(&mut state, 'j'), vec![]);
    assert_eq!(state.highlighted(), Some(alice()));
    sidebar_key(&mut state, 'k');
    sidebar_key(&mut state, 'k');
    assert_eq!(state.highlighted(), Some(bob()), "stops at the top");
    state.focus = Focus::Input;
    let alt_j = update(&mut state, key(KeyCode::Char('j'), KeyModifiers::ALT));
    assert_eq!(alt_j, vec![]);
    assert_eq!(state.highlighted(), Some(climbing()));
    assert_eq!(state.focus, Focus::Conversations);
    assert_eq!(open_id(&state), None);
    // Curation keys act on the highlighted conversation.
    sidebar_key(&mut state, 'm');
    assert!(matches!(state.popup, Some(Popup::Mute { target, .. }) if target == climbing()));
    state.popup = None;
    // Enter opens it, and only then is it read.
    let effects = update(&mut state, key(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(open_id(&state), Some(climbing()));
    assert!(effects.contains(&Effect::MarkRead(climbing())));
    assert_eq!(unread(&state, climbing()), Some(0));
}
