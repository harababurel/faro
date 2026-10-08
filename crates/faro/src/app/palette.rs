//! The command palette (D44, tree since D52): a tree of levels. The root
//! lists the categories (chats, themes, settings, commands, keys); typing
//! filters only the level shown, `Enter`/`→` opens a category, `←` (or
//! `Backspace` on an empty query) goes back up. At the root, `@` `:` `>` `?`
//! jump straight into chats, settings, commands and keys. Settings is one
//! list under section headers that can't be selected (D57). A level's items are
//! built from the state when it's shown; matching is synchronous
//! (`nucleo-matcher`): a few hundred short strings take well under a
//! millisecond.

use std::path::PathBuf;

use faro_core::config::{
    AutoDownload, BubbleStyle, Choice as _, ColorChoice, EmojiChoice, InlineImages, TranscribeMode,
};
use faro_core::domain::{ConversationId, ServiceId};
use faro_core::store::{NotifyLevel, RemoteTranscription};
use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};

use super::input::Input;
use super::keymap::{Action, Context, Curate, Keymap};
use super::state::{
    ConversationAction, FileEntry, Focus, MUTE_CHOICES, MessageLayout, Section, SidebarView, State,
};
use crate::theme::{ColorDepth, Theme};

/// How many recently used commands are remembered (and ranked higher).
pub const RECENT_MAX: usize = 10;
/// Score bonus per place in the recent list (a strong match still wins).
const RECENT_BONUS: u32 = 3;
/// Recently used commands shown first when nothing is typed.
const RECENT_SHOWN: usize = 3;
/// "Hide conversations inactive for…" choices: days and label.
pub const INACTIVE_CHOICES: [(u32, &str); 4] = [
    (7, "1 week"),
    (30, "1 month"),
    (90, "3 months"),
    (365, "1 year"),
];

/// A level of the palette's tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    /// The categories.
    Root,
    Chats,
    Themes,
    Settings,
    /// One setting's values (`Settings › Colours`).
    Setting(Setting),
    Commands,
    /// The keybinding list.
    Keys,
    /// Everyone faro knows, to give one a nickname (D65).
    People,
    /// Typing a nickname for someone: the query is the name (D65).
    Nickname(ServiceId),
    /// Browsing a directory to attach a file to the open conversation (D77).
    AttachFile,
}

/// The sections of `Settings ›`, each under a header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsGroup {
    Appearance,
    Notifications,
    Conversations,
    Media,
    Input,
}

/// A setting: on/off ones flip where they are, the others list their
/// values as a level of their own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Setting {
    Colors,
    Emoji,
    Layout,
    BubbleStyle,
    ReduceMotion,
    Bell,
    Title,
    Allowlist,
    InlineImages,
    AutoDownload,
    Transcribe,
    Mouse,
}

impl Level {
    /// The root's categories, in order.
    pub const CATEGORIES: [Self; 5] = [
        Self::Chats,
        Self::Themes,
        Self::Settings,
        Self::Commands,
        Self::Keys,
    ];

    /// What the breadcrumb (and the item that opens it) calls it.
    pub const fn title(self) -> &'static str {
        match self {
            Self::Root => "Command palette",
            Self::Chats => "Chats",
            Self::Themes => "Themes",
            Self::Settings => "Settings",
            Self::Setting(setting) => setting.title(),
            Self::Commands => "Commands",
            Self::Keys => "Keys",
            Self::People => "Nicknames",
            Self::Nickname(_) => "Nickname",
            Self::AttachFile => "Send a file",
        }
    }

    /// The character that jumps here from the root.
    pub const fn prefix(self) -> Option<char> {
        match self {
            Self::Chats => Some('@'),
            Self::Settings => Some(':'),
            Self::Commands => Some('>'),
            Self::Keys => Some('?'),
            Self::Root
            | Self::Themes
            | Self::Setting(_)
            | Self::People
            | Self::Nickname(_)
            | Self::AttachFile => None,
        }
    }

    /// The category a character typed first at the root jumps to.
    pub fn for_prefix(c: char) -> Option<Self> {
        Self::CATEGORIES
            .into_iter()
            .find(|level| level.prefix() == Some(c))
    }
}

impl SettingsGroup {
    pub const ALL: [Self; 5] = [
        Self::Appearance,
        Self::Notifications,
        Self::Conversations,
        Self::Media,
        Self::Input,
    ];

    pub const fn title(self) -> &'static str {
        match self {
            Self::Appearance => "Appearance",
            Self::Notifications => "Notifications",
            Self::Conversations => "Conversations",
            Self::Media => "Media",
            Self::Input => "Input",
        }
    }

    /// Its settings, in order. Themes are a category of their own at the
    /// root, not a row here (D60).
    const fn settings(self) -> &'static [Setting] {
        match self {
            Self::Appearance => &[
                Setting::Colors,
                Setting::Emoji,
                Setting::Layout,
                Setting::BubbleStyle,
                Setting::ReduceMotion,
            ],
            Self::Notifications => &[Setting::Bell, Setting::Title],
            Self::Conversations => &[Setting::Allowlist],
            Self::Media => &[
                Setting::InlineImages,
                Setting::AutoDownload,
                Setting::Transcribe,
            ],
            Self::Input => &[Setting::Mouse],
        }
    }
}

impl Setting {
    pub const fn title(self) -> &'static str {
        match self {
            Self::Colors => "Colours",
            Self::Emoji => "Emoji",
            Self::Layout => "Message layout",
            Self::BubbleStyle => "Bubble style",
            Self::ReduceMotion => "Reduce motion",
            Self::Bell => "Bell",
            Self::Title => "Unread count in the window title",
            Self::Allowlist => "Allowlist mode",
            Self::InlineImages => "Inline images",
            Self::AutoDownload => "Auto-download media",
            Self::Transcribe => "Voice note transcription",
            Self::Mouse => "Mouse",
        }
    }

    /// Whether it's on or off (flipped by `Enter` in the settings list).
    pub const fn is_switch(self) -> bool {
        match self {
            Self::ReduceMotion | Self::Bell | Self::Title | Self::Allowlist | Self::Mouse => true,
            Self::Colors
            | Self::Emoji
            | Self::Layout
            | Self::BubbleStyle
            | Self::InlineImages
            | Self::AutoDownload
            | Self::Transcribe => false,
        }
    }

    /// Whether moving through its values draws each one before it's chosen
    /// (D59): the ones that only change how things are drawn. Emoji isn't
    /// one: switching modes repaints everything and `auto` waits for a look
    /// at the terminal.
    pub const fn previews(self) -> bool {
        match self {
            Self::Colors | Self::Layout | Self::BubbleStyle => true,
            Self::Emoji
            | Self::ReduceMotion
            | Self::Bell
            | Self::Title
            | Self::Allowlist
            | Self::InlineImages
            | Self::AutoDownload
            | Self::Transcribe
            | Self::Mouse => false,
        }
    }
}

/// What the palette tries on while it's open (D54, D59).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Preview {
    Theme(String),
    Setting(SettingChoice),
}

/// What a command item does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaletteCommand {
    /// The same as a key.
    Action(Action),
    /// To the open conversation.
    Conversation(ConversationAction),
    /// Mute the open conversation for `MUTE_CHOICES[i]`.
    MuteFor(usize),
    /// Hide every conversation inactive for `INACTIVE_CHOICES[i]`.
    HideInactive(usize),
    /// Copy the local file path of a downloaded attachment on the selected message.
    CopyAttachmentPath,
    /// Show the keybinding list.
    ShowKeys,
}

/// A value for a setting (D48); choosing it applies it and saves it to the
/// config file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingChoice {
    Colors(ColorChoice),
    Layout(MessageLayout),
    BubbleStyle(BubbleStyle),
    Allowlist(bool),
    Emoji(EmojiChoice),
    InlineImages(InlineImages),
    AutoDownload(AutoDownload),
    Transcribe(TranscribeMode),
    Mouse(bool),
    Bell(bool),
    Title(bool),
    ReduceMotion(bool),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ItemKind {
    /// A category or sub-level: choosing it opens it.
    Level(Level),
    Command(PaletteCommand),
    Conversation(ConversationId),
    Setting(SettingChoice),
    /// An on/off setting in the settings list: choosing it flips it and the
    /// palette stays open.
    Switch(Setting),
    /// A section's title; never selected, kept while a match is under it.
    Header,
    /// A theme by name (the config's `theme`).
    Theme(String),
    /// A line of the keybinding list (for reference; choosing it closes).
    Key,
    /// Give this person the typed name as a nickname (an empty query
    /// removes it) (D65).
    SetNickname(ServiceId),
    /// Take this person's nickname away.
    ClearNickname(ServiceId),
    /// Enter a subdirectory in `Level::AttachFile` by setting the query to this path.
    EnterDir(String),
    /// Stage a file for sending from `Level::AttachFile` (D77).
    AttachFile(PathBuf),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub kind: ItemKind,
    /// What's matched and shown.
    pub title: String,
    /// Shown dim before the title (`Alice › `, `Everywhere › `); not matched.
    pub path: Option<String>,
    /// Right-aligned: its key, a note (`3 unread · archived`) or a value.
    pub detail: String,
    /// Place in the recently used commands.
    pub recent: Option<usize>,
    /// For one of a setting's values: whether it's the one in use (drawn as
    /// a radio button).
    pub current: Option<bool>,
}

impl Item {
    fn new(kind: ItemKind, title: impl Into<String>) -> Self {
        Self {
            kind,
            title: title.into(),
            path: None,
            detail: String::new(),
            recent: None,
            current: None,
        }
    }

    fn detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = detail.into();
        self
    }

    fn current(mut self, current: bool) -> Self {
        self.current = Some(current);
        self
    }

    /// Whether it can be selected (headers can't).
    pub fn selectable(&self) -> bool {
        self.kind != ItemKind::Header
    }
}

/// An item that matches, with the title's matched characters (grapheme
/// indices, for highlighting).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Match {
    pub item: usize,
    pub highlight: Vec<u32>,
}

#[derive(Debug)]
pub struct Palette {
    /// From the root down to the level shown (never empty).
    pub path: Vec<Level>,
    pub input: Input,
    /// The level's items.
    pub items: Vec<Item>,
    /// What matches the query, best first.
    pub matches: Vec<Match>,
    /// Index into `matches`.
    pub selected: usize,
    /// First result shown (the renderer keeps `selected` in view).
    pub offset: usize,
    /// What's drawn while the palette is open: the theme selected at the
    /// Themes level (D54) or the value selected at a setting that changes
    /// how things look (D59), tried on before it's chosen.
    pub preview: Option<Preview>,
    /// A line above the results about the level (the colour depth at
    /// Themes, D57).
    pub note: Option<String>,
    matcher: Matcher,
}

impl Palette {
    pub fn new(path: Vec<Level>, items: Vec<Item>) -> Self {
        let mut palette = Self {
            path: Vec::new(),
            input: Input::new(""),
            items: Vec::new(),
            matches: Vec::new(),
            selected: 0,
            offset: 0,
            preview: None,
            note: None,
            matcher: Matcher::new(Config::DEFAULT),
        };
        palette.show(path, items, "", None);
        palette
    }

    /// The level shown.
    pub fn level(&self) -> Level {
        self.path.last().copied().unwrap_or(Level::Root)
    }

    /// The palette's title: where in the tree it is (`Settings › Bell`).
    pub fn breadcrumb(&self) -> String {
        let below_root: Vec<&str> = self
            .path
            .iter()
            .filter(|level| **level != Level::Root)
            .map(|level| level.title())
            .collect();
        if below_root.is_empty() {
            Level::Root.title().to_owned()
        } else {
            below_root.join(" › ")
        }
    }

    /// Show another level with `items`, the query set to `query`, and the
    /// item of kind `select` selected if it matches (else the best match).
    pub fn show(
        &mut self,
        path: Vec<Level>,
        items: Vec<Item>,
        query: &str,
        select: Option<&ItemKind>,
    ) {
        self.path = if path.is_empty() {
            vec![Level::Root]
        } else {
            path
        };
        self.items = items;
        self.input = Input::new(query);
        self.refilter();
        if let Some(kind) = select {
            self.select(kind);
        }
    }

    /// Select the item of `kind`, if it's among the matches.
    pub fn select(&mut self, kind: &ItemKind) {
        let found = self.matches.iter().position(|m| {
            self.items
                .get(m.item)
                .is_some_and(|item| item.kind == *kind)
        });
        if let Some(index) = found {
            self.selected = index;
        }
        self.sync_preview();
    }

    /// At the Themes level, preview the selected theme; at a setting that
    /// changes how things look, the selected value (D59); elsewhere, none.
    fn sync_preview(&mut self) {
        let selected = self.selected_item().map(|item| &item.kind);
        self.preview = match (self.level(), selected) {
            (Level::Themes, Some(ItemKind::Theme(name))) => Some(Preview::Theme(name.clone())),
            (Level::Setting(setting), Some(ItemKind::Setting(choice))) if setting.previews() => {
                Some(Preview::Setting(*choice))
            }
            (_, Some(_) | None) => None,
        };
    }

    /// The theme being tried on.
    pub fn previewed_theme(&self) -> Option<&str> {
        match &self.preview {
            Some(Preview::Theme(name)) => Some(name),
            Some(Preview::Setting(_)) | None => None,
        }
    }

    /// The setting value being tried on.
    pub const fn previewed_setting(&self) -> Option<SettingChoice> {
        match &self.preview {
            Some(Preview::Setting(choice)) => Some(*choice),
            Some(Preview::Theme(_)) | None => None,
        }
    }

    pub fn selected_item(&self) -> Option<&Item> {
        let found = self.matches.get(self.selected)?;
        self.items.get(found.item)
    }

    /// Match the query again; back to the best result.
    pub fn refilter(&mut self) {
        // Typing a nickname: the query is the name, every item stays.
        if let Level::Nickname(_) = self.level() {
            self.matches = (0..self.items.len())
                .map(|item| Match {
                    item,
                    highlight: Vec::new(),
                })
                .collect();
            self.selected = 0;
            self.offset = 0;
            self.sync_preview();
            return;
        }
        let filter_text = if self.level() == Level::AttachFile {
            super::system::split_file_query(self.input.text()).1.trim()
        } else {
            self.input.text().trim()
        };
        let pattern = Pattern::parse(filter_text, CaseMatching::Smart, Normalization::Smart);
        let typed = !pattern.atoms.is_empty();
        let mut buf = Vec::new();
        let mut found: Vec<(Option<u32>, Vec<u32>)> = Vec::with_capacity(self.items.len());
        for item in &self.items {
            let mut highlight = Vec::new();
            let haystack = Utf32Str::new(&item.title, &mut buf);
            let score = pattern.indices(haystack, &mut self.matcher, &mut highlight);
            highlight.sort_unstable();
            highlight.dedup();
            let bonus = item.recent.map_or(0, |r| {
                RECENT_BONUS * u32::try_from(RECENT_MAX.saturating_sub(r)).unwrap_or(0)
            });
            let score = score.map(|score| if typed { score + bonus } else { 0 });
            found.push((score, highlight));
        }
        let (matches, best) = if self.items.iter().any(|item| !item.selectable()) {
            grouped(&self.items, found)
        } else {
            ranked(found)
        };
        self.matches = matches;
        self.selected = best;
        self.offset = 0;
        self.sync_preview();
    }

    /// Move the selection by `delta`, wrapping at the ends for single steps
    /// and stopping there for pages; keep it within `rows` of `offset`.
    /// Headers are skipped.
    pub fn step(&mut self, delta: isize, rows: usize) {
        let len = self.matches.len();
        if len == 0 {
            return;
        }
        let last = len - 1;
        let single = |from: usize, up: bool| match (up, from) {
            (true, 0) => last,
            (true, i) => i - 1,
            (false, i) if i == last => 0,
            (false, i) => i + 1,
        };
        let up = delta < 0;
        if delta.unsigned_abs() == 1 {
            let mut at = single(self.selected, up);
            for _ in 0..len {
                if self.selectable(at) {
                    break;
                }
                at = single(at, up);
            }
            self.selected = at;
        } else {
            let target = if up {
                self.selected.saturating_sub(delta.unsigned_abs())
            } else {
                self.selected.saturating_add(delta.unsigned_abs()).min(last)
            };
            // The nearest item that can be selected, onwards first.
            let onwards = (target..=last).find(|i| self.selectable(*i));
            let back = (0..target).rev().find(|i| self.selectable(*i));
            self.selected = onwards.or(back).unwrap_or(self.selected);
        }
        self.reveal(rows);
        self.sync_preview();
    }

    /// Whether the result at `index` can be selected.
    pub fn selectable(&self, index: usize) -> bool {
        self.matches
            .get(index)
            .and_then(|m| self.items.get(m.item))
            .is_some_and(Item::selectable)
    }

    /// Scroll so the selection is among the `rows` shown.
    pub fn reveal(&mut self, rows: usize) {
        let rows = rows.max(1);
        if self.selected < self.offset {
            self.offset = self.selected;
        } else if self.selected >= self.offset + rows {
            self.offset = self.selected + 1 - rows;
        }
    }
}

/// Matches best first (ties in item order), and the best one's index.
fn ranked(found: Vec<(Option<u32>, Vec<u32>)>) -> (Vec<Match>, usize) {
    let mut found: Vec<(u32, usize, Vec<u32>)> = found
        .into_iter()
        .enumerate()
        .filter_map(|(item, (score, highlight))| Some((score?, item, highlight)))
        .collect();
    found.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    let matches = found
        .into_iter()
        .map(|(_, item, highlight)| Match { item, highlight })
        .collect();
    (matches, 0)
}

/// Matches in item order under their headers: an item matches by its own
/// title or its header's (typing `notif` lists the notification settings),
/// and a header is kept only above a match. The best match is selected.
fn grouped(items: &[Item], found: Vec<(Option<u32>, Vec<u32>)>) -> (Vec<Match>, usize) {
    let mut matches = Vec::new();
    let mut best: Option<(u32, usize)> = None;
    // The header above, its score and highlight, and whether it's shown.
    let mut header: Option<(usize, Option<u32>, Vec<u32>, bool)> = None;
    for (i, ((score, highlight), item)) in found.into_iter().zip(items).enumerate() {
        if !item.selectable() {
            header = Some((i, score, highlight, false));
            continue;
        }
        let under = header.as_ref().and_then(|(_, score, _, _)| *score);
        let Some(score) = score.max(under) else {
            continue;
        };
        if let Some((index, _, highlight, shown)) = header.as_mut()
            && !*shown
        {
            *shown = true;
            matches.push(Match {
                item: *index,
                highlight: std::mem::take(highlight),
            });
        }
        if best.is_none_or(|(top, _)| score > top) {
            best = Some((score, matches.len()));
        }
        matches.push(Match { item: i, highlight });
    }
    (matches, best.map_or(0, |(_, index)| index))
}

/// The items of `level`, in the order shown when nothing is typed.
pub fn items(state: &State, level: Level, recent: &[PaletteCommand]) -> Vec<Item> {
    match level {
        Level::Root => categories(state),
        Level::Chats => conversations(state),
        Level::Themes => themes(state),
        Level::Settings => settings(state),
        Level::Setting(setting) => setting_values(state, setting),
        Level::Commands => commands_by_recency(state, recent),
        Level::Keys => keybindings(&state.keymap),
        Level::People => people(state),
        Level::Nickname(id) => nickname_actions(state, id),
        Level::AttachFile => Vec::new(),
    }
}

/// Directory entries in `Level::AttachFile` as palette items (folders first, then files).
pub fn file_items(entries: &[FileEntry]) -> Vec<Item> {
    entries
        .iter()
        .map(|entry| {
            if entry.is_dir {
                Item::new(
                    ItemKind::EnterDir(entry.dir_query.clone()),
                    format!("{}/", entry.name),
                )
                .detail("folder")
            } else {
                Item::new(ItemKind::AttachFile(entry.path.clone()), entry.name.clone())
                    .detail(faro_core::domain::format_byte_size(entry.size))
            }
        })
        .collect()
}

/// Everyone faro knows, by name, to give one a nickname (D65).
fn people(state: &State) -> Vec<Item> {
    let mut items: Vec<Item> = state
        .people()
        .into_iter()
        .map(|id| {
            let detail = if state.nicknames.contains_key(&id) {
                "nickname"
            } else {
                ""
            };
            Item::new(ItemKind::Level(Level::Nickname(id)), state.person_name(id)).detail(detail)
        })
        .collect();
    items.sort_by_cached_key(|item| item.title.to_lowercase());
    items
}

/// At `Nickname`: save what's typed, or remove the nickname there is.
fn nickname_actions(state: &State, id: ServiceId) -> Vec<Item> {
    let mut items = vec![Item::new(ItemKind::SetNickname(id), "Save as nickname").detail("enter")];
    if state.nicknames.contains_key(&id) {
        items.push(Item::new(
            ItemKind::ClearNickname(id),
            "Remove the nickname",
        ));
    }
    items
}

/// The palette path that types a nickname for `id`.
pub fn nickname_path(id: ServiceId) -> Vec<Level> {
    vec![Level::Root, Level::People, Level::Nickname(id)]
}

/// The root: one item per category, with its prefix (or, for themes, the
/// theme in use) on the right.
fn categories(state: &State) -> Vec<Item> {
    Level::CATEGORIES
        .into_iter()
        .map(|level| {
            let detail = match level {
                Level::Themes => current_theme(state).to_owned(),
                Level::Root
                | Level::Chats
                | Level::Settings
                | Level::Setting(_)
                | Level::Commands
                | Level::Keys
                | Level::People
                | Level::Nickname(_)
                | Level::AttachFile => level.prefix().map(String::from).unwrap_or_default(),
            };
            Item::new(ItemKind::Level(level), level.title()).detail(detail)
        })
        .collect()
}

/// Commands, the 3 most recently used first.
fn commands_by_recency(state: &State, recent: &[PaletteCommand]) -> Vec<Item> {
    let mut commands = commands(state);
    for item in &mut commands {
        if let ItemKind::Command(command) = item.kind {
            item.recent = recent.iter().position(|c| *c == command);
        }
    }
    let (mut first, rest): (Vec<Item>, Vec<Item>) = commands
        .into_iter()
        .partition(|item| item.recent.is_some_and(|r| r < RECENT_SHOWN));
    first.sort_by_key(|item| item.recent);
    first.extend(rest);
    first
}

fn command(command: PaletteCommand, title: impl Into<String>, detail: Option<String>) -> Item {
    Item::new(ItemKind::Command(command), title).detail(detail.unwrap_or_default())
}

fn commands(state: &State) -> Vec<Item> {
    let keymap = &state.keymap;
    let key = |action: Action, focus: Focus| keymap.hint(&[action], focus);
    let global = |action: Action, title: &str| {
        command(
            PaletteCommand::Action(action),
            title,
            key(action, Focus::Input),
        )
    };
    let mut items = vec![
        global(Action::NextConversation, "Next conversation"),
        global(Action::PreviousConversation, "Previous conversation"),
    ];
    items.extend(open_conversation_commands(state));
    items.extend(selected_message_commands(state));
    let switch = Action::Curate(Curate::SwitchView);
    items.push(command(
        PaletteCommand::Action(switch),
        match state.list.view {
            SidebarView::Chats => "Show the archive",
            SidebarView::Archive => "Show the chats",
        },
        key(switch, Focus::Conversations),
    ));
    let mode = Action::Curate(Curate::AllowlistMode);
    items.push(command(
        PaletteCommand::Action(mode),
        if state.list.allowlist {
            "Turn allowlist mode off"
        } else {
            "Turn allowlist mode on"
        },
        key(mode, Focus::Conversations),
    ));
    for (i, (_, label)) in INACTIVE_CHOICES.iter().enumerate() {
        items.push(command(
            PaletteCommand::HideInactive(i),
            format!("Hide conversations inactive for {label}"),
            None,
        ));
    }
    items.push(Item::new(ItemKind::Level(Level::People), "Set a nickname…"));
    items.push(global(
        Action::ToggleLeftSidebar,
        if state.panels.left {
            "Hide the conversation list"
        } else {
            "Show the conversation list"
        },
    ));
    items.push(global(
        Action::ToggleRightSidebar,
        if state.panels.right {
            "Hide conversation info"
        } else {
            "Show conversation info"
        },
    ));
    items.push(global(
        Action::ToggleBell,
        if state.config.notifications.bell {
            "Turn do not disturb on (bell off)"
        } else {
            "Turn do not disturb off (bell on)"
        },
    ));
    items.push(command(
        PaletteCommand::ShowKeys,
        "Show keybindings",
        key(Action::Help, Focus::Input),
    ));
    items.push(global(Action::Quit, "Quit"));
    items
}

/// The theme in use (an unknown name was reported when the config was read
/// and the default is drawn).
pub fn current_theme(state: &State) -> &str {
    Theme::find(&state.config.theme, &state.user_themes)
        .map_or(faro_core::config::DEFAULT_THEME, |theme| {
            theme.name.as_str()
        })
}

/// Every theme, the one in use marked and where it comes from on the right.
fn themes(state: &State) -> Vec<Item> {
    let current = current_theme(state);
    Theme::all(&state.user_themes)
        .into_iter()
        .map(|theme| {
            let name = theme.name.as_str();
            Item::new(ItemKind::Theme(name.to_owned()), name)
                .current(name == current)
                .detail(theme.source.label().unwrap_or_default())
        })
        .collect()
}

/// `Settings ›`: every setting under its section's header, its current
/// value on the right.
fn settings(state: &State) -> Vec<Item> {
    let mut items = Vec::new();
    for group in SettingsGroup::ALL {
        items.push(Item::new(ItemKind::Header, group.title()));
        items.extend(group.settings().iter().map(|s| setting_item(state, *s)));
    }
    items
}

/// The row for `setting`, showing its current value: flipped by `Enter`
/// if it's on/off, else opening its values.
fn setting_item(state: &State, setting: Setting) -> Item {
    let value = values(state, setting)
        .into_iter()
        .find(|(_, _, current)| *current)
        .map(|(_, title, _)| title)
        .unwrap_or_default();
    let kind = if setting.is_switch() {
        ItemKind::Switch(setting)
    } else {
        ItemKind::Level(Level::Setting(setting))
    };
    Item::new(kind, setting.title()).detail(value)
}

/// The value an on/off `setting` changes to: the one not in use.
pub fn flipped(state: &State, setting: Setting) -> Option<SettingChoice> {
    values(state, setting)
        .into_iter()
        .find(|(_, _, current)| !*current)
        .map(|(choice, _, _)| choice)
}

/// A line about `level` shown above its results: at Themes, how the
/// terminal's colours change what's drawn (D57).
pub fn note(state: &State, level: Level) -> Option<String> {
    if let Level::Nickname(id) = level {
        let theirs = state
            .contacts
            .get(&id)
            .and_then(faro_core::domain::Contact::display_name)
            .map_or_else(String::new, |name| format!(" (their name: {name})"));
        return Some(format!(
            "Type a nickname{theirs}; only faro shows it. Empty removes it."
        ));
    }
    if level == Level::AttachFile {
        return Some(
            "Type a path; enter or → opens a folder, enter on a file attaches it (≤ 100 MB)."
                .to_owned(),
        );
    }
    if level != Level::Themes {
        return None;
    }
    match state.colors() {
        ColorDepth::Truecolor => None,
        ColorDepth::Indexed256 => {
            Some("256 colours: themes are approximated (truecolor: Settings › Colours)".to_owned())
        }
        ColorDepth::Ansi16 => Some(
            "16 colours: the system theme is drawn; others need 256 colours or more".to_owned(),
        ),
    }
}

/// `setting`'s values as items, the one in use marked.
fn setting_values(state: &State, setting: Setting) -> Vec<Item> {
    let overridden = state.env.emoji.from_cli && setting == Setting::Emoji;
    values(state, setting)
        .into_iter()
        .map(|(choice, title, current)| {
            let item = Item::new(ItemKind::Setting(choice), title).current(current);
            if overridden {
                item.detail("--emoji decides for now")
            } else {
                item
            }
        })
        .collect()
}

/// Every value of `setting`: what choosing it does, its name, and whether
/// it's the one in use.
fn values(state: &State, setting: Setting) -> Vec<(SettingChoice, String, bool)> {
    let on_off = |make: fn(bool) -> SettingChoice, current: bool| {
        [true, false]
            .into_iter()
            .map(|on| {
                (
                    make(on),
                    if on { "on" } else { "off" }.into(),
                    on == current,
                )
            })
            .collect()
    };
    match setting {
        Setting::Colors => ColorChoice::ALL
            .iter()
            .map(|&c| {
                let title = match c {
                    ColorChoice::Auto => format!("auto ({} detected)", state.env.colors.name()),
                    ColorChoice::Truecolor => "truecolor".to_owned(),
                    ColorChoice::Colors256 => "256".to_owned(),
                    ColorChoice::Colors16 => "16 (system theme)".to_owned(),
                };
                (SettingChoice::Colors(c), title, state.config.ui.colors == c)
            })
            .collect(),
        Setting::Emoji => EmojiChoice::ALL
            .iter()
            .map(|&e| {
                (
                    SettingChoice::Emoji(e),
                    e.name().into(),
                    state.config.ui.emoji == e,
                )
            })
            .collect(),
        Setting::Layout => MessageLayout::ALL
            .iter()
            .map(|&l| {
                let name = match l {
                    MessageLayout::Bubbles => "bubbles",
                    MessageLayout::Irc => "IRC",
                };
                (SettingChoice::Layout(l), name.into(), state.layout == l)
            })
            .collect(),
        Setting::BubbleStyle => BubbleStyle::ALL
            .iter()
            .map(|&s| {
                let name = match s {
                    BubbleStyle::Auto => "auto",
                    BubbleStyle::Rounded => "rounded",
                    BubbleStyle::Bar => "bar",
                    BubbleStyle::Mixed => "mixed",
                };
                (
                    SettingChoice::BubbleStyle(s),
                    name.into(),
                    state.config.messages.bubble_style == s,
                )
            })
            .collect(),
        Setting::ReduceMotion => on_off(SettingChoice::ReduceMotion, state.config.reduce_motion),
        Setting::Bell => on_off(SettingChoice::Bell, state.config.notifications.bell),
        Setting::Title => on_off(SettingChoice::Title, state.config.notifications.title),
        Setting::Allowlist => on_off(SettingChoice::Allowlist, state.list.allowlist),
        Setting::InlineImages => InlineImages::ALL
            .iter()
            .map(|&m| {
                (
                    SettingChoice::InlineImages(m),
                    m.name().into(),
                    state.config.media.inline_images == m,
                )
            })
            .collect(),
        Setting::AutoDownload => AutoDownload::ALL
            .iter()
            .map(|&p| {
                (
                    SettingChoice::AutoDownload(p),
                    p.name().into(),
                    state.config.media.auto_download == p,
                )
            })
            .collect(),
        Setting::Transcribe => TranscribeMode::ALL
            .iter()
            .map(|&m| {
                (
                    SettingChoice::Transcribe(m),
                    m.name().into(),
                    state.config.voice.transcribe == m,
                )
            })
            .collect(),
        Setting::Mouse => on_off(SettingChoice::Mouse, state.config.mouse.enabled),
    }
}

/// Curating the open conversation (the sidebar's highlight), with its name
/// as the path.
fn open_conversation_commands(state: &State) -> Vec<Item> {
    let Some(summary) = state.open.as_ref().and_then(|o| state.summary(o.id)) else {
        return vec![];
    };
    let current = &summary.state;
    let key = |curate| {
        state
            .keymap
            .hint(&[Action::Curate(curate)], Focus::Conversations)
    };
    let item = |action, title: &str, curate| {
        command(PaletteCommand::Conversation(action), title, key(curate))
    };
    let mut items = vec![Item::new(
        ItemKind::Level(Level::AttachFile),
        "Send a file…",
    )];
    if current.pinned.is_some() {
        items.push(item(ConversationAction::Unpin, "Unpin", Curate::Pin));
        let pins = state
            .conversations
            .iter()
            .filter(|c| c.state.pinned.is_some())
            .count();
        if pins > 1 {
            items.extend([
                item(ConversationAction::MoveUp, "Move pin up", Curate::MoveUp),
                item(
                    ConversationAction::MoveDown,
                    "Move pin down",
                    Curate::MoveDown,
                ),
            ]);
        }
    } else {
        items.push(item(ConversationAction::Pin, "Pin", Curate::Pin));
    }
    if current.muted_at(super::update::wall_clock()) {
        items.push(item(ConversationAction::Unmute, "Unmute", Curate::Mute));
    } else {
        for (i, (label, length)) in MUTE_CHOICES.iter().enumerate() {
            let title = if length.is_some() {
                format!("Mute for {label}")
            } else {
                "Mute until unmuted".to_owned()
            };
            items.push(command(PaletteCommand::MuteFor(i), title, None));
        }
    }
    let mentions = super::update::mentions_toggle(current);
    items.push(item(mentions, mentions.label().0, Curate::Mentions));
    items.push(if current.hidden {
        item(ConversationAction::Unhide, "Unhide", Curate::Hide)
    } else {
        item(ConversationAction::Hide, "Hide", Curate::Hide)
    });
    items.push(if current.archived {
        item(ConversationAction::Unarchive, "Unarchive", Curate::Archive)
    } else {
        item(ConversationAction::Archive, "Archive", Curate::Archive)
    });
    items.push(if current.allowed {
        item(
            ConversationAction::Disallow,
            "Remove from the allowlist",
            Curate::Allow,
        )
    } else {
        item(
            ConversationAction::Allow,
            "Add to the allowlist",
            Curate::Allow,
        )
    });
    if state.config.voice.transcribe == TranscribeMode::Remote
        || current.remote_transcription == RemoteTranscription::Allowed
    {
        let remote_action = if current.remote_transcription == RemoteTranscription::Allowed {
            ConversationAction::DisallowRemoteTranscription
        } else {
            ConversationAction::AllowRemoteTranscription
        };
        items.push(command(
            PaletteCommand::Conversation(remote_action),
            remote_action.label().0,
            None,
        ));
    }
    let name = state.conversation_title(summary.id);
    if let ConversationId::Direct(peer) = summary.id
        && peer != state.me
    {
        items.push(Item::new(
            ItemKind::Level(Level::Nickname(peer)),
            "Set a nickname…",
        ));
    }
    for item in &mut items {
        item.path = Some(name.clone());
    }
    items
}

/// What the selected message offers, if one is selected.
fn selected_message_commands(state: &State) -> Vec<Item> {
    let Some(open) = state.open.as_ref() else {
        return vec![];
    };
    let Some(selection) = open.selected else {
        return vec![];
    };
    let selected_msg = open
        .messages
        .iter()
        .rfind(|m| m.reference() == selection.message)
        .filter(|m| !m.deleted);
    let has_attachments = selected_msg.is_some_and(|m| !m.attachments.is_empty());
    let has_voice_note = selected_msg.is_some_and(|m| {
        m.attachments
            .iter()
            .any(faro_core::domain::AttachmentRef::is_voice_note)
    });
    let mut items: Vec<Item> = [
        (Action::Reply, "Reply"),
        (Action::ReactPicker, "React"),
        (Action::Copy, "Copy text"),
        (Action::MessageInfo, "Message info"),
        (Action::OpenLink, "Open link"),
    ]
    .into_iter()
    .map(|(action, title)| {
        let mut item = command(
            PaletteCommand::Action(action),
            title,
            state.keymap.hint(&[action], Focus::Messages),
        );
        item.path = Some("Message".to_owned());
        item
    })
    .collect();
    if has_voice_note {
        for (action, title) in [
            (Action::PlayVoiceNote, "Play or stop voice note"),
            (Action::TranscribeVoiceNote, "Transcribe voice note"),
        ] {
            let mut item = command(
                PaletteCommand::Action(action),
                title,
                state.keymap.hint(&[action], Focus::Messages),
            );
            item.path = Some("Message".to_owned());
            items.push(item);
        }
    }
    if has_attachments {
        for (action, title) in [
            (Action::DownloadAttachment, "Download attachment"),
            (Action::OpenAttachment, "View or open attachment"),
            (Action::SaveAttachment, "Save attachment to Downloads"),
        ] {
            let mut item = command(
                PaletteCommand::Action(action),
                title,
                state.keymap.hint(&[action], Focus::Messages),
            );
            item.path = Some("Message".to_owned());
            items.push(item);
        }
        let mut copy_path = command(
            PaletteCommand::CopyAttachmentPath,
            "Copy attachment path",
            None,
        );
        copy_path.path = Some("Message".to_owned());
        items.push(copy_path);
    }
    if selected_msg
        .is_some_and(|m| matches!(m.direction, faro_core::domain::Direction::Outgoing(_)))
    {
        if state
            .capabilities
            .supports(faro_core::backend::Capability::Edit)
        {
            let mut item = command(
                PaletteCommand::Action(Action::EditMessage),
                "Edit message",
                state.keymap.hint(&[Action::EditMessage], Focus::Messages),
            );
            item.path = Some("Message".to_owned());
            items.push(item);
        }
        if state
            .capabilities
            .supports(faro_core::backend::Capability::DeleteForEveryone)
        {
            let mut item = command(
                PaletteCommand::Action(Action::DeleteMessage),
                "Delete for everyone",
                state.keymap.hint(&[Action::DeleteMessage], Focus::Messages),
            );
            item.path = Some("Message".to_owned());
            items.push(item);
        }
    }
    items
}

/// Every conversation, including hidden, archived and empty ones, in list
/// order.
fn conversations(state: &State) -> Vec<Item> {
    let now = super::update::wall_clock();
    let open = state.open.as_ref().map(|o| o.id);
    state
        .conversations
        .iter()
        .map(|summary| {
            let mut notes = Vec::new();
            if summary.unread > 0 {
                notes.push(format!("{} unread", summary.unread));
            }
            if summary.state.muted_at(now) {
                notes.push("muted".to_owned());
            }
            if summary.state.notify == NotifyLevel::Mentions {
                notes.push("mentions only".to_owned());
            }
            match state.section(summary) {
                Section::Pinned => notes.push("pinned".to_owned()),
                Section::Recent => {}
                Section::Archived => notes.push("archived".to_owned()),
                Section::Hidden => notes.push("hidden".to_owned()),
                Section::NotAllowed => notes.push("not on the allowlist".to_owned()),
            }
            if open == Some(summary.id) {
                notes.push("open".to_owned());
            }
            Item::new(
                ItemKind::Conversation(summary.id),
                state.conversation_title(summary.id),
            )
            .detail(notes.join(" · "))
        })
        .collect()
}

/// The keybinding list: every bound action per context, then popups.
fn keybindings(keymap: &Keymap) -> Vec<Item> {
    let mut items = Vec::new();
    for context in Context::ALL {
        let path = match context {
            Context::Global => "Everywhere",
            Context::Input => "Input",
            Context::Conversations => "Sidebar",
            Context::Messages => "Messages",
        };
        for action in Action::bindable() {
            let keys = keymap.keys_in(context, action);
            let (title, detail) = if let Action::JumpToPinned(n) = action {
                // One line for Alt+1…Alt+9.
                let last = keymap.keys_in(context, Action::JumpToPinned(9));
                let (1, Some(first), Some(last)) = (n, keys.first(), last.first()) else {
                    continue;
                };
                (
                    "Open pinned conversation 1–9".to_owned(),
                    format!("{}…{}", first.label(), last.label()),
                )
            } else if keys.is_empty() {
                continue;
            } else {
                (
                    action.title().unwrap_or_default().to_owned(),
                    keys.iter()
                        .map(|k| k.label())
                        .collect::<Vec<_>>()
                        .join(", "),
                )
            };
            let mut item = Item::new(ItemKind::Key, title).detail(detail);
            item.path = Some(path.to_owned());
            items.push(item);
        }
    }
    for (title, detail) in [
        ("Move", "↑↓, j k, tab"),
        ("Choose", "enter, or the shortcut shown"),
        ("Close", "esc, q"),
    ] {
        let mut item = Item::new(ItemKind::Key, title).detail(detail);
        item.path = Some("Menus".to_owned());
        items.push(item);
    }
    let palette = [
        ("Open a category", "enter, →"),
        ("Run a command, apply a setting", "enter"),
        ("Back up a level", "←, backspace"),
        ("Jump to chats, settings, commands, keys", "@ : > ?"),
    ];
    for (title, detail) in palette {
        let mut item = Item::new(ItemKind::Key, title).detail(detail);
        item.path = Some("Palette".to_owned());
        items.push(item);
    }
    items
}
