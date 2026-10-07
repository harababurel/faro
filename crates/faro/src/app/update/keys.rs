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
    PageUp,
    PageDown,
    /// Send the input (Enter).
    Send,
    Edit(Edit),
    /// Message selection (D40): move to an older / newer message.
    SelectOlder,
    SelectNewer,
    SelectOldest,
    SelectNewest,
    /// What the selected message offers (`Enter`).
    MessageMenu,
    Reply,
    ReactPicker,
    Copy,
    OpenLink,
    Popup(PopupKey),
}

/// Keys while a menu or picker is open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PopupKey {
    Previous,
    Next,
    Choose,
    Close,
    /// A shortcut letter or digit.
    Char(char),
}

/// Changes to the input (M4 task 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edit {
    Insert(char),
    NewLine,
    DeleteBackward,
    DeleteForward,
    DeleteWordBackward,
    DeleteWordForward,
    DeleteToLineStart,
    DeleteToLineEnd,
    Left,
    Right,
    WordLeft,
    WordRight,
    LineStart,
    LineEnd,
    Up,
    Down,
}

/// Rows per mouse wheel notch.
pub const WHEEL_ROWS: isize = 3;

pub fn key_action(key: KeyEvent, focus: Focus) -> Option<Action> {
    if key.kind == KeyEventKind::Release {
        return None;
    }
    // Shift+Enter is only distinguishable with the kitty keyboard protocol.
    if focus == Focus::Input && key.code == KeyCode::Enter && key.modifiers == KeyModifiers::SHIFT {
        return Some(Action::Edit(Edit::NewLine));
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

/// Editing keys: arrows and Home/End, plus the readline/Emacs keys that work in
/// shells (Ctrl+A/E/U/K/W, Alt+B/F/D). Alt+arrows and Ctrl+arrows move by
/// word. Newline: Alt+Enter or Ctrl+J everywhere, Shift+Enter with the kitty
/// keyboard protocol.
fn input(code: KeyCode, modifiers: KeyModifiers) -> Option<Action> {
    const NONE: KeyModifiers = KeyModifiers::NONE;
    const CTRL: KeyModifiers = KeyModifiers::CONTROL;
    const ALT: KeyModifiers = KeyModifiers::ALT;
    let edit = match (code, modifiers) {
        (KeyCode::Enter, NONE) => return Some(Action::Send),
        (KeyCode::Char(c), NONE) => Edit::Insert(c),
        (KeyCode::Enter, ALT) | (KeyCode::Char('j'), CTRL) => Edit::NewLine,
        (KeyCode::Backspace, NONE) => Edit::DeleteBackward,
        (KeyCode::Delete, NONE) => Edit::DeleteForward,
        (KeyCode::Backspace, ALT | CTRL) | (KeyCode::Char('w'), CTRL) => Edit::DeleteWordBackward,
        (KeyCode::Delete, CTRL) | (KeyCode::Char('d'), ALT) => Edit::DeleteWordForward,
        (KeyCode::Char('u'), CTRL) => Edit::DeleteToLineStart,
        (KeyCode::Char('k'), CTRL) => Edit::DeleteToLineEnd,
        (KeyCode::Left, NONE) => Edit::Left,
        (KeyCode::Right, NONE) => Edit::Right,
        (KeyCode::Left, ALT | CTRL) | (KeyCode::Char('b'), ALT) => Edit::WordLeft,
        (KeyCode::Right, ALT | CTRL) | (KeyCode::Char('f'), ALT) => Edit::WordRight,
        (KeyCode::Home, NONE) | (KeyCode::Char('a'), CTRL) => Edit::LineStart,
        (KeyCode::End, NONE) | (KeyCode::Char('e'), CTRL) => Edit::LineEnd,
        (KeyCode::Up, NONE) => Edit::Up,
        (KeyCode::Down, NONE) => Edit::Down,
        _ => return None,
    };
    Some(Action::Edit(edit))
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

/// The message pane moves a selection (D40); keys act on the selected message.
fn messages(code: KeyCode, modifiers: KeyModifiers) -> Option<Action> {
    const NONE: KeyModifiers = KeyModifiers::NONE;
    let action = match (code, modifiers) {
        (KeyCode::Up | KeyCode::Char('k'), NONE) => Action::SelectOlder,
        (KeyCode::Down | KeyCode::Char('j'), NONE) => Action::SelectNewer,
        (KeyCode::Home | KeyCode::Char('g'), NONE) => Action::SelectOldest,
        (KeyCode::End | KeyCode::Char('G'), NONE) => Action::SelectNewest,
        (KeyCode::Enter, NONE) => Action::MessageMenu,
        (KeyCode::Char('r'), NONE) => Action::Reply,
        (KeyCode::Char('+'), NONE) => Action::ReactPicker,
        (KeyCode::Char('y'), NONE) => Action::Copy,
        (KeyCode::Char('o'), NONE) => Action::OpenLink,
        _ => return None,
    };
    Some(action)
}

/// While a popup is open it takes every key except quitting.
pub fn popup_action(key: KeyEvent) -> Option<Action> {
    if key.kind == KeyEventKind::Release {
        return None;
    }
    let modifiers = key.modifiers.difference(KeyModifiers::SHIFT);
    let popup = match (key.code, modifiers) {
        (KeyCode::Char('q' | 'c'), KeyModifiers::CONTROL) => return global(key.code, modifiers),
        (KeyCode::Up | KeyCode::Left | KeyCode::BackTab, _)
        | (KeyCode::Char('k' | 'h'), KeyModifiers::NONE)
        | (KeyCode::Char('p'), KeyModifiers::CONTROL) => PopupKey::Previous,
        (KeyCode::Down | KeyCode::Right | KeyCode::Tab, _)
        | (KeyCode::Char('j' | 'l'), KeyModifiers::NONE)
        | (KeyCode::Char('n'), KeyModifiers::CONTROL) => PopupKey::Next,
        (KeyCode::Enter, _) => PopupKey::Choose,
        (KeyCode::Esc, _) | (KeyCode::Char('q'), KeyModifiers::NONE) => PopupKey::Close,
        (KeyCode::Char(c), KeyModifiers::NONE) => PopupKey::Char(c),
        _ => return None,
    };
    Some(Action::Popup(popup))
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
        assert_eq!(j(Focus::Input), Some(Action::Edit(Edit::Insert('j'))));
        assert_eq!(j(Focus::Conversations), Some(Action::NextConversation));
        assert_eq!(j(Focus::Messages), Some(Action::SelectNewer));
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
            Some(Action::SelectNewest)
        );
        assert_eq!(
            press(KeyCode::Char('H'), KeyModifiers::SHIFT, Focus::Input),
            Some(Action::Edit(Edit::Insert('H')))
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
    fn editing_keys() {
        let input = |code, modifiers| press(code, modifiers, Focus::Input);
        let edit = |e| Some(Action::Edit(e));
        assert_eq!(
            input(KeyCode::Enter, KeyModifiers::NONE),
            Some(Action::Send)
        );
        assert_eq!(
            input(KeyCode::Enter, KeyModifiers::ALT),
            edit(Edit::NewLine)
        );
        assert_eq!(
            input(KeyCode::Enter, KeyModifiers::SHIFT),
            edit(Edit::NewLine)
        );
        assert_eq!(
            input(KeyCode::Char('j'), KeyModifiers::CONTROL),
            edit(Edit::NewLine)
        );
        assert_eq!(
            input(KeyCode::Backspace, KeyModifiers::ALT),
            edit(Edit::DeleteWordBackward)
        );
        assert_eq!(
            input(KeyCode::Char('w'), KeyModifiers::CONTROL),
            edit(Edit::DeleteWordBackward)
        );
        assert_eq!(
            input(KeyCode::Char('b'), KeyModifiers::ALT),
            edit(Edit::WordLeft)
        );
        assert_eq!(
            input(KeyCode::Left, KeyModifiers::CONTROL),
            edit(Edit::WordLeft)
        );
        // Alt+J/K stay conversation switching, even in the input.
        assert_eq!(
            input(KeyCode::Char('k'), KeyModifiers::ALT),
            Some(Action::PreviousConversation)
        );
        // Ctrl+K is "delete to end of line" in the input.
        assert_eq!(
            input(KeyCode::Char('k'), KeyModifiers::CONTROL),
            edit(Edit::DeleteToLineEnd)
        );
    }

    #[test]
    fn popups_take_keys_but_not_quitting() {
        let popup = |code, modifiers| popup_action(KeyEvent::new(code, modifiers));
        assert_eq!(
            popup(KeyCode::Char('j'), KeyModifiers::NONE),
            Some(Action::Popup(PopupKey::Next))
        );
        assert_eq!(
            popup(KeyCode::Char('3'), KeyModifiers::NONE),
            Some(Action::Popup(PopupKey::Char('3')))
        );
        assert_eq!(
            popup(KeyCode::Char('+'), KeyModifiers::SHIFT),
            Some(Action::Popup(PopupKey::Char('+')))
        );
        assert_eq!(
            popup(KeyCode::Esc, KeyModifiers::NONE),
            Some(Action::Popup(PopupKey::Close))
        );
        assert_eq!(
            popup(KeyCode::Char('q'), KeyModifiers::CONTROL),
            Some(Action::Quit)
        );
        assert_eq!(popup(KeyCode::Char('j'), KeyModifiers::ALT), None);
    }

    #[test]
    fn releases_are_ignored() {
        let mut key = KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE);
        key.kind = KeyEventKind::Release;
        assert_eq!(key_action(key, Focus::Input), None);
    }
}
