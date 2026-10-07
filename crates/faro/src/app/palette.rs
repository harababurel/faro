//! The command palette (M4 task 8, D44): one fuzzy-searched list of commands
//! and conversations (`>` commands only, `@` conversations only, `:`
//! settings, `?` keybindings). Items are built from the state when it opens;
//! matching is synchronous (`nucleo-matcher`): a few hundred short strings
//! take well under a millisecond.

use faro_core::domain::ConversationId;
use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};

use super::input::Input;
use super::keymap::{Action, Context, Curate, Keymap};
use super::state::{
    ConversationAction, Focus, MUTE_CHOICES, MessageLayout, Section, SidebarView, State,
};

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
    /// Switch between the bubbles and IRC layouts (D45).
    SwitchLayout,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemKind {
    Command(PaletteCommand),
    Conversation(ConversationId),
    /// A line of the keybinding list (for reference; choosing it closes).
    Key,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub kind: ItemKind,
    /// What's matched and shown.
    pub title: String,
    /// Shown dim before the title (`Alice › `, `Sidebar › `); not matched.
    pub path: Option<String>,
    /// Right-aligned: its key, or a note (`3 unread · archived`).
    pub detail: String,
    /// Place in the recently used commands.
    pub recent: Option<usize>,
}

/// Which items the query is about, from its first character.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    All,
    Commands,
    Conversations,
    Settings,
    Keys,
}

impl Mode {
    /// The mode and the pattern after the prefix.
    pub fn of(query: &str) -> (Self, &str) {
        let mut chars = query.chars();
        let mode = match chars.next() {
            Some('>') => Self::Commands,
            Some('@') => Self::Conversations,
            Some(':') => Self::Settings,
            Some('?') => Self::Keys,
            Some(_) | None => return (Self::All, query.trim()),
        };
        (mode, chars.as_str().trim())
    }

    fn includes(self, kind: ItemKind) -> bool {
        matches!(
            (self, kind),
            (Self::All, ItemKind::Command(_) | ItemKind::Conversation(_))
                | (Self::Commands, ItemKind::Command(_))
                | (Self::Conversations, ItemKind::Conversation(_))
                | (Self::Keys, ItemKind::Key)
        )
    }

    pub const fn title(self) -> &'static str {
        match self {
            Self::All => "Search",
            Self::Commands => "Commands",
            Self::Conversations => "Conversations",
            Self::Settings => "Settings",
            Self::Keys => "Keybindings",
        }
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
    pub input: Input,
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
    pub fn new(items: Vec<Item>, query: &str) -> Self {
        let mut palette = Self {
            input: Input::new(query),
            items,
            matches: Vec::new(),
            selected: 0,
            offset: 0,
            matcher: Matcher::new(Config::DEFAULT),
        };
        palette.refilter();
        palette
    }

    pub fn mode(&self) -> Mode {
        Mode::of(self.input.text()).0
    }

    pub fn selected_item(&self) -> Option<&Item> {
        let found = self.matches.get(self.selected)?;
        self.items.get(found.item)
    }

    /// Match the query again; back to the best result.
    pub fn refilter(&mut self) {
        let (mode, pattern) = Mode::of(self.input.text());
        let pattern = Pattern::parse(pattern, CaseMatching::Smart, Normalization::Smart);
        let typed = !pattern.atoms.is_empty();
        let mut buf = Vec::new();
        let mut found: Vec<(u32, usize, Vec<u32>)> = Vec::new();
        for (i, item) in self.items.iter().enumerate() {
            if !mode.includes(item.kind) {
                continue;
            }
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

/// Everything the palette can offer right now, in the order shown when
/// nothing is typed: recently used commands, conversations, commands, then
/// the keybinding list (`?` only).
pub fn items(state: &State, recent: &[PaletteCommand]) -> Vec<Item> {
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
    first.extend(conversations(state));
    first.extend(rest);
    first.extend(keybindings(&state.keymap));
    first
}

fn command(command: PaletteCommand, title: impl Into<String>, detail: Option<String>) -> Item {
    Item {
        kind: ItemKind::Command(command),
        title: title.into(),
        path: None,
        detail: detail.unwrap_or_default(),
        recent: None,
    }
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
    items.push(command(
        PaletteCommand::SwitchLayout,
        match state.layout {
            MessageLayout::Bubbles => "Use the IRC layout",
            MessageLayout::Irc => "Use the bubbles layout",
        },
        None,
    ));
    items.push(command(
        PaletteCommand::ShowKeys,
        "Show keybindings",
        key(Action::Help, Focus::Input),
    ));
    items.push(global(Action::Quit, "Quit"));
    items
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
            Item {
                kind: ItemKind::Conversation(summary.id),
                title: state.conversation_title(summary.id),
                path: None,
                detail: notes.join(" · "),
                recent: None,
            }
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
            items.push(Item {
                kind: ItemKind::Key,
                title,
                path: Some(path.to_owned()),
                detail,
                recent: None,
            });
        }
    }
    for (title, detail) in [
        ("Move", "↑↓, j k, tab"),
        ("Choose", "enter, or the shortcut shown"),
        ("Close", "esc, q"),
    ] {
        items.push(Item {
            kind: ItemKind::Key,
            title: title.to_owned(),
            path: Some("Menus".to_owned()),
            detail: detail.to_owned(),
            recent: None,
        });
    }
    items
}
