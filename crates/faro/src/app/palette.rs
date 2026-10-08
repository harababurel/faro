//! The command palette (D44, tree since D52): a tree of levels. The root
//! lists the categories (chats, themes, settings, commands, keys); typing
//! filters only the level shown, `Enter`/`→` opens a category, `←` (or
//! `Backspace` on an empty query) goes back up. At the root, `@` `:` `>` `?`
//! jump straight into chats, settings, commands and keys. A level's items are
//! built from the state when it's shown; matching is synchronous
//! (`nucleo-matcher`): a few hundred short strings take well under a
//! millisecond.

use faro_core::config::{Choice as _, ColorChoice, EmojiChoice};
use faro_core::domain::ConversationId;
use faro_core::store::NotifyLevel;
use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};

use super::input::Input;
use super::keymap::{Action, Context, Curate, Keymap};
use super::state::{
    ConversationAction, Focus, MUTE_CHOICES, MessageLayout, Section, SidebarView, State,
};
use crate::theme::Theme;

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
    /// A group of settings (`Settings › Notifications`).
    Group(SettingsGroup),
    /// One setting's values (`Settings › Notifications › Bell`).
    Setting(Setting),
    Commands,
    /// The keybinding list.
    Keys,
}

/// Settings grouped under `Settings ›`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsGroup {
    Appearance,
    Notifications,
    Conversations,
}

/// A setting with a few values, listed as a level of its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Setting {
    Colors,
    Emoji,
    Layout,
    Bell,
    Title,
    Allowlist,
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
            Self::Group(group) => group.title(),
            Self::Setting(setting) => setting.title(),
            Self::Commands => "Commands",
            Self::Keys => "Keys",
        }
    }

    /// The character that jumps here from the root.
    pub const fn prefix(self) -> Option<char> {
        match self {
            Self::Chats => Some('@'),
            Self::Settings => Some(':'),
            Self::Commands => Some('>'),
            Self::Keys => Some('?'),
            Self::Root | Self::Themes | Self::Group(_) | Self::Setting(_) => None,
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
    pub const fn title(self) -> &'static str {
        match self {
            Self::Appearance => "Appearance",
            Self::Notifications => "Notifications",
            Self::Conversations => "Conversations",
        }
    }
}

impl Setting {
    pub const fn title(self) -> &'static str {
        match self {
            Self::Colors => "Colours",
            Self::Emoji => "Emoji",
            Self::Layout => "Message layout",
            Self::Bell => "Bell",
            Self::Title => "Unread count in the window title",
            Self::Allowlist => "Allowlist mode",
            Self::Mouse => "Mouse",
        }
    }
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
    /// Show the keybinding list.
    ShowKeys,
}

/// A value for a setting (D48); choosing it applies it and saves it to the
/// config file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingChoice {
    Colors(ColorChoice),
    Layout(MessageLayout),
    Allowlist(bool),
    Emoji(EmojiChoice),
    Mouse(bool),
    Bell(bool),
    Title(bool),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ItemKind {
    /// A category or sub-level: choosing it opens it.
    Level(Level),
    Command(PaletteCommand),
    Conversation(ConversationId),
    Setting(SettingChoice),
    /// A theme by name (the config's `theme`).
    Theme(String),
    /// A line of the keybinding list (for reference; choosing it closes).
    Key,
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
    }

    pub fn selected_item(&self) -> Option<&Item> {
        let found = self.matches.get(self.selected)?;
        self.items.get(found.item)
    }

    /// Match the query again; back to the best result.
    pub fn refilter(&mut self) {
        let pattern = Pattern::parse(
            self.input.text().trim(),
            CaseMatching::Smart,
            Normalization::Smart,
        );
        let typed = !pattern.atoms.is_empty();
        let mut buf = Vec::new();
        let mut found: Vec<(u32, usize, Vec<u32>)> = Vec::new();
        for (i, item) in self.items.iter().enumerate() {
            let mut highlight = Vec::new();
            let haystack = Utf32Str::new(&item.title, &mut buf);
            let Some(score) = pattern.indices(haystack, &mut self.matcher, &mut highlight) else {
                continue;
            };
            highlight.sort_unstable();
            highlight.dedup();
            let bonus = item.recent.map_or(0, |r| {
                RECENT_BONUS * u32::try_from(RECENT_MAX.saturating_sub(r)).unwrap_or(0)
            });
            let score = if typed { score + bonus } else { 0 };
            found.push((score, i, highlight));
        }
        // Best first; ties (and everything, when nothing is typed) in item order.
        found.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        self.matches = found
            .into_iter()
            .map(|(_, item, highlight)| Match { item, highlight })
            .collect();
        self.selected = 0;
        self.offset = 0;
    }

    /// Move the selection by `delta`, wrapping at the ends for single steps
    /// and stopping there for pages; keep it within `rows` of `offset`.
    pub fn step(&mut self, delta: isize, rows: usize) {
        let len = self.matches.len();
        if len == 0 {
            return;
        }
        let last = len - 1;
        self.selected = match (delta.unsigned_abs() == 1, delta < 0) {
            (true, true) if self.selected == 0 => last,
            (true, false) if self.selected == last => 0,
            (_, true) => self.selected.saturating_sub(delta.unsigned_abs()),
            (_, false) => self.selected.saturating_add(delta.unsigned_abs()).min(last),
        };
        self.reveal(rows);
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

/// The items of `level`, in the order shown when nothing is typed.
pub fn items(state: &State, level: Level, recent: &[PaletteCommand]) -> Vec<Item> {
    match level {
        Level::Root => categories(state),
        Level::Chats => conversations(state),
        Level::Themes => themes(state),
        Level::Settings => settings(state),
        Level::Group(group) => group_settings(state, group),
        Level::Setting(setting) => setting_values(state, setting),
        Level::Commands => commands_by_recency(state, recent),
        Level::Keys => keybindings(&state.keymap),
    }
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
                | Level::Group(_)
                | Level::Setting(_)
                | Level::Commands
                | Level::Keys => level.prefix().map(String::from).unwrap_or_default(),
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
fn current_theme(state: &State) -> &str {
    let configured = state.config.theme.as_str();
    if Theme::NAMES.contains(&configured) {
        configured
    } else {
        faro_core::config::DEFAULT_THEME
    }
}

/// Every theme, the one in use marked.
fn themes(state: &State) -> Vec<Item> {
    let current = current_theme(state);
    Theme::NAMES
        .iter()
        .map(|name| Item::new(ItemKind::Theme((*name).to_owned()), *name).current(*name == current))
        .collect()
}

/// `Settings ›`: the groups, then settings that stand alone.
fn settings(state: &State) -> Vec<Item> {
    let mut items: Vec<Item> = [
        SettingsGroup::Appearance,
        SettingsGroup::Notifications,
        SettingsGroup::Conversations,
    ]
    .into_iter()
    .map(|group| Item::new(ItemKind::Level(Level::Group(group)), group.title()))
    .collect();
    items.push(setting_item(state, Setting::Mouse));
    items
}

/// A group's settings, each with its current value on the right.
fn group_settings(state: &State, group: SettingsGroup) -> Vec<Item> {
    let settings: &[Setting] = match group {
        SettingsGroup::Appearance => &[Setting::Colors, Setting::Emoji, Setting::Layout],
        SettingsGroup::Notifications => &[Setting::Bell, Setting::Title],
        SettingsGroup::Conversations => &[Setting::Allowlist],
    };
    let mut items = Vec::new();
    if group == SettingsGroup::Appearance {
        items.push(Item::new(ItemKind::Level(Level::Themes), "Theme").detail(current_theme(state)));
    }
    items.extend(settings.iter().map(|s| setting_item(state, *s)));
    items
}

/// The item that opens `setting`'s values, showing the current one.
fn setting_item(state: &State, setting: Setting) -> Item {
    let value = values(state, setting)
        .into_iter()
        .find(|(_, _, current)| *current)
        .map(|(_, title, _)| title)
        .unwrap_or_default();
    Item::new(ItemKind::Level(Level::Setting(setting)), setting.title()).detail(value)
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
                let name = if on { "on" } else { "off" };
                (make(on), name.to_owned(), on == current)
            })
            .collect()
    };
    match setting {
        Setting::Colors => ColorChoice::ALL
            .iter()
            .map(|colors| {
                let title = match colors {
                    ColorChoice::Auto => format!("auto ({} detected)", state.env.colors.name()),
                    ColorChoice::Truecolor => "truecolor".to_owned(),
                    ColorChoice::Colors256 => "256".to_owned(),
                    ColorChoice::Colors16 => "16 (system theme)".to_owned(),
                };
                (
                    SettingChoice::Colors(*colors),
                    title,
                    state.config.ui.colors == *colors,
                )
            })
            .collect(),
        Setting::Emoji => EmojiChoice::ALL
            .iter()
            .map(|emoji| {
                (
                    SettingChoice::Emoji(*emoji),
                    emoji.name().to_owned(),
                    state.config.ui.emoji == *emoji,
                )
            })
            .collect(),
        Setting::Layout => MessageLayout::ALL
            .iter()
            .map(|layout| {
                let name = match layout {
                    MessageLayout::Bubbles => "bubbles",
                    MessageLayout::Irc => "IRC",
                };
                (
                    SettingChoice::Layout(*layout),
                    name.to_owned(),
                    state.layout == *layout,
                )
            })
            .collect(),
        Setting::Bell => on_off(SettingChoice::Bell, state.config.notifications.bell),
        Setting::Title => on_off(SettingChoice::Title, state.config.notifications.title),
        Setting::Allowlist => on_off(SettingChoice::Allowlist, state.list.allowlist),
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
    let mut items = Vec::new();
    if current.pinned.is_some() {
        items.push(item(ConversationAction::Unpin, "Unpin", Curate::Pin));
        let pins = state
            .conversations
            .iter()
            .filter(|c| c.state.pinned.is_some())
            .count();
        if pins > 1 {
            items.push(item(
                ConversationAction::MoveUp,
                "Move pin up",
                Curate::MoveUp,
            ));
            items.push(item(
                ConversationAction::MoveDown,
                "Move pin down",
                Curate::MoveDown,
            ));
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
    let name = state.conversation_title(summary.id);
    for item in &mut items {
        item.path = Some(name.clone());
    }
    items
}

/// What the selected message offers, if one is selected.
fn selected_message_commands(state: &State) -> Vec<Item> {
    if state.open.as_ref().and_then(|o| o.selected).is_none() {
        return vec![];
    }
    [
        (Action::Reply, "Reply"),
        (Action::ReactPicker, "React"),
        (Action::Copy, "Copy text"),
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
    .collect()
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
