//! Default key bindings (DESIGN.md §7.6, D13, D36): keys to `Action`s,
//! depending on which pane has focus. M4 task 7 makes these rebindable.

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::app::state::Focus;

/// What a key (or mouse gesture) does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Quit,
    /// Ctrl+C: quits on the second press (D13).
    Interrupt,
    PreviousConversation,
    NextConversation,
    FirstConversation,
    LastConversation,
    ToggleLeftSidebar,
    ToggleRightSidebar,
    FocusNext,
    FocusPrevious,
    FocusInput,
    /// Esc: dismiss the notice; outside the input, also go back to it.
    Escape,
    /// Scroll the messages this many rows up (negative: down).
    Scroll(isize),
    PageUp,
    PageDown,
    ScrollTop,
    ScrollBottom,
    DeleteBackward,
    Insert(char),
}

/// Rows per mouse wheel notch.
pub const WHEEL_ROWS: isize = 3;

pub fn key_action(key: KeyEvent, focus: Focus) -> Option<Action> {
    if key.kind == KeyEventKind::Release {
        return None;
    }
    // Shift only changes which character arrives (and turns Tab into BackTab).
    let modifiers = key.modifiers.difference(KeyModifiers::SHIFT);
    global(key.code, modifiers).or_else(|| match focus {
        Focus::Input => input(key.code, modifiers),
        Focus::Conversations => conversations(key.code, modifiers),
        Focus::Messages => messages(key.code, modifiers),
    })
}

// Most keys do nothing, hence the catch-alls (on tuples, not enums of ours).

fn global(code: KeyCode, modifiers: KeyModifiers) -> Option<Action> {
    match (code, modifiers) {
        (KeyCode::Char('q'), KeyModifiers::CONTROL) => Some(Action::Quit),
        (KeyCode::Char('c'), KeyModifiers::CONTROL) => Some(Action::Interrupt),
        (KeyCode::Char('k' | 'K') | KeyCode::Up, KeyModifiers::ALT) => {
            Some(Action::PreviousConversation)
        }
        (KeyCode::Char('j' | 'J') | KeyCode::Down, KeyModifiers::ALT) => {
            Some(Action::NextConversation)
        }
        (KeyCode::Char('s' | 'S'), KeyModifiers::ALT) => Some(Action::ToggleLeftSidebar),
        (KeyCode::Char('i' | 'I'), KeyModifiers::ALT) => Some(Action::ToggleRightSidebar),
        (KeyCode::Tab, KeyModifiers::NONE) => Some(Action::FocusNext),
        (KeyCode::BackTab, KeyModifiers::NONE) => Some(Action::FocusPrevious),
        (KeyCode::PageUp, KeyModifiers::NONE) => Some(Action::PageUp),
        (KeyCode::PageDown, KeyModifiers::NONE) => Some(Action::PageDown),
        (KeyCode::Esc, KeyModifiers::NONE) => Some(Action::Escape),
        _ => None,
    }
}

fn input(code: KeyCode, modifiers: KeyModifiers) -> Option<Action> {
    match (code, modifiers) {
        (KeyCode::Backspace, KeyModifiers::NONE) => Some(Action::DeleteBackward),
        (KeyCode::Char(c), KeyModifiers::NONE) => Some(Action::Insert(c)),
        _ => None,
    }
}

fn conversations(code: KeyCode, modifiers: KeyModifiers) -> Option<Action> {
    match (code, modifiers) {
        (KeyCode::Up | KeyCode::Char('k'), KeyModifiers::NONE) => {
            Some(Action::PreviousConversation)
        }
        (KeyCode::Down | KeyCode::Char('j'), KeyModifiers::NONE) => Some(Action::NextConversation),
        (KeyCode::Home | KeyCode::Char('g'), KeyModifiers::NONE) => Some(Action::FirstConversation),
        (KeyCode::End | KeyCode::Char('G'), KeyModifiers::NONE) => Some(Action::LastConversation),
        (KeyCode::Enter, KeyModifiers::NONE) => Some(Action::FocusInput),
        _ => None,
    }
}

fn messages(code: KeyCode, modifiers: KeyModifiers) -> Option<Action> {
    match (code, modifiers) {
        (KeyCode::Up | KeyCode::Char('k'), KeyModifiers::NONE) => Some(Action::Scroll(1)),
        (KeyCode::Down | KeyCode::Char('j'), KeyModifiers::NONE) => Some(Action::Scroll(-1)),
        (KeyCode::Home | KeyCode::Char('g'), KeyModifiers::NONE) => Some(Action::ScrollTop),
        (KeyCode::End | KeyCode::Char('G'), KeyModifiers::NONE) => Some(Action::ScrollBottom),
        (KeyCode::Enter, KeyModifiers::NONE) => Some(Action::FocusInput),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(code: KeyCode, modifiers: KeyModifiers, focus: Focus) -> Option<Action> {
        key_action(KeyEvent::new(code, modifiers), focus)
    }

    #[test]
    fn letters_type_in_the_input_and_navigate_elsewhere() {
        let j = |focus| press(KeyCode::Char('j'), KeyModifiers::NONE, focus);
        assert_eq!(j(Focus::Input), Some(Action::Insert('j')));
        assert_eq!(j(Focus::Conversations), Some(Action::NextConversation));
        assert_eq!(j(Focus::Messages), Some(Action::Scroll(-1)));
        // Unbound letters do nothing outside the input.
        assert_eq!(
            press(KeyCode::Char('x'), KeyModifiers::NONE, Focus::Messages),
            None
        );
    }

    #[test]
    fn shift_tab_and_capitals_work() {
        assert_eq!(
            press(KeyCode::BackTab, KeyModifiers::SHIFT, Focus::Input),
            Some(Action::FocusPrevious)
        );
        assert_eq!(
            press(KeyCode::Char('G'), KeyModifiers::SHIFT, Focus::Messages),
            Some(Action::ScrollBottom)
        );
        assert_eq!(
            press(KeyCode::Char('H'), KeyModifiers::SHIFT, Focus::Input),
            Some(Action::Insert('H'))
        );
        assert_eq!(
            press(
                KeyCode::Char('J'),
                KeyModifiers::ALT | KeyModifiers::SHIFT,
                Focus::Input
            ),
            Some(Action::NextConversation)
        );
    }

    #[test]
    fn releases_are_ignored() {
        let mut key = KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE);
        key.kind = KeyEventKind::Release;
        assert_eq!(key_action(key, Focus::Input), None);
    }
}
