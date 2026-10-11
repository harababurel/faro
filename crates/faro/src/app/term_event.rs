//! Terminal input as faro sees it: keys, mouse, paste, resize and focus.
//!
//! The same shape as crossterm's event types, trimmed to what faro uses, so
//! the shared app code doesn't depend on crossterm and also builds for the
//! web (M13). The terminal binary converts crossterm's events at the boundary
//! (`tui::input`); the web shell will make these from DOM events.

use bitflags::bitflags;

/// One terminal event.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Event {
    FocusGained,
    FocusLost,
    Key(KeyEvent),
    Mouse(MouseEvent),
    /// Bracketed paste.
    Paste(String),
    /// The new size in columns and rows.
    Resize(u16, u16),
}

/// A key press, repeat or release.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct KeyEvent {
    pub code: KeyCode,
    pub modifiers: KeyModifiers,
    pub kind: KeyEventKind,
}

impl KeyEvent {
    /// A press.
    pub const fn new(code: KeyCode, modifiers: KeyModifiers) -> Self {
        Self {
            code,
            modifiers,
            kind: KeyEventKind::Press,
        }
    }
}

impl From<KeyCode> for KeyEvent {
    fn from(code: KeyCode) -> Self {
        Self::new(code, KeyModifiers::NONE)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum KeyCode {
    Backspace,
    Enter,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    PageUp,
    PageDown,
    Tab,
    /// Shift+Tab.
    BackTab,
    Delete,
    Insert,
    /// A function key, `F(1)` to `F(24)` or so.
    F(u8),
    Char(char),
    Esc,
    /// A key faro has no name for (Caps Lock, media keys, a lone modifier…).
    /// It still counts as input: it reaches the reducer, binds to nothing.
    Other,
}

bitflags! {
    /// Held modifiers. The bits are crossterm's.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct KeyModifiers: u8 {
        const SHIFT = 0b0000_0001;
        const CONTROL = 0b0000_0010;
        const ALT = 0b0000_0100;
        const SUPER = 0b0000_1000;
        const HYPER = 0b0001_0000;
        const META = 0b0010_0000;
        const NONE = 0b0000_0000;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KeyEventKind {
    Press,
    Repeat,
    Release,
}

/// A mouse event at a cell (0-based column and row).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MouseEvent {
    pub kind: MouseEventKind,
    pub column: u16,
    pub row: u16,
    pub modifiers: KeyModifiers,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MouseEventKind {
    Down(MouseButton),
    Up(MouseButton),
    Drag(MouseButton),
    Moved,
    ScrollDown,
    ScrollUp,
    ScrollLeft,
    ScrollRight,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
}
