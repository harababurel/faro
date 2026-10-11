//! crossterm's input events as faro's own (`app::term_event`), at the edge of
//! the terminal binary. Every event converts: keys faro has no name for become
//! `KeyCode::Other`, so they still count as input (focus, catch-up).

use crossterm::event as ct;

use crate::app::term_event::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};

pub fn convert(event: ct::Event) -> Event {
    match event {
        ct::Event::FocusGained => Event::FocusGained,
        ct::Event::FocusLost => Event::FocusLost,
        ct::Event::Key(key) => Event::Key(KeyEvent {
            code: key_code(key.code),
            modifiers: modifiers(key.modifiers),
            kind: match key.kind {
                ct::KeyEventKind::Press => KeyEventKind::Press,
                ct::KeyEventKind::Repeat => KeyEventKind::Repeat,
                ct::KeyEventKind::Release => KeyEventKind::Release,
            },
        }),
        ct::Event::Mouse(mouse) => Event::Mouse(MouseEvent {
            kind: match mouse.kind {
                ct::MouseEventKind::Down(b) => MouseEventKind::Down(button(b)),
                ct::MouseEventKind::Up(b) => MouseEventKind::Up(button(b)),
                ct::MouseEventKind::Drag(b) => MouseEventKind::Drag(button(b)),
                ct::MouseEventKind::Moved => MouseEventKind::Moved,
                ct::MouseEventKind::ScrollDown => MouseEventKind::ScrollDown,
                ct::MouseEventKind::ScrollUp => MouseEventKind::ScrollUp,
                ct::MouseEventKind::ScrollLeft => MouseEventKind::ScrollLeft,
                ct::MouseEventKind::ScrollRight => MouseEventKind::ScrollRight,
            },
            column: mouse.column,
            row: mouse.row,
            modifiers: modifiers(mouse.modifiers),
        }),
        ct::Event::Paste(text) => Event::Paste(text),
        ct::Event::Resize(columns, rows) => Event::Resize(columns, rows),
    }
}

fn key_code(code: ct::KeyCode) -> KeyCode {
    match code {
        ct::KeyCode::Backspace => KeyCode::Backspace,
        ct::KeyCode::Enter => KeyCode::Enter,
        ct::KeyCode::Left => KeyCode::Left,
        ct::KeyCode::Right => KeyCode::Right,
        ct::KeyCode::Up => KeyCode::Up,
        ct::KeyCode::Down => KeyCode::Down,
        ct::KeyCode::Home => KeyCode::Home,
        ct::KeyCode::End => KeyCode::End,
        ct::KeyCode::PageUp => KeyCode::PageUp,
        ct::KeyCode::PageDown => KeyCode::PageDown,
        ct::KeyCode::Tab => KeyCode::Tab,
        ct::KeyCode::BackTab => KeyCode::BackTab,
        ct::KeyCode::Delete => KeyCode::Delete,
        ct::KeyCode::Insert => KeyCode::Insert,
        ct::KeyCode::F(n) => KeyCode::F(n),
        ct::KeyCode::Char(c) => KeyCode::Char(c),
        ct::KeyCode::Esc => KeyCode::Esc,
        ct::KeyCode::Null
        | ct::KeyCode::CapsLock
        | ct::KeyCode::ScrollLock
        | ct::KeyCode::NumLock
        | ct::KeyCode::PrintScreen
        | ct::KeyCode::Pause
        | ct::KeyCode::Menu
        | ct::KeyCode::KeypadBegin
        | ct::KeyCode::Media(_)
        | ct::KeyCode::Modifier(_) => KeyCode::Other,
    }
}

/// The bits are the same; unknown ones are kept, as crossterm would.
fn modifiers(modifiers: ct::KeyModifiers) -> KeyModifiers {
    KeyModifiers::from_bits_retain(modifiers.bits())
}

fn button(button: ct::MouseButton) -> MouseButton {
    match button {
        ct::MouseButton::Left => MouseButton::Left,
        ct::MouseButton::Right => MouseButton::Right,
        ct::MouseButton::Middle => MouseButton::Middle,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modifier_bits_match_crossterm() {
        for (theirs, ours) in [
            (ct::KeyModifiers::SHIFT, KeyModifiers::SHIFT),
            (ct::KeyModifiers::CONTROL, KeyModifiers::CONTROL),
            (ct::KeyModifiers::ALT, KeyModifiers::ALT),
            (ct::KeyModifiers::SUPER, KeyModifiers::SUPER),
            (ct::KeyModifiers::HYPER, KeyModifiers::HYPER),
            (ct::KeyModifiers::META, KeyModifiers::META),
            (ct::KeyModifiers::NONE, KeyModifiers::NONE),
        ] {
            assert_eq!(modifiers(theirs), ours);
        }
    }

    #[test]
    fn keys_convert_with_kind_and_modifiers() {
        let key = ct::KeyEvent::new_with_kind(
            ct::KeyCode::Char('p'),
            ct::KeyModifiers::CONTROL | ct::KeyModifiers::SHIFT,
            ct::KeyEventKind::Release,
        );
        assert_eq!(
            convert(ct::Event::Key(key)),
            Event::Key(KeyEvent {
                code: KeyCode::Char('p'),
                modifiers: KeyModifiers::CONTROL | KeyModifiers::SHIFT,
                kind: KeyEventKind::Release,
            })
        );
        assert_eq!(
            convert(ct::Event::Key(ct::KeyCode::CapsLock.into())),
            Event::Key(KeyEvent::from(KeyCode::Other))
        );
    }
}
