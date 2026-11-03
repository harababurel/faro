//! Resizing the conversation list (D156): dragging its border, keys, and
//! the palette's widths.

use faro_core::config::{ConfigValue, SettingEdit, key};

use super::*;

fn saved_width(width: u32) -> Effect {
    Effect::SaveSetting(SettingEdit {
        path: key::SIDEBAR_WIDTH,
        value: ConfigValue::Uint(width),
    })
}

/// A list 30 wide on a 20-row screen with room for up to 60.
fn laid_out() -> State {
    let mut state = loaded_state();
    state.geometry.sidebar_border = Some(Rect::new(0, 0, 30, 20));
    state.geometry.sidebar_max = 60;
    state
}

#[test]
fn dragging_the_border_resizes_live_and_saves_once_on_release() {
    let mut state = laid_out();
    // The list's right border is column 29.
    let effects = update(
        &mut state,
        mouse(MouseEventKind::Down(MouseButton::Left), 29, 5),
    );
    assert_eq!(effects, vec![]);
    assert_eq!(state.resize.dragging, Some(30));

    let effects = update(
        &mut state,
        mouse(MouseEventKind::Drag(MouseButton::Left), 39, 6),
    );
    assert_eq!(effects, vec![]);
    assert_eq!(
        state.sidebar_width(),
        40,
        "drawn at the new width while dragging"
    );
    assert_eq!(state.config.ui.sidebar_width, 30, "not saved yet");

    let effects = update(
        &mut state,
        mouse(MouseEventKind::Up(MouseButton::Left), 39, 6),
    );
    assert_eq!(effects, vec![saved_width(40)]);
    assert_eq!(state.config.ui.sidebar_width, 40);
    assert_eq!(state.resize.dragging, None);
}

#[test]
fn a_drag_stays_between_the_narrowest_and_what_the_terminal_allows() {
    let mut state = laid_out();
    // The chat's left border (column 30) is a handle too.
    update(
        &mut state,
        mouse(MouseEventKind::Down(MouseButton::Left), 30, 5),
    );
    update(
        &mut state,
        mouse(MouseEventKind::Drag(MouseButton::Left), 99, 5),
    );
    assert_eq!(state.sidebar_width(), 60);
    update(
        &mut state,
        mouse(MouseEventKind::Drag(MouseButton::Left), 2, 5),
    );
    assert_eq!(state.sidebar_width(), 20);
    let effects = update(
        &mut state,
        mouse(MouseEventKind::Up(MouseButton::Left), 2, 5),
    );
    assert_eq!(effects, vec![saved_width(20)]);
}

#[test]
fn a_click_on_the_border_or_elsewhere_saves_nothing() {
    let mut state = laid_out();
    update(
        &mut state,
        mouse(MouseEventKind::Down(MouseButton::Left), 29, 5),
    );
    let effects = update(
        &mut state,
        mouse(MouseEventKind::Up(MouseButton::Left), 29, 5),
    );
    assert_eq!(effects, vec![]);
    assert_eq!(state.config.ui.sidebar_width, 30);

    // Inside the list: an ordinary click, not a resize.
    update(
        &mut state,
        mouse(MouseEventKind::Down(MouseButton::Left), 10, 5),
    );
    assert_eq!(state.resize.dragging, None);
    update(
        &mut state,
        mouse(MouseEventKind::Drag(MouseButton::Left), 40, 5),
    );
    assert_eq!(state.sidebar_width(), 30);
}

#[test]
fn hovering_the_border_lights_it() {
    let mut state = laid_out();
    update(&mut state, mouse(MouseEventKind::Moved, 29, 3));
    assert!(state.resize.hover);
    update(&mut state, mouse(MouseEventKind::Moved, 45, 3));
    assert!(!state.resize.hover);
}

#[test]
fn keys_step_the_width_and_stop_at_the_ends() {
    let mut state = laid_out();
    let effects = update(&mut state, key(KeyCode::Char('.'), KeyModifiers::ALT));
    assert_eq!(effects, vec![saved_width(32)]);
    assert_eq!(state.config.ui.sidebar_width, 32);

    state.geometry.sidebar_border = Some(Rect::new(0, 0, 20, 20));
    let effects = update(&mut state, key(KeyCode::Char(','), KeyModifiers::ALT));
    assert_eq!(effects, vec![]);
    assert!(
        state.notice.as_ref().unwrap().text.contains("narrowest"),
        "{:?}",
        state.notice
    );

    state.geometry.sidebar_border = None;
    update(&mut state, key(KeyCode::Char('.'), KeyModifiers::ALT));
    assert!(state.notice.as_ref().unwrap().text.contains("hidden"));
}
