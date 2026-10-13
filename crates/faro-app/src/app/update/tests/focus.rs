//! The `Escape` chain and the `Effect::ReleaseFocus` signal (WEBSITE.md §4.3):
//! Esc dismisses one thing at a time, and only when nothing is left to close
//! (nothing open, no notice) does it tell the host it may release focus.

use std::path::PathBuf;

use crate::app::state::{Notice, StagedAttachment};

use super::*;

fn esc() -> AppEvent {
    key(KeyCode::Esc, KeyModifiers::NONE)
}

fn released(effects: &[Effect]) -> bool {
    effects.contains(&Effect::ReleaseFocus)
}

#[test]
fn escape_with_nothing_open_and_no_notice_releases_focus() {
    let mut state = State::new(ME, fixtures::contacts(), fixtures::groups());
    // Nothing loaded, nothing open, no notice: the whole chain is empty.
    let effects = update(&mut state, esc());
    assert_eq!(effects, vec![Effect::ReleaseFocus]);
}

#[test]
fn a_notice_consumes_one_escape_before_the_release() {
    let mut state = State::new(ME, fixtures::contacts(), fixtures::groups());
    state.notice = Some(Notice::info("still here"));
    // Clearing the notice is a step of its own, not a release.
    let effects = update(&mut state, esc());
    assert!(!released(&effects), "the notice was the thing to dismiss");
    assert!(state.notice.is_none());
    // Now nothing is left.
    assert_eq!(update(&mut state, esc()), vec![Effect::ReleaseFocus]);
}

#[test]
fn the_palette_consumes_escape() {
    let mut state = loaded_state();
    update(&mut state, key(KeyCode::Char('p'), KeyModifiers::CONTROL));
    assert!(state.palette.is_some());
    let effects = update(&mut state, esc());
    assert!(state.palette.is_none(), "Esc closed the palette");
    assert!(!released(&effects), "closing the palette is not a release");
}

#[test]
fn a_popup_consumes_escape() {
    let mut state = loaded_state();
    update(&mut state, key(KeyCode::Up, KeyModifiers::NONE)); // focus Messages, select
    update(&mut state, key(KeyCode::Enter, KeyModifiers::NONE)); // open the message menu
    assert!(state.popup.is_some());
    let effects = update(&mut state, esc());
    assert_eq!(state.popup, None, "Esc closed the menu");
    assert!(!released(&effects), "closing a popup is not a release");
}

#[test]
fn the_pane_step_then_close_then_release() {
    let mut state = loaded_state();
    update(&mut state, key(KeyCode::Up, KeyModifiers::NONE)); // Messages focus
    // A pane step: back to the input, chat still open.
    let effects = update(&mut state, esc());
    assert!(!released(&effects));
    assert_eq!(state.focus, Focus::Input);
    assert!(state.open.is_some());
    // Next Esc closes the conversation.
    let effects = update(&mut state, esc());
    assert!(!released(&effects));
    assert!(state.open.is_none());
    // Then nothing is left.
    assert_eq!(update(&mut state, esc()), vec![Effect::ReleaseFocus]);
}

#[test]
fn closing_a_chat_keeps_its_draft_and_does_not_release() {
    let mut state = loaded_state();
    typed(&mut state, "keep me");
    let effects = update(&mut state, esc()); // closes the chat
    assert!(state.open.is_none());
    assert!(
        !released(&effects),
        "closing the chat is a step, not a release"
    );
    assert_eq!(
        state
            .summary(climbing())
            .and_then(|s| s.state.draft.as_ref())
            .map(faro_core::domain::MessageBody::expose),
        Some("keep me"),
        "the draft is kept"
    );
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::SaveConversationState(id, _) if *id == climbing())),
        "the draft is saved"
    );
    // Only the next Esc, with nothing open, releases.
    assert_eq!(update(&mut state, esc()), vec![Effect::ReleaseFocus]);
}

#[test]
fn a_reply_consumes_escape() {
    let mut state = loaded_state();
    update(&mut state, key(KeyCode::Up, KeyModifiers::NONE));
    press(&mut state, 'r');
    assert!(state.open.as_ref().unwrap().reply_to.is_some());
    let effects = update(&mut state, esc());
    assert!(state.open.as_ref().unwrap().reply_to.is_none());
    assert!(state.open.is_some());
    assert!(!released(&effects));
}

#[test]
fn a_staged_attachment_consumes_escape() {
    let mut state = loaded_state();
    let staged = StagedAttachment {
        path: PathBuf::from("/home/user/Downloads/photo.png"),
        name: "photo.png".into(),
        content_type: "image/png".into(),
        size: 16_384,
        width: Some(800),
        height: Some(600),
    };
    update(&mut state, AppEvent::FileStaged(Ok(staged)));
    assert!(state.open.as_ref().unwrap().staged_attachment.is_some());
    let effects = update(&mut state, esc());
    assert!(state.open.as_ref().unwrap().staged_attachment.is_none());
    assert!(state.open.is_some());
    assert!(!released(&effects));
}

#[test]
fn an_edit_in_progress_consumes_escape() {
    let mut state = loaded_state();
    update(&mut state, key(KeyCode::Up, KeyModifiers::NONE));
    press(&mut state, 'k'); // step to our own "On my way"
    assert_eq!(selected_ref(&state).map(|r| r.sender), Some(ME));
    press(&mut state, 'e'); // start editing
    assert!(state.open.as_ref().unwrap().editing.is_some());
    let effects = update(&mut state, esc());
    assert!(state.open.as_ref().unwrap().editing.is_none());
    assert!(state.open.is_some());
    assert!(!released(&effects));
}

#[test]
fn an_autocomplete_popup_consumes_escape() {
    let mut state = loaded_state();
    typed(&mut state, ":smi");
    assert!(
        !state.shortcode_candidates().is_empty(),
        "a shortcode is up"
    );
    let effects = update(&mut state, esc());
    assert!(
        state.shortcode_candidates().is_empty(),
        "Esc dismissed the picker"
    );
    assert!(state.open.is_some());
    assert!(!released(&effects));
}
