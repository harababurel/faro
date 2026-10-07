//! Key bindings (M4 task 7, D43): what each key does in each pane, as data.
//!
//! The defaults (DESIGN.md §7.6) are a table in `defaults.rs`; every
//! bindable `Action` has a stable name (`conversation.next`) so the config
//! file (M5) can rebind keys per context with [`Keymap::apply`]. Lookup tries
//! the focused pane's bindings, then the global ones, then (in the input)
//! types the character. Popups keep fixed keys (`popup_action`).

mod chord;
mod defaults;
#[cfg(test)]
mod tests;

use std::fmt;

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

pub use self::chord::KeyChord;
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
    /// `Alt+1`…`Alt+9`: open the Nth pinned conversation.
    JumpToPinned(u8),
    /// Curate the conversation highlighted in the sidebar (M4 task 5, D41).
    Curate(Curate),
    /// `Ctrl+P` (M4 task 8, D44).
    OpenPalette,
    /// `F1`, `?`: the palette listing the keybindings.
    Help,
    /// `Ctrl+L`: clear the screen and draw everything again.
    Redraw,
    Palette(PaletteKey),
}

/// Keys while the palette is open: moving through the results, editing the
/// query, running the chosen one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaletteKey {
    Previous,
    Next,
    PageUp,
    PageDown,
    Choose,
    Close,
    Edit(Edit),
}

/// Sidebar keys for curating the list (D41). The toggles resolve to a
/// `ConversationAction` against the conversation's current state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Curate {
    Menu,
    Pin,
    MoveUp,
    MoveDown,
    Mute,
    Hide,
    Archive,
    Allow,
    AllowlistMode,
    /// Between the chats and the archive.
    SwitchView,
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

/// Where a binding applies: everywhere, or while a pane has focus. The names
/// are the config file's `[keybindings.<context>]` tables.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Context {
    Global,
    Input,
    Conversations,
    Messages,
}

#[cfg_attr(
    not(test),
    expect(dead_code, reason = "the config file (M5) rebinds keys through this")
)]
impl Context {
    pub const ALL: [Self; 4] = [
        Self::Global,
        Self::Input,
        Self::Conversations,
        Self::Messages,
    ];

    pub const fn name(self) -> &'static str {
        match self {
            Self::Global => "global",
            Self::Input => "input",
            Self::Conversations => "conversations",
            Self::Messages => "messages",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.name() == name)
    }
}

impl From<Focus> for Context {
    fn from(focus: Focus) -> Self {
        match focus {
            Focus::Input => Self::Input,
            Focus::Conversations => Self::Conversations,
            Focus::Messages => Self::Messages,
        }
    }
}

#[cfg_attr(
    not(test),
    expect(dead_code, reason = "the config file (M5) rebinds keys through this")
)]
impl Action {
    /// The config name, for actions that can be bound (not typing a
    /// character, nor popup keys).
    pub fn name(self) -> Option<&'static str> {
        defaults::NAMES
            .iter()
            .find(|(_, _, action)| *action == self)
            .map(|(name, _, _)| *name)
    }

    pub fn parse(name: &str) -> Option<Self> {
        defaults::NAMES
            .iter()
            .find(|(n, _, _)| *n == name)
            .map(|(_, _, action)| *action)
    }
}

impl Action {
    /// What the keybinding list calls it.
    pub fn title(self) -> Option<&'static str> {
        defaults::NAMES
            .iter()
            .find(|(_, _, action)| *action == self)
            .map(|(_, title, _)| *title)
    }

    /// Every bindable action in a sensible order for listing.
    pub fn bindable() -> impl Iterator<Item = Self> {
        defaults::NAMES.iter().map(|(_, _, action)| *action)
    }
}

/// A key binding the config file got wrong.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "the config file (M5) rebinds keys through this")
)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeymapError {
    Context(String),
    Key(String),
    Action(String),
}

impl fmt::Display for KeymapError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Context(c) => write!(
                f,
                "unknown keybinding section \"{c}\" (use global, input, conversations or messages)"
            ),
            Self::Key(k) => write!(f, "unknown key \"{k}\""),
            Self::Action(a) => write!(f, "unknown action \"{a}\""),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Binding {
    context: Context,
    chord: KeyChord,
    action: Action,
}

/// All bindings, most preferred first (hints show the first key bound to an
/// action). At most one binding per context and key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Keymap {
    bindings: Vec<Binding>,
}

impl Default for Keymap {
    fn default() -> Self {
        let bindings = defaults::BINDINGS
            .iter()
            .filter_map(|&(context, key, action)| {
                // A typo here is caught by `every_default_parses`.
                KeyChord::parse(key).map(|chord| Binding {
                    context,
                    chord,
                    action,
                })
            })
            .collect();
        Self { bindings }
    }
}

impl Keymap {
    /// What `key` does with `focus`.
    pub fn lookup(&self, key: KeyEvent, focus: Focus) -> Option<Action> {
        if key.kind == KeyEventKind::Release {
            return None;
        }
        let chord = KeyChord::from_event(key);
        let candidates = [Some(chord), chord.lowercase()];
        for context in [Context::from(focus), Context::Global] {
            for candidate in candidates.iter().flatten() {
                if let Some(action) = self.get(context, *candidate) {
                    return Some(action);
                }
            }
        }
        match (focus, chord.code) {
            (Focus::Input, KeyCode::Char(c)) if chord.modifiers.is_empty() => {
                Some(Action::Edit(Edit::Insert(c)))
            }
            (Focus::Input | Focus::Conversations | Focus::Messages, _) => None,
        }
    }

    fn get(&self, context: Context, chord: KeyChord) -> Option<Action> {
        self.bindings
            .iter()
            .find(|b| b.context == context && b.chord == chord)
            .map(|b| b.action)
    }

    /// Bind `chord` to `action` in `context`, replacing what it did there; it
    /// becomes the key hints show for `action`.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "the config file (M5) rebinds keys through this")
    )]
    pub fn bind(&mut self, context: Context, chord: KeyChord, action: Action) {
        self.unbind(context, chord);
        self.bindings.insert(
            0,
            Binding {
                context,
                chord,
                action,
            },
        );
    }

    /// Make `chord` do nothing in `context` (a global binding still applies).
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "the config file (M5) rebinds keys through this")
    )]
    pub fn unbind(&mut self, context: Context, chord: KeyChord) {
        self.bindings
            .retain(|b| b.context != context || b.chord != chord);
    }

    /// One config entry: `[keybindings.<context>] "<key>" = "<action>"`;
    /// the action `"none"` unbinds the key.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "the config file (M5) rebinds keys through this")
    )]
    pub fn apply(&mut self, context: &str, key: &str, action: &str) -> Result<(), KeymapError> {
        let context =
            Context::parse(context).ok_or_else(|| KeymapError::Context(context.into()))?;
        let chord = KeyChord::parse(key).ok_or_else(|| KeymapError::Key(key.into()))?;
        if action == "none" {
            self.unbind(context, chord);
            return Ok(());
        }
        let action = Action::parse(action).ok_or_else(|| KeymapError::Action(action.into()))?;
        self.bind(context, chord, action);
        Ok(())
    }

    /// The keys bound to `action` in exactly `context`, preferred first.
    pub fn keys_in(&self, context: Context, action: Action) -> Vec<KeyChord> {
        self.bindings
            .iter()
            .filter(|b| b.context == context && b.action == action)
            .map(|b| b.chord)
            .collect()
    }

    /// The keys that do `action` with `focus`, preferred first: the pane's own
    /// bindings, then global ones the pane doesn't override.
    pub fn keys_for(&self, action: Action, focus: Focus) -> Vec<KeyChord> {
        let context = Context::from(focus);
        let own = self
            .bindings
            .iter()
            .filter(|b| b.context == context && b.action == action);
        let global = self.bindings.iter().filter(|b| {
            b.context == Context::Global
                && b.action == action
                && self.get(context, b.chord).is_none()
        });
        own.chain(global).map(|b| b.chord).collect()
    }

    /// A key hint for `actions` (e.g. previous + next): the first key of each,
    /// `↑↓` for bare arrows, `alt+j/k` when they share modifiers. `None` if
    /// any of them has no key.
    pub fn hint(&self, actions: &[Action], focus: Focus) -> Option<String> {
        let chords = actions
            .iter()
            .map(|a| self.keys_for(*a, focus).first().copied())
            .collect::<Option<Vec<_>>>()?;
        let first = chords.first()?;
        let arrows = chords.iter().all(|c| {
            c.modifiers.is_empty()
                && matches!(
                    c.code,
                    KeyCode::Up | KeyCode::Down | KeyCode::Left | KeyCode::Right
                )
        });
        if arrows {
            return Some(chords.iter().map(|c| c.label()).collect());
        }
        if chords.iter().all(|c| c.modifiers == first.modifiers) && !first.modifiers.is_empty() {
            let bare: Vec<String> = chords
                .iter()
                .map(|c| KeyChord::new(c.code, KeyModifiers::NONE).label())
                .collect();
            let prefix = first.label();
            let prefix = prefix
                .strip_suffix(bare.first().map_or("", String::as_str))
                .unwrap_or_default();
            return Some(format!("{prefix}{}", bare.join("/")));
        }
        Some(
            chords
                .iter()
                .map(|c| c.label())
                .collect::<Vec<_>>()
                .join("/"),
        )
    }
}

/// While the palette is open it takes every key except quitting: arrows,
/// `Tab` and `Ctrl+P/N` move, `Enter` runs, `Esc` closes, and the input's
/// editing keys (as bound) edit the query.
pub fn palette_action(keymap: &Keymap, key: KeyEvent) -> Option<Action> {
    if key.kind == KeyEventKind::Release {
        return None;
    }
    let modifiers = key.modifiers.difference(KeyModifiers::SHIFT);
    let palette = match (key.code, modifiers) {
        (KeyCode::Char('q'), KeyModifiers::CONTROL) => return Some(Action::Quit),
        (KeyCode::Char('c'), KeyModifiers::CONTROL) => return Some(Action::Interrupt),
        (KeyCode::Up | KeyCode::BackTab, _) | (KeyCode::Char('p'), KeyModifiers::CONTROL) => {
            PaletteKey::Previous
        }
        (KeyCode::Down | KeyCode::Tab, _) | (KeyCode::Char('n'), KeyModifiers::CONTROL) => {
            PaletteKey::Next
        }
        (KeyCode::PageUp, _) => PaletteKey::PageUp,
        (KeyCode::PageDown, _) => PaletteKey::PageDown,
        (KeyCode::Enter, KeyModifiers::NONE) => PaletteKey::Choose,
        (KeyCode::Esc, _) => PaletteKey::Close,
        _ => {
            let Some(Action::Edit(edit)) = keymap.lookup(key, Focus::Input) else {
                return None;
            };
            if matches!(edit, Edit::NewLine | Edit::Up | Edit::Down) {
                return None;
            }
            PaletteKey::Edit(edit)
        }
    };
    Some(Action::Palette(palette))
}

/// While a popup is open it takes every key except quitting. `h`/`l` move
/// only in a `horizontal` popup (the reaction picker); in menus they're
/// shortcuts (D41). Not rebindable (D43).
pub fn popup_action(key: KeyEvent, horizontal: bool) -> Option<Action> {
    if key.kind == KeyEventKind::Release {
        return None;
    }
    let modifiers = key.modifiers.difference(KeyModifiers::SHIFT);
    let popup = match (key.code, modifiers) {
        (KeyCode::Char('q'), KeyModifiers::CONTROL) => return Some(Action::Quit),
        (KeyCode::Char('c'), KeyModifiers::CONTROL) => return Some(Action::Interrupt),
        (KeyCode::Char('h'), KeyModifiers::NONE) if horizontal => PopupKey::Previous,
        (KeyCode::Char('l'), KeyModifiers::NONE) if horizontal => PopupKey::Next,
        (KeyCode::Up | KeyCode::Left | KeyCode::BackTab, _)
        | (KeyCode::Char('k'), KeyModifiers::NONE)
        | (KeyCode::Char('p'), KeyModifiers::CONTROL) => PopupKey::Previous,
        (KeyCode::Down | KeyCode::Right | KeyCode::Tab, _)
        | (KeyCode::Char('j'), KeyModifiers::NONE)
        | (KeyCode::Char('n'), KeyModifiers::CONTROL) => PopupKey::Next,
        (KeyCode::Enter, _) => PopupKey::Choose,
        (KeyCode::Esc, _) | (KeyCode::Char('q'), KeyModifiers::NONE) => PopupKey::Close,
        (KeyCode::Char(c), KeyModifiers::NONE) => PopupKey::Char(c),
        _ => return None,
    };
    Some(Action::Popup(popup))
}
