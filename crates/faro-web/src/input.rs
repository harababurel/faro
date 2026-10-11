//! Browser input as faro's terminal events (D114). Pure functions over the
//! DOM events' fields, so they're tested natively; `dom.rs` reads the fields.

use faro_app::app::term_event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEventKind};

/// The modifier flags of a DOM keyboard, mouse or wheel event.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "mirrors the DOM's four modifier flags"
)]
pub struct Held {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub meta: bool,
}

impl Held {
    pub fn modifiers(self) -> KeyModifiers {
        let mut modifiers = KeyModifiers::NONE;
        modifiers.set(KeyModifiers::SHIFT, self.shift);
        modifiers.set(KeyModifiers::CONTROL, self.ctrl);
        modifiers.set(KeyModifiers::ALT, self.alt);
        modifiers.set(KeyModifiers::SUPER, self.meta);
        modifiers
    }
}

/// Who gets a key press.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    /// faro: the page stops the browser acting on it.
    Faro(KeyEvent),
    /// The browser, untouched: its own shortcuts, paste (which arrives as a
    /// `paste` event), and keys faro has no use for (a lone modifier, a dead
    /// key).
    Browser,
}

/// Where a `keydown` with these `key` and `code` values goes.
pub fn route(key: &str, code: &str, held: Held) -> Route {
    if browser_keeps(key, code, held) {
        return Route::Browser;
    }
    key_code(key, code, held).map_or(Route::Browser, |code| {
        Route::Faro(KeyEvent::new(code, held.modifiers()))
    })
}

/// Chords the browser keeps: `Cmd`/`Win` shortcuts, paste, reload, zoom,
/// full screen and the developer tools. `Ctrl+W`, `Ctrl+T`, `Ctrl+N` and
/// `Ctrl+Tab` never reach a page anyway.
fn browser_keeps(key: &str, code: &str, held: Held) -> bool {
    let Held {
        shift, ctrl, meta, ..
    } = held;
    let letter = code.strip_prefix("Key").unwrap_or("");
    meta || matches!(key, "F5" | "F11" | "F12")
        || (shift && key == "Insert")
        || (ctrl && letter == "V")
        || (ctrl && shift && matches!(letter, "I" | "J" | "C" | "R"))
        || (ctrl && matches!(key, "+" | "=" | "-" | "0"))
        || (ctrl && matches!(key, "w" | "t" | "n" | "Tab"))
}

/// faro's name for the key, if it has one. Alt chords (and Ctrl chords
/// whose character isn't ASCII, as on a Cyrillic layout) are read from the
/// key's position (`code`), because macOS turns Option+letter into another
/// character (`Option+J` is `∆`).
fn key_code(key: &str, code: &str, held: Held) -> Option<KeyCode> {
    if let Some(named) = named(key, held.shift) {
        return Some(named);
    }
    let mut chars = key.chars();
    let typed = match (chars.next(), chars.next()) {
        (Some(c), None) => Some(c),
        _ => None,
    };
    let by_position = held.alt || (held.ctrl && typed.is_some_and(|c| !c.is_ascii()));
    if by_position && let Some(c) = position_char(code, held.shift) {
        return Some(KeyCode::Char(c));
    }
    // Multi-character values are names faro doesn't use ("Shift", "Dead",
    // "CapsLock", "Unidentified", "Process"…).
    typed.map(KeyCode::Char)
}

fn named(key: &str, shift: bool) -> Option<KeyCode> {
    Some(match key {
        "Enter" => KeyCode::Enter,
        "Backspace" => KeyCode::Backspace,
        "Tab" if shift => KeyCode::BackTab,
        "Tab" => KeyCode::Tab,
        "Escape" | "Esc" => KeyCode::Esc,
        "ArrowLeft" | "Left" => KeyCode::Left,
        "ArrowRight" | "Right" => KeyCode::Right,
        "ArrowUp" | "Up" => KeyCode::Up,
        "ArrowDown" | "Down" => KeyCode::Down,
        "Home" => KeyCode::Home,
        "End" => KeyCode::End,
        "PageUp" => KeyCode::PageUp,
        "PageDown" => KeyCode::PageDown,
        "Delete" | "Del" => KeyCode::Delete,
        "Insert" => KeyCode::Insert,
        _ => return function_key(key),
    })
}

fn function_key(key: &str) -> Option<KeyCode> {
    let number: u8 = key.strip_prefix('F')?.parse().ok()?;
    (1..=24).contains(&number).then_some(KeyCode::F(number))
}

/// The character a US layout puts at `code`.
fn position_char(code: &str, shift: bool) -> Option<char> {
    if let Some(letter) = code.strip_prefix("Key") {
        let c = single(letter)?;
        return c
            .is_ascii_uppercase()
            .then(|| if shift { c } else { c.to_ascii_lowercase() });
    }
    if let Some(digit) = code.strip_prefix("Digit") {
        let c = single(digit)?;
        return c.is_ascii_digit().then_some(c);
    }
    Some(match code {
        "Space" => ' ',
        "Minus" => '-',
        "Equal" => '=',
        "BracketLeft" => '[',
        "BracketRight" => ']',
        "Backslash" => '\\',
        "Semicolon" => ';',
        "Quote" => '\'',
        "Backquote" => '`',
        "Comma" => ',',
        "Period" => '.',
        "Slash" => '/',
        _ => return None,
    })
}

fn single(text: &str) -> Option<char> {
    let mut chars = text.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) => Some(c),
        _ => None,
    }
}

/// A DOM `MouseEvent.button`.
pub const fn button(button: i16) -> Option<MouseButton> {
    match button {
        0 => Some(MouseButton::Left),
        1 => Some(MouseButton::Middle),
        2 => Some(MouseButton::Right),
        _ => None,
    }
}

/// The button a move is dragging, from `MouseEvent.buttons` (a bit set).
pub const fn dragging(buttons: u16) -> Option<MouseButton> {
    if buttons & 1 != 0 {
        Some(MouseButton::Left)
    } else if buttons & 2 != 0 {
        Some(MouseButton::Right)
    } else if buttons & 4 != 0 {
        Some(MouseButton::Middle)
    } else {
        None
    }
}

/// The cell under a point, in CSS pixels from the canvas's top left.
pub fn cell_at(x: f64, y: f64, cell: (f64, f64), grid: (u16, u16)) -> (u16, u16) {
    (axis(x, cell.0, grid.0), axis(y, cell.1, grid.1))
}

fn axis(position: f64, size: f64, count: u16) -> u16 {
    if size <= 0.0 || count == 0 || position.is_nan() {
        return 0;
    }
    let index = (position / size).floor().clamp(0.0, f64::from(count - 1));
    // In 0..count, so it fits.
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "clamped to 0..count above"
    )]
    let index = index as u16;
    index
}

/// Wheel movement turned into scroll steps: one per notch of a mouse wheel,
/// and a steady rate for a trackpad's many small deltas.
#[derive(Debug, Default)]
pub struct Wheel {
    pending: f64,
}

/// CSS pixels of wheel movement per scroll step (a Chrome notch is 100).
const STEP: f64 = 50.0;

impl Wheel {
    /// Feed a `wheel` event's `deltaY` in `deltaMode` (0 pixels, 1 lines,
    /// 2 pages); how many steps to scroll, negative for up.
    pub fn feed(&mut self, delta: f64, mode: u32, line: f64) -> i32 {
        let pixels = match mode {
            1 => delta * line,
            2 => delta * line * 20.0,
            _ => delta,
        };
        if !pixels.is_finite() {
            return 0;
        }
        // A new direction starts afresh.
        if pixels.signum() != self.pending.signum() {
            self.pending = 0.0;
        }
        self.pending += pixels;
        let steps = (self.pending / STEP).trunc();
        self.pending -= steps * STEP;
        #[expect(
            clippy::cast_possible_truncation,
            reason = "a handful of steps per event"
        )]
        let steps = steps.clamp(-100.0, 100.0) as i32;
        steps
    }
}

/// The scroll event for a step direction.
pub const fn scroll(down: bool) -> MouseEventKind {
    if down {
        MouseEventKind::ScrollDown
    } else {
        MouseEventKind::ScrollUp
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(key: &str, code: &str, held: Held) -> Option<KeyEvent> {
        match route(key, code, held) {
            Route::Faro(event) => Some(event),
            Route::Browser => None,
        }
    }

    const NONE: Held = Held {
        shift: false,
        ctrl: false,
        alt: false,
        meta: false,
    };
    const SHIFT: Held = Held {
        shift: true,
        ..NONE
    };
    const CTRL: Held = Held { ctrl: true, ..NONE };
    const ALT: Held = Held { alt: true, ..NONE };

    #[test]
    fn characters_and_named_keys_read_like_crossterm() {
        assert_eq!(
            press("a", "KeyA", NONE),
            Some(KeyEvent::from(KeyCode::Char('a')))
        );
        assert_eq!(
            press("G", "KeyG", SHIFT),
            Some(KeyEvent::new(KeyCode::Char('G'), KeyModifiers::SHIFT))
        );
        assert_eq!(
            press(" ", "Space", NONE),
            Some(KeyEvent::from(KeyCode::Char(' ')))
        );
        assert_eq!(
            press("Tab", "Tab", SHIFT),
            Some(KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT))
        );
        assert_eq!(
            press("Escape", "Escape", NONE),
            Some(KeyEvent::from(KeyCode::Esc))
        );
        assert_eq!(press("F2", "F2", NONE), Some(KeyEvent::from(KeyCode::F(2))));
        assert_eq!(
            press("é", "Digit2", NONE),
            Some(KeyEvent::from(KeyCode::Char('é'))),
            "a character typed on another layout is kept"
        );
    }

    #[test]
    fn alt_chords_come_from_the_key_position() {
        // macOS: Option+J types ∆.
        assert_eq!(
            press("∆", "KeyJ", ALT),
            Some(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::ALT))
        );
        assert_eq!(
            press("Dead", "KeyN", ALT),
            Some(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::ALT))
        );
        assert_eq!(
            press("ArrowLeft", "ArrowLeft", ALT),
            Some(KeyEvent::new(KeyCode::Left, KeyModifiers::ALT))
        );
        // A Cyrillic layout: Ctrl+К is Ctrl+R's key.
        assert_eq!(
            press("к", "KeyR", CTRL),
            Some(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL))
        );
    }

    #[test]
    fn the_browser_keeps_its_own_chords_and_keys_faro_ignores() {
        let meta = Held { meta: true, ..NONE };
        assert_eq!(route("c", "KeyC", meta), Route::Browser);
        assert_eq!(route("v", "KeyV", CTRL), Route::Browser, "paste");
        assert_eq!(route("Insert", "Insert", SHIFT), Route::Browser);
        assert_eq!(route("F5", "F5", NONE), Route::Browser);
        assert_eq!(route("Shift", "ShiftLeft", SHIFT), Route::Browser);
        assert_eq!(route("Dead", "Quote", NONE), Route::Browser);
        assert!(matches!(route("p", "KeyP", CTRL), Route::Faro(_)));
        assert!(matches!(route("c", "KeyC", CTRL), Route::Faro(_)));
    }

    #[test]
    fn points_map_to_cells_inside_the_grid() {
        let cell = (8.0, 17.0);
        assert_eq!(cell_at(0.0, 0.0, cell, (120, 34)), (0, 0));
        assert_eq!(cell_at(15.9, 17.0, cell, (120, 34)), (1, 1));
        assert_eq!(cell_at(-3.0, 9999.0, cell, (120, 34)), (0, 33));
        assert_eq!(cell_at(5.0, 5.0, cell, (0, 0)), (0, 0));
    }

    #[test]
    fn wheel_notches_and_trackpads_scroll_steadily() {
        let mut wheel = Wheel::default();
        assert_eq!(wheel.feed(100.0, 0, 16.0), 2, "a Chrome notch");
        assert_eq!(wheel.feed(3.0, 1, 16.0), 0, "48 px: not yet");
        assert_eq!(wheel.feed(3.0, 1, 16.0), 1, "96 px with the rest");
        assert_eq!(wheel.feed(-10.0, 0, 16.0), 0, "turning drops the rest");
        assert_eq!(wheel.feed(-45.0, 0, 16.0), -1);
        assert_eq!(wheel.feed(f64::NAN, 0, 16.0), 0);
    }

    #[test]
    fn buttons_map() {
        assert_eq!(button(0), Some(MouseButton::Left));
        assert_eq!(button(2), Some(MouseButton::Right));
        assert_eq!(button(3), None);
        assert_eq!(dragging(1), Some(MouseButton::Left));
        assert_eq!(dragging(0), None);
    }
}
