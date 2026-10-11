//! Telling same-name people apart, and nicknames (D65).

use faro_core::domain::Contact;
use uuid::Uuid;

use super::*;
use crate::app::palette::{ItemKind, Level};
use crate::app::state::ConversationAction;

/// Someone else called "Alice 🌸", whose ID starts with `abcd`.
const DAVE: ServiceId = ServiceId::Aci(Uuid::from_u128(0xabcd << 112));

fn with_second_alice() -> State {
    let mut state = loaded_state();
    update(
        &mut state,
        AppEvent::Backend(BackendEvent::ContactUpdated(Contact {
            service_id: DAVE,
            name: None,
            profile_name: Some("alice 🌸".to_owned()),
            phone: None,
        })),
    );
    state
}

fn enter(state: &mut State) -> Vec<Effect> {
    update(state, key(KeyCode::Enter, KeyModifiers::NONE))
}

fn level(state: &State) -> Option<Level> {
    state
        .palette
        .as_ref()
        .map(crate::app::palette::Palette::level)
}

#[test]
fn people_with_the_same_name_get_a_short_tag() {
    let mut state = loaded_state();
    assert_eq!(state.person_name(ALICE), "Alice 🌸");
    state = with_second_alice();
    // Case doesn't tell them apart; the first four hex digits of the ID do.
    assert_eq!(state.person_name(ALICE), "Alice 🌸 · 0000");
    assert_eq!(state.person_name(DAVE), "alice 🌸 · abcd");
    assert_eq!(state.conversation_title(alice()), "Alice 🌸 · 0000");
    // Nobody else is tagged, and "You" stays "You".
    assert_eq!(state.person_name(BOB), "Bob");
    assert_eq!(state.person_name(ME), "You");
}

#[test]
fn a_nickname_from_the_palette_replaces_the_name_and_ends_the_clash() {
    let mut state = with_second_alice();
    update(&mut state, key(KeyCode::Char('p'), KeyModifiers::CONTROL));
    typed(&mut state, "commands");
    enter(&mut state);
    typed(&mut state, "nickname");
    enter(&mut state);
    assert_eq!(level(&state), Some(Level::People));
    typed(&mut state, "abcd");
    enter(&mut state);
    assert_eq!(level(&state), Some(Level::Nickname(DAVE)));
    // Typing doesn't filter: the query is the name.
    typed(&mut state, "Dee");
    let palette = state.palette.as_ref().unwrap();
    assert_eq!(
        palette.selected_item().map(|i| i.kind.clone()),
        Some(ItemKind::SetNickname(DAVE))
    );
    let effects = enter(&mut state);
    assert!(state.palette.is_none());
    assert_eq!(
        effects,
        vec![Effect::SaveNickname(DAVE, Some("Dee".to_owned()))]
    );
    assert_eq!(state.person_name(DAVE), "Dee");
    assert_eq!(state.person_name(ALICE), "Alice 🌸");
    assert!(state.notice.is_some());
}

#[test]
fn the_chat_menu_sets_and_removes_a_nickname() {
    let mut state = with_second_alice();
    crate::app::update::curation::perform(&mut state, alice(), ConversationAction::Nickname);
    assert_eq!(level(&state), Some(Level::Nickname(ALICE)));
    typed(&mut state, "  Ali ");
    let effects = enter(&mut state);
    assert_eq!(
        effects,
        vec![Effect::SaveNickname(ALICE, Some("Ali".to_owned()))]
    );
    assert_eq!(state.conversation_title(alice()), "Ali");
    assert_eq!(state.person_name(DAVE), "alice 🌸");

    // Opening it again starts from the nickname; "Remove" is offered.
    crate::app::update::curation::perform(&mut state, alice(), ConversationAction::Nickname);
    let palette = state.palette.as_ref().unwrap();
    assert_eq!(palette.input.text(), "Ali");
    update(&mut state, key(KeyCode::Down, KeyModifiers::NONE));
    let effects = enter(&mut state);
    assert_eq!(effects, vec![Effect::SaveNickname(ALICE, None)]);
    assert_eq!(state.person_name(ALICE), "Alice 🌸 · 0000");

    // Saving an empty name when there's none does nothing.
    crate::app::update::curation::perform(&mut state, alice(), ConversationAction::Nickname);
    assert_eq!(enter(&mut state), vec![]);
}

#[test]
fn groups_and_this_account_get_no_nickname_entry() {
    let mut state = loaded_state();
    crate::app::update::curation::menu(&mut state, climbing());
    let Some(crate::app::state::Popup::Conversation { items, .. }) = &state.popup else {
        panic!("no menu");
    };
    assert!(!items.contains(&ConversationAction::Nickname));
    crate::app::update::curation::menu(&mut state, alice());
    let Some(crate::app::state::Popup::Conversation { items, .. }) = &state.popup else {
        panic!("no menu");
    };
    assert!(items.contains(&ConversationAction::Nickname));
    assert!(!state.people().contains(&ME));
}
