//! faro's configuration file (DESIGN.md §9, D48): the settings, their
//! defaults, the commented template written on first run, parsing that
//! tolerates mistakes, and edits that keep comments (`edit.rs`).
//!
//! Pure: no file I/O here. Every setting is one entry in [`SETTINGS`], so the
//! template, the parser and write-back can't disagree about what exists.
//! Key bindings are only collected here (`[keybindings.<context>]`); the TUI
//! checks them against its key map.

mod edit;
#[cfg(test)]
mod tests;

use std::fmt;

use toml_edit::{DocumentMut, Item, Value};

pub use self::edit::{EditError, set};

/// A setting whose value is one of a few names.
pub trait Choice: Copy + Eq + 'static {
    const ALL: &'static [Self];

    fn name(self) -> &'static str;

    fn parse(name: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|c| c.name() == name)
    }
}

macro_rules! choice {
    (
        $(#[$meta:meta])*
        $name:ident { $($(#[$vmeta:meta])* $variant:ident = $text:literal),+ $(,)? }
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
        pub enum $name {
            $($(#[$vmeta])* $variant),+
        }

        impl Choice for $name {
            const ALL: &'static [Self] = &[$(Self::$variant),+];

            fn name(self) -> &'static str {
                match self {
                    $(Self::$variant => $text),+
                }
            }
        }
    };
}

choice! {
    /// How messages are laid out (DESIGN.md §7.3, D45).
    MessageLayout {
        /// Own messages on the right, others on the left, in coloured boxes.
        #[default]
        Bubbles = "bubbles",
        /// Everything on the left: `time  name  text`.
        Irc = "irc",
    }
}

impl MessageLayout {
    #[must_use]
    pub const fn other(self) -> Self {
        match self {
            Self::Bubbles => Self::Irc,
            Self::Irc => Self::Bubbles,
        }
    }
}

choice! {
    /// How message bubbles are drawn (D58).
    BubbleStyle {
        /// Rounded where the terminal is known to draw the glyphs itself
        /// (kitty), bars elsewhere.
        #[default]
        Auto = "auto",
        /// Half-circle caps on one-line bubbles, diagonal corners on taller
        /// ones (Powerline and Legacy Computing glyphs).
        Rounded = "rounded",
        /// A heavy bar on the bubble's outer side, in the sender's colour.
        Bar = "bar",
        /// The bar on the outer side, rounded toward the middle (D66).
        Mixed = "mixed",
    }
}

choice! {
    /// How emoji are drawn (D47): `auto` is compatible when mosh is in the way.
    EmojiChoice {
        #[default]
        Auto = "auto",
        Full = "full",
        Compatible = "compatible",
    }
}

choice! {
    /// How many colours to draw with (D50): `auto` guesses from the
    /// environment; with `16`, every theme becomes the `system` theme.
    ColorChoice {
        #[default]
        Auto = "auto",
        Truecolor = "truecolor",
        Colors256 = "256",
        Colors16 = "16",
    }
}

choice! {
    /// How desktop notifications are sent (DESIGN.md §8; used from M6).
    DesktopNotifications {
        #[default]
        Auto = "auto",
        Osc = "osc",
        Native = "native",
        Off = "off",
    }
}

choice! {
    /// What a notification shows (DESIGN.md §8; used from M6).
    NotificationPrivacy {
        /// Only that something arrived.
        None = "none",
        #[default]
        Sender = "sender",
        /// Sender and the start of the message.
        Preview = "preview",
    }
}

choice! {
    /// Which attachments download automatically when a conversation opens or a
    /// message arrives in the open conversation (DESIGN.md §7.9, D75).
    AutoDownload {
        /// Images and stickers up to 25 MB.
        #[default]
        Images = "images",
        /// Every attachment up to 25 MB.
        All = "all",
        /// Only when requested with `d`.
        Off = "off",
    }
}

choice! {
    /// How downloaded images are rendered inline in the message list
    /// (DESIGN.md §7.9, D76).
    InlineImages {
        /// Terminal graphics protocol when supported, Unicode half-blocks
        /// elsewhere (off at 16 colours).
        #[default]
        Auto = "auto",
        /// Always Unicode half-blocks (`▀`/`▄`), even in kitty/iTerm2/Sixel.
        Halfblocks = "halfblocks",
        /// Only the `📎` metadata line.
        Off = "off",
    }
}

/// Everything the config file sets. `Default` is what faro does without one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub theme: String,
    pub reduce_motion: bool,
    pub ui: Ui,
    pub conversations: Conversations,
    pub messages: Messages,
    pub media: Media,
    pub voice: Voice,
    pub notifications: Notifications,
    pub mouse: Mouse,
    /// `[keybindings.<context>]` entries in file order, unchecked.
    pub keybindings: Vec<Keybinding>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Ui {
    pub emoji: EmojiChoice,
    pub colors: ColorChoice,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Conversations {
    /// Allowlist mode (D41).
    pub allowlist: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Messages {
    pub layout: MessageLayout,
    pub bubble_style: BubbleStyle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Media {
    pub auto_download: AutoDownload,
    pub inline_images: InlineImages,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Voice {
    pub player: String,
}

impl Default for Voice {
    fn default() -> Self {
        Self {
            player: "auto".to_owned(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Notifications {
    pub bell: bool,
    /// Put the unread count in the window (and tmux pane) title.
    pub title: bool,
    pub desktop: DesktopNotifications,
    pub privacy: NotificationPrivacy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mouse {
    pub enabled: bool,
}

/// `[keybindings.<context>] "<key>" = "<action>"`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Keybinding {
    pub context: String,
    pub key: String,
    pub action: String,
}

/// The theme used when the config names none.
pub const DEFAULT_THEME: &str = "faro-dark";

impl Default for Config {
    fn default() -> Self {
        Self {
            theme: DEFAULT_THEME.to_owned(),
            reduce_motion: false,
            ui: Ui::default(),
            conversations: Conversations::default(),
            messages: Messages::default(),
            media: Media::default(),
            voice: Voice::default(),
            notifications: Notifications {
                bell: true,
                title: true,
                desktop: DesktopNotifications::default(),
                privacy: NotificationPrivacy::default(),
            },
            mouse: Mouse { enabled: true },
            keybindings: Vec::new(),
        }
    }
}

/// Paths of the settings the app changes itself (palette toggles).
pub mod key {
    pub const THEME: &str = "theme";
    pub const REDUCE_MOTION: &str = "reduce_motion";
    pub const EMOJI: &str = "ui.emoji";
    pub const COLORS: &str = "ui.colors";
    pub const ALLOWLIST: &str = "conversations.allowlist";
    pub const LAYOUT: &str = "messages.layout";
    pub const BUBBLE_STYLE: &str = "messages.bubble_style";
    pub const AUTO_DOWNLOAD: &str = "media.auto_download";
    pub const INLINE_IMAGES: &str = "media.inline_images";
    pub const VOICE_PLAYER: &str = "voice.player";
    pub const MOUSE: &str = "mouse.enabled";
    pub const BELL: &str = "notifications.bell";
    pub const TITLE: &str = "notifications.title";
}

/// A setting's value, as written in the file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigValue {
    Bool(bool),
    Text(String),
}

impl ConfigValue {
    pub fn to_toml(&self) -> Value {
        match self {
            Self::Bool(b) => Value::from(*b),
            Self::Text(s) => Value::from(s.as_str()),
        }
    }

    fn matches(&self, value: &Value) -> bool {
        match self {
            Self::Bool(b) => value.as_bool() == Some(*b),
            Self::Text(s) => value.as_str() == Some(s.as_str()),
        }
    }
}

impl fmt::Display for ConfigValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_toml().to_string().trim())
    }
}

/// One change to write back: `path` (e.g. `messages.layout`) gets `value`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingEdit {
    pub path: &'static str,
    pub value: ConfigValue,
}

/// One setting: where it lives, what it's for, and how to read and write it.
pub struct Setting {
    /// `table.key`, or just `key` at the top of the file.
    pub path: &'static str,
    /// One line for the template.
    pub help: &'static str,
    get: fn(&Config) -> ConfigValue,
    set: fn(&mut Config, &Value) -> Result<(), String>,
}

impl Setting {
    /// The table (`""` at the top) and the key.
    pub fn split(&self) -> (&'static str, &'static str) {
        split(self.path)
    }

    pub fn get(&self, config: &Config) -> ConfigValue {
        (self.get)(config)
    }
}

fn split(path: &str) -> (&str, &str) {
    path.rsplit_once('.').unwrap_or(("", path))
}

fn text(c: impl Choice) -> ConfigValue {
    ConfigValue::Text(c.name().to_owned())
}

/// Every setting, in template order (top-level keys first, then by table).
pub static SETTINGS: &[Setting] = &[
    Setting {
        path: key::THEME,
        help: "Colour theme: \"faro-dark\", \"faro-dusk\", \"faro-light\", \"system\" (the \
               terminal's own 16 colours), one of OpenCode's (\"tokyonight\", \"nord\", \
               \"github-light\"…) or your own: themes/<name>.json beside this file, in \
               OpenCode's format. Ctrl+P › Themes lists and previews them all.",
        get: |c| ConfigValue::Text(c.theme.clone()),
        set: |c, v| {
            c.theme = string(v)?;
            Ok(())
        },
    },
    Setting {
        path: key::REDUCE_MOTION,
        help: "Turn off animations: the lighthouse shown while no chat is open stands still.",
        get: |c| ConfigValue::Bool(c.reduce_motion),
        set: |c, v| {
            c.reduce_motion = boolean(v)?;
            Ok(())
        },
    },
    Setting {
        path: key::EMOJI,
        help: "Emoji: \"auto\" (simpler forms under mosh), \"full\" or \"compatible\". \
               --emoji overrides it.",
        get: |c| text(c.ui.emoji),
        set: |c, v| {
            c.ui.emoji = choice(v)?;
            Ok(())
        },
    },
    Setting {
        path: key::COLORS,
        help: "Colours: \"auto\" (detected), \"truecolor\", \"256\" (nearest of the 256) or \
               \"16\" (the system theme).",
        get: |c| text(c.ui.colors),
        set: |c, v| {
            c.ui.colors = choice(v)?;
            Ok(())
        },
    },
    Setting {
        path: key::ALLOWLIST,
        help: "Allowlist mode: list only allowed and pinned conversations; the rest wait in \
               the archive.",
        get: |c| ConfigValue::Bool(c.conversations.allowlist),
        set: |c, v| {
            c.conversations.allowlist = boolean(v)?;
            Ok(())
        },
    },
    Setting {
        path: key::LAYOUT,
        help: "How messages are laid out: \"bubbles\" or \"irc\".",
        get: |c| text(c.messages.layout),
        set: |c, v| {
            c.messages.layout = choice(v)?;
            Ok(())
        },
    },
    Setting {
        path: key::BUBBLE_STYLE,
        help: "Bubbles: \"rounded\" (needs a terminal or font with Powerline and Legacy \
               Computing glyphs, as kitty has), \"bar\" (a bar on the outer side), \
               \"mixed\" (the bar outside, rounded toward the middle) or \"auto\" (rounded in kitty, bars elsewhere). The system theme always draws bars.",
        get: |c| text(c.messages.bubble_style),
        set: |c, v| {
            c.messages.bubble_style = choice(v)?;
            Ok(())
        },
    },
    Setting {
        path: key::AUTO_DOWNLOAD,
        help: "Auto-download attachments in the open chat (up to 25 MB): \"images\", \"all\" or \
               \"off\" (download on demand with d).",
        get: |c| text(c.media.auto_download),
        set: |c, v| {
            c.media.auto_download = choice(v)?;
            Ok(())
        },
    },
    Setting {
        path: key::INLINE_IMAGES,
        help: "Inline images in the message list: \"auto\" (terminal graphics where supported, \
               half-blocks elsewhere), \"halfblocks\" or \"off\".",
        get: |c| text(c.media.inline_images),
        set: |c, v| {
            c.media.inline_images = choice(v)?;
            Ok(())
        },
    },
    Setting {
        path: key::VOICE_PLAYER,
        help: "External command for voice-note playback: \"auto\" (afplay on macOS, mpv, \
               ffplay, paplay, aplay or play) or a command name.",
        get: |c| ConfigValue::Text(c.voice.player.clone()),
        set: |c, v| {
            c.voice.player = string(v)?;
            Ok(())
        },
    },
    Setting {
        path: key::BELL,
        help: "Ring the terminal bell for new messages (not for muted or archived \
               conversations, nor for what you're looking at).",
        get: |c| ConfigValue::Bool(c.notifications.bell),
        set: |c, v| {
            c.notifications.bell = boolean(v)?;
            Ok(())
        },
    },
    Setting {
        path: key::TITLE,
        help: "Show the unread count in the window and tmux pane title: \"faro (3)\".",
        get: |c| ConfigValue::Bool(c.notifications.title),
        set: |c, v| {
            c.notifications.title = boolean(v)?;
            Ok(())
        },
    },
    Setting {
        path: "notifications.desktop",
        help: "Desktop notifications: \"auto\", \"osc\" (through the terminal), \"native\" or \
               \"off\" (not used yet: v0.2).",
        get: |c| text(c.notifications.desktop),
        set: |c, v| {
            c.notifications.desktop = choice(v)?;
            Ok(())
        },
    },
    Setting {
        path: "notifications.privacy",
        help: "What notifications show: \"none\", \"sender\" or \"preview\" (sender and text; \
               not used yet: v0.2).",
        get: |c| text(c.notifications.privacy),
        set: |c, v| {
            c.notifications.privacy = choice(v)?;
            Ok(())
        },
    },
    Setting {
        path: key::MOUSE,
        help: "Use the mouse (clicks, wheel). Off leaves it to the terminal, e.g. to select \
               text.",
        get: |c| ConfigValue::Bool(c.mouse.enabled),
        set: |c, v| {
            c.mouse.enabled = boolean(v)?;
            Ok(())
        },
    },
];

/// Settings DESIGN.md sketches that faro doesn't have yet: setting them says
/// so instead of "unknown".
const PLANNED: &[&str] = &["messages.density", "messages.timestamps"];

/// Settings that were renamed: still read (with a warning) when the new name
/// isn't set.
const RENAMED: &[Renamed] = &[Renamed {
    old: "messages.bubble_corners",
    new: key::BUBBLE_STYLE,
    values: &[("rounded", "rounded"), ("square", "bar")],
}];

/// A renamed setting: its old path, the new one, and old values → new ones.
struct Renamed {
    old: &'static str,
    new: &'static str,
    values: &'static [(&'static str, &'static str)],
}

/// The table holding key bindings.
const KEYBINDINGS: &str = "keybindings";

pub fn setting(path: &str) -> Option<&'static Setting> {
    SETTINGS.iter().find(|s| s.path == path)
}

impl Config {
    /// The value of the setting at `path`.
    pub fn value(&self, path: &str) -> Option<ConfigValue> {
        setting(path).map(|s| s.get(self))
    }

    /// Set the setting at `path`, as if the file said `value`.
    pub fn set(&mut self, path: &str, value: &ConfigValue) -> Result<(), String> {
        let setting = setting(path).ok_or_else(|| format!("unknown setting {path}"))?;
        (setting.set)(self, &value.to_toml())
    }

    /// The edit that writes this config's value of `path` to the file.
    pub fn edit(&self, path: &'static str) -> Option<SettingEdit> {
        self.value(path).map(|value| SettingEdit { path, value })
    }
}

fn shown(value: &Value) -> String {
    value.to_string().trim().to_owned()
}

fn boolean(value: &Value) -> Result<bool, String> {
    value
        .as_bool()
        .ok_or_else(|| format!("expected true or false, not {}", shown(value)))
}

fn string(value: &Value) -> Result<String, String> {
    value
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| format!("expected text in quotes, not {}", shown(value)))
}

fn choice<T: Choice>(value: &Value) -> Result<T, String> {
    let names: Vec<String> = T::ALL.iter().map(|c| format!("\"{}\"", c.name())).collect();
    let expected = match names.split_last() {
        Some((last, [])) => last.clone(),
        Some((last, rest)) => format!("{} or {last}", rest.join(", ")),
        None => String::new(),
    };
    value
        .as_str()
        .and_then(T::parse)
        .ok_or_else(|| format!("expected {expected}, not {}", shown(value)))
}

/// Something wrong with the file. Faro keeps going: a broken file keeps the
/// last good config, a bad value keeps that setting's previous value, and
/// unknown keys are ignored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    pub severity: Severity,
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Warning,
    Error,
}

impl Problem {
    pub fn warning(text: impl Into<String>) -> Self {
        Self {
            severity: Severity::Warning,
            text: text.into(),
        }
    }

    pub fn error(text: impl Into<String>) -> Self {
        Self {
            severity: Severity::Error,
            text: text.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parsed {
    pub config: Config,
    pub problems: Vec<Problem>,
}

/// Read a config file's text. `previous` is the config in use: it's kept
/// whole if the file isn't valid TOML, and per setting for bad values.
/// Settings the file doesn't mention take their defaults.
pub fn parse(text: &str, previous: &Config) -> Parsed {
    let doc = match text.parse::<DocumentMut>() {
        Ok(doc) => doc,
        Err(error) => {
            return Parsed {
                config: previous.clone(),
                problems: vec![Problem::error(syntax_error(text, &error))],
            };
        }
    };
    let defaults = Config::default();
    let mut config = previous.clone();
    let mut problems = Vec::new();
    for setting in SETTINGS {
        let (table, key) = setting.split();
        let item = if table.is_empty() {
            doc.get(key)
        } else {
            doc.get(table)
                .and_then(Item::as_table_like)
                .and_then(|t| t.get(key))
        };
        let Some(item) = item else {
            // Can't fail: it's the setting's own default.
            let _ = (setting.set)(&mut config, &setting.get(&defaults).to_toml());
            continue;
        };
        let result = item
            .as_value()
            .ok_or_else(|| "expected a value, not a table".to_owned())
            .and_then(|value| (setting.set)(&mut config, value));
        if let Err(error) = result {
            problems.push(Problem::error(format!(
                "{}: {error}; keeping {}",
                setting.path,
                setting.get(previous)
            )));
        }
    }
    renamed(&doc, &mut config, &mut problems);
    config.keybindings = keybindings(&doc, &mut problems);
    unknown_keys(&doc, &mut problems);
    Parsed { config, problems }
}

/// Whether `text` sets the setting at `path` (to anything, even a bad value).
pub fn mentions(text: &str, path: &str) -> bool {
    let Ok(doc) = text.parse::<DocumentMut>() else {
        return false;
    };
    let (table, key) = split(path);
    if table.is_empty() {
        doc.contains_key(key)
    } else {
        doc.get(table)
            .and_then(Item::as_table_like)
            .is_some_and(|t| t.contains_key(key))
    }
}

/// The item at `path` in `doc`, if it's there.
fn item<'a>(doc: &'a DocumentMut, path: &str) -> Option<&'a Item> {
    let (table, key) = split(path);
    if table.is_empty() {
        doc.get(key)
    } else {
        doc.get(table)
            .and_then(Item::as_table_like)
            .and_then(|t| t.get(key))
    }
}

/// Read renamed settings under their old names, unless the new name is set,
/// and say how to rename them.
fn renamed(doc: &DocumentMut, config: &mut Config, problems: &mut Vec<Problem>) {
    for rename in RENAMED {
        let Some(old) = item(doc, rename.old) else {
            continue;
        };
        let (_, new_key) = split(rename.new);
        if item(doc, rename.new).is_some() {
            problems.push(Problem::warning(format!(
                "{} is replaced by {}; ignored (remove it)",
                rename.old, rename.new
            )));
            continue;
        }
        let new_value = old
            .as_str()
            .and_then(|v| rename.values.iter().find(|(o, _)| *o == v))
            .map(|(_, n)| *n);
        let applied = new_value.and_then(|value| {
            config
                .set(rename.new, &ConfigValue::Text(value.to_owned()))
                .ok()
                .map(|()| value)
        });
        problems.push(match applied {
            Some(value) => Problem::warning(format!(
                "{} is now {}: write {new_key} = \"{value}\"",
                rename.old, rename.new
            )),
            None => Problem::error(format!(
                "{} is now {}; {} isn't a value it had",
                rename.old,
                rename.new,
                old.as_value().map_or_else(|| "a table".to_owned(), shown)
            )),
        });
    }
}

/// "line 3: invalid string" from `toml_edit`'s multi-line report.
fn syntax_error(text: &str, error: &toml_edit::TomlError) -> String {
    let message = error.message().lines().next().unwrap_or("not valid TOML");
    match error.span() {
        Some(span) => {
            let line = text
                .get(..span.start)
                .map_or(0, |before| before.matches('\n').count())
                + 1;
            format!("line {line}: {message}")
        }
        None => message.to_owned(),
    }
}

fn keybindings(doc: &DocumentMut, problems: &mut Vec<Problem>) -> Vec<Keybinding> {
    let Some(item) = doc.get(KEYBINDINGS) else {
        return Vec::new();
    };
    let Some(contexts) = item.as_table_like() else {
        problems.push(Problem::error(
            "keybindings: expected [keybindings.<context>] tables",
        ));
        return Vec::new();
    };
    let mut bindings = Vec::new();
    for (context, item) in contexts.iter() {
        let Some(entries) = item.as_table_like() else {
            problems.push(Problem::error(format!(
                "keybindings.{context}: expected a table of \"key\" = \"action\""
            )));
            continue;
        };
        for (key, item) in entries.iter() {
            match item.as_str() {
                Some(action) => bindings.push(Keybinding {
                    context: context.to_owned(),
                    key: key.to_owned(),
                    action: action.to_owned(),
                }),
                None => problems.push(Problem::error(format!(
                    "keybindings.{context}: \"{key}\" should name an action in quotes"
                ))),
            }
        }
    }
    bindings
}

/// Warn about keys faro doesn't know (typos, or settings from a newer faro).
fn unknown_keys(doc: &DocumentMut, problems: &mut Vec<Problem>) {
    let known_table = |name: &str| SETTINGS.iter().any(|s| s.split().0 == name);
    let mut check = |path: String| {
        if setting(&path).is_some() || RENAMED.iter().any(|r| r.old == path) {
            return;
        }
        if PLANNED.contains(&path.as_str()) {
            problems.push(Problem::warning(format!(
                "{path} isn't supported yet; ignored"
            )));
        } else {
            problems.push(Problem::warning(format!("unknown setting {path}; ignored")));
        }
    };
    for (key, item) in doc.iter() {
        if key == KEYBINDINGS {
            continue;
        }
        match item.as_table_like() {
            Some(table) if known_table(key) => {
                for (inner, _) in table.iter() {
                    check(format!("{key}.{inner}"));
                }
            }
            Some(_) | None => check(key.to_owned()),
        }
    }
}

/// The file written on first run: every setting commented out with its
/// default and a line saying what it does, then `extra` (the key bindings,
/// which the TUI knows).
pub fn template(extra: &str) -> String {
    let defaults = Config::default();
    let mut out = String::from(
        "# faro configuration.\n\
         #\n\
         # Every setting is listed with its default, commented out: remove the \"#\" to\n\
         # change one. faro applies changes while it runs. Mistakes are reported in the\n\
         # footer and the previous value is kept.\n",
    );
    let mut table = "";
    for setting in SETTINGS {
        let (this, key) = setting.split();
        let header = if this == table {
            String::new()
        } else {
            table = this;
            format!("[{table}]\n")
        };
        let default = setting.get(&defaults);
        out = format!("{out}\n{header}# {}\n# {key} = {default}\n", setting.help);
    }
    if !extra.is_empty() {
        out.push('\n');
        out.push_str(extra);
    }
    out
}

/// `text` as a TOML string (quoted and escaped), e.g. for keys like `"ctrl+p"`.
pub fn quoted(text: &str) -> String {
    Value::from(text).to_string().trim().to_owned()
}
