//! One key with its modifiers, as written in the config file (`"ctrl+p"`,
//! `"alt+j"`, `"G"`, `"space"`, `"shift+tab"`) and as shown in key hints
//! (`"ctrl+p"`, `"↑"`).

use std::fmt;

use crate::app::term_event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct KeyChord {
    pub code: KeyCode,
    pub modifiers: KeyModifiers,
}

/// Keys with names, in their canonical spelling first.
const NAMED_KEYS: &[(&str, KeyCode)] = &[
    ("enter", KeyCode::Enter),
    ("esc", KeyCode::Esc),
    ("tab", KeyCode::Tab),
    ("backspace", KeyCode::Backspace),
    ("delete", KeyCode::Delete),
    ("insert", KeyCode::Insert),
    ("home", KeyCode::Home),
    ("end", KeyCode::End),
    ("pgup", KeyCode::PageUp),
    ("pgdn", KeyCode::PageDown),
    ("up", KeyCode::Up),
    ("down", KeyCode::Down),
    ("left", KeyCode::Left),
    ("right", KeyCode::Right),
    ("space", KeyCode::Char(' ')),
    // Accepted when parsing, never printed.
    ("escape", KeyCode::Esc),
    ("return", KeyCode::Enter),
    ("del", KeyCode::Delete),
    ("pageup", KeyCode::PageUp),
    ("pagedown", KeyCode::PageDown),
];

/// How hints show the arrows.
const ARROWS: &[(KeyCode, &str)] = &[
    (KeyCode::Up, "↑"),
    (KeyCode::Down, "↓"),
    (KeyCode::Left, "←"),
    (KeyCode::Right, "→"),
];

impl KeyChord {
    pub const fn new(code: KeyCode, modifiers: KeyModifiers) -> Self {
        Self { code, modifiers }
    }

    /// The chord a key press is looked up as. Shift is already part of the
    /// character (`G`) and of `BackTab`, so it's dropped there; elsewhere it
    /// stays (`shift+enter`).
    pub fn from_event(key: KeyEvent) -> Self {
        Self::normalised(key.code, key.modifiers)
    }

    /// Parse the config spelling: modifiers (`ctrl`, `alt`, `shift`; also
    /// `control`, `meta`, `option`) joined to a key with `+`. Single
    /// characters are case-sensitive (`G` is `shift+g`); names aren't.
    pub fn parse(text: &str) -> Option<Self> {
        let mut modifiers = KeyModifiers::NONE;
        let mut rest = text.trim();
        // `+` alone and `ctrl++` are the plus key.
        while let Some((head, tail)) = rest.split_once('+')
            && !tail.is_empty()
        {
            modifiers |= match head.trim().to_ascii_lowercase().as_str() {
                "ctrl" | "control" => KeyModifiers::CONTROL,
                "alt" | "meta" | "option" | "opt" => KeyModifiers::ALT,
                "shift" => KeyModifiers::SHIFT,
                _ => return None,
            };
            rest = tail;
        }
        Some(Self::normalised(key_code(rest)?, modifiers))
    }

    fn normalised(code: KeyCode, mut modifiers: KeyModifiers) -> Self {
        let shift = modifiers.contains(KeyModifiers::SHIFT);
        let code = match (code, shift) {
            (KeyCode::Tab, true) => KeyCode::BackTab,
            (KeyCode::Char(c), true) => KeyCode::Char(c.to_ascii_uppercase()),
            (other, _) => other,
        };
        if matches!(code, KeyCode::Char(_) | KeyCode::BackTab) {
            modifiers.remove(KeyModifiers::SHIFT);
        }
        Self { code, modifiers }
    }

    /// `Ctrl`/`Alt` with a capital also matches the lowercase binding:
    /// whether Shift reaches us there depends on the terminal.
    pub fn lowercase(self) -> Option<Self> {
        if let KeyCode::Char(c) = self.code
            && c.is_ascii_uppercase()
            && self
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
        {
            Some(Self::new(
                KeyCode::Char(c.to_ascii_lowercase()),
                self.modifiers,
            ))
        } else {
            None
        }
    }

    /// The spelling for key hints: arrows as glyphs, otherwise as in config.
    pub fn label(self) -> String {
        let mut text = String::new();
        self.write(&mut text, true);
        text
    }

    fn write(self, out: &mut String, label: bool) {
        for (modifier, name) in [
            (KeyModifiers::CONTROL, "ctrl+"),
            (KeyModifiers::ALT, "alt+"),
            (KeyModifiers::SHIFT, "shift+"),
        ] {
            if self.modifiers.contains(modifier) {
                out.push_str(name);
            }
        }
        let arrow = ARROWS.iter().find(|(code, _)| *code == self.code);
        let named = NAMED_KEYS.iter().find(|(_, code)| *code == self.code);
        if let (true, Some((_, glyph))) = (label, arrow) {
            out.push_str(glyph);
        } else if let Some((name, _)) = named {
            out.push_str(name);
        } else if self.code == KeyCode::BackTab {
            out.push_str("shift+tab");
        } else if let KeyCode::Char(c) = self.code {
            out.push(c);
        } else if let KeyCode::F(n) = self.code {
            out.push('f');
            out.push_str(&n.to_string());
        } else {
            let name = format!("{:?}", self.code).to_ascii_lowercase();
            out.push_str(&name);
        }
    }
}

/// The config spelling (parses back to the same chord).
impl fmt::Display for KeyChord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut text = String::new();
        self.write(&mut text, false);
        f.write_str(&text)
    }
}

fn key_code(name: &str) -> Option<KeyCode> {
    let mut chars = name.chars();
    if let (Some(c), None) = (chars.next(), chars.next()) {
        return Some(KeyCode::Char(c));
    }
    let lower = name.trim().to_ascii_lowercase();
    if let Some(n) = lower
        .strip_prefix('f')
        .and_then(|n| n.parse::<u8>().ok())
        .filter(|n| (1..=12).contains(n))
    {
        return Some(KeyCode::F(n));
    }
    NAMED_KEYS
        .iter()
        .find(|(key, _)| *key == lower)
        .map(|(_, code)| *code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[expect(clippy::unnecessary_wraps, reason = "compared with `parse` results")]
    fn chord(code: KeyCode, modifiers: KeyModifiers) -> Option<KeyChord> {
        Some(KeyChord::new(code, modifiers))
    }

    #[test]
    fn parses_the_config_spelling() {
        let ctrl = KeyModifiers::CONTROL;
        let alt = KeyModifiers::ALT;
        let none = KeyModifiers::NONE;
        assert_eq!(KeyChord::parse("ctrl+p"), chord(KeyCode::Char('p'), ctrl));
        assert_eq!(KeyChord::parse("Ctrl+P"), chord(KeyCode::Char('P'), ctrl));
        assert_eq!(KeyChord::parse("alt+enter"), chord(KeyCode::Enter, alt));
        assert_eq!(KeyChord::parse("meta+Down"), chord(KeyCode::Down, alt));
        assert_eq!(KeyChord::parse("G"), chord(KeyCode::Char('G'), none));
        assert_eq!(KeyChord::parse("shift+g"), chord(KeyCode::Char('G'), none));
        assert_eq!(KeyChord::parse("shift+tab"), chord(KeyCode::BackTab, none));
        assert_eq!(
            KeyChord::parse("shift+enter"),
            chord(KeyCode::Enter, KeyModifiers::SHIFT)
        );
        assert_eq!(KeyChord::parse("space"), chord(KeyCode::Char(' '), none));
        assert_eq!(KeyChord::parse("+"), chord(KeyCode::Char('+'), none));
        assert_eq!(KeyChord::parse("ctrl++"), chord(KeyCode::Char('+'), ctrl));
        assert_eq!(KeyChord::parse("?"), chord(KeyCode::Char('?'), none));
        assert_eq!(KeyChord::parse("F1"), chord(KeyCode::F(1), none));
        assert_eq!(KeyChord::parse("PageUp"), chord(KeyCode::PageUp, none));
        assert_eq!(KeyChord::parse("hyper+x"), None);
        assert_eq!(KeyChord::parse("ctrl+"), None);
        assert_eq!(KeyChord::parse("f13"), None);
        assert_eq!(KeyChord::parse("bogus"), None);
        assert_eq!(KeyChord::parse(""), None);
    }

    #[test]
    fn prints_what_it_parses() {
        for text in [
            "ctrl+p",
            "alt+j",
            "G",
            "space",
            "shift+tab",
            "shift+enter",
            "+",
            "ctrl++",
            "f1",
            "pgup",
            "esc",
            "alt+1",
            "ctrl+alt+delete",
        ] {
            let parsed = KeyChord::parse(text).unwrap();
            assert_eq!(parsed.to_string(), text);
            assert_eq!(KeyChord::parse(&parsed.to_string()), Some(parsed));
        }
        assert_eq!(KeyChord::parse("alt+up").unwrap().label(), "alt+↑");
        assert_eq!(KeyChord::parse("pgdn").unwrap().label(), "pgdn");
    }

    #[test]
    fn events_drop_shift_where_it_is_part_of_the_key() {
        let event = |code, modifiers| KeyChord::from_event(KeyEvent::new(code, modifiers));
        assert_eq!(
            event(KeyCode::Char('G'), KeyModifiers::SHIFT),
            KeyChord::new(KeyCode::Char('G'), KeyModifiers::NONE)
        );
        assert_eq!(
            event(KeyCode::BackTab, KeyModifiers::SHIFT),
            KeyChord::new(KeyCode::BackTab, KeyModifiers::NONE)
        );
        assert_eq!(
            event(KeyCode::Enter, KeyModifiers::SHIFT),
            KeyChord::new(KeyCode::Enter, KeyModifiers::SHIFT)
        );
        let alt_j = event(KeyCode::Char('J'), KeyModifiers::ALT | KeyModifiers::SHIFT);
        assert_eq!(
            alt_j.lowercase(),
            Some(KeyChord::new(KeyCode::Char('j'), KeyModifiers::ALT))
        );
        assert_eq!(
            event(KeyCode::Char('J'), KeyModifiers::NONE).lowercase(),
            None
        );
    }
}
