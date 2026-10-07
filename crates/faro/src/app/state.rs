//! Everything the UI shows. Changed only by the reducer (`update.rs`); read by the
//! views (`ui/`).

use std::collections::HashMap;
use std::time::Duration;

use faro_core::backend::BackendEvent;
use faro_core::domain::{
    ConnectionState, Contact, ConversationId, Direction, Group, GroupId, Message, MessageRef,
    Quote, ServiceId, Timestamp,
};
use faro_core::store::ConversationSummary;
use ratatui::layout::Rect;
use tokio::time::Instant;

use super::input::Input;
use super::keymap::Keymap;

/// Signal clients stop showing a typing indicator after this long without an update.
pub const TYPING_TIMEOUT: Duration = Duration::from_secs(15);
/// A draft is saved this long after the last edit (and on switching or quitting).
pub const DRAFT_SAVE_DELAY: Duration = Duration::from_secs(1);
/// After this long without an edit we tell the others we stopped typing (D42).
pub const TYPING_PAUSE: Duration = Duration::from_secs(3);
/// While typing goes on, "started typing" is repeated this often, so the
/// others' indicators don't time out (`TYPING_TIMEOUT`).
pub const TYPING_REFRESH: Duration = Duration::from_secs(10);
/// Informational notices ("Copied") disappear after this long.
pub const INFO_NOTICE_TIME: Duration = Duration::from_secs(3);
/// Signal's default reactions, in its order.
pub const REACTIONS: [&str; 6] = ["❤️", "👍", "👎", "😂", "😮", "😢"];
/// The mute picker's choices: label and length (`None`: until unmuted).
pub const MUTE_CHOICES: [(&str, Option<Duration>); 5] = [
    ("1 hour", Some(Duration::from_hours(1))),
    ("8 hours", Some(Duration::from_hours(8))),
    ("1 day", Some(Duration::from_hours(24))),
    ("1 week", Some(Duration::from_hours(7 * 24))),
    ("Until unmuted", None),
];

#[derive(Debug)]
pub struct State {
    /// This account.
    pub me: ServiceId,
    pub contacts: HashMap<ServiceId, Contact>,
    pub groups: HashMap<GroupId, Group>,
    /// Every known conversation, in display order (see `sort_conversations`).
    pub conversations: Vec<ConversationSummary>,
    /// The conversation list hasn't arrived from the store yet.
    pub loading: bool,
    pub open: Option<OpenConversation>,
    pub connection: ConnectionState,
    pub typing: HashMap<ConversationId, Vec<Typist>>,
    pub panels: Panels,
    /// Which pane keys go to (Tab cycles).
    pub focus: Focus,
    pub sidebar: SidebarScroll,
    /// Which conversations the sidebar lists (D41).
    pub list: ConversationList,
    /// Where things were on the last frame, for mouse hits and page sizes.
    pub geometry: Geometry,
    pub notice: Option<Notice>,
    /// A menu or picker over the message area; takes the keys while open.
    pub popup: Option<Popup>,
    pub env: Environment,
    /// What each key does (D43).
    pub keymap: Keymap,
    /// `sent_at` of the last message sent from here; the next one is later, so
    /// two quick sends can't share an identity.
    pub last_sent_at: Timestamp,
    /// Ctrl+C was pressed once; a second press quits.
    pub quit_armed: bool,
    pub quit: bool,
}

#[derive(Debug)]
pub struct OpenConversation {
    pub id: ConversationId,
    /// The newest page of history plus anything that arrived since, oldest first.
    pub messages: Vec<Message>,
    /// History is being loaded; events for this conversation wait in `pending`.
    pub loading: bool,
    pub pending: Vec<BackendEvent>,
    /// Unread count when the conversation was opened (for the "new" marker).
    pub unread_on_open: u32,
    /// First message that was unread when the conversation was opened.
    pub unread_marker: Option<MessageRef>,
    pub input: Input,
    /// When the input last changed without being saved as a draft.
    pub draft_edited: Option<Instant>,
    /// `None` follows the newest message; otherwise the view is pinned to a
    /// message, so new arrivals don't move it.
    pub scroll: Option<Anchor>,
    /// The store may have older messages than the first one loaded.
    pub has_older: bool,
    pub loading_older: bool,
    /// Messages that arrived while scrolled up.
    pub unseen: u32,
    /// The selected message (message selection, M4 task 4); highlighted while
    /// the message pane has focus.
    pub selected: Option<Selection>,
    /// The next message sent replies to this.
    pub reply_to: Option<Quote>,
    /// Incoming messages not yet marked read: receipts wait until they've been
    /// seen (`State::seeing_open`, D42).
    pub unreceipted: Vec<MessageRef>,
    /// We told the others we're typing here.
    pub typing_sent: Option<TypingSent>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TypingSent {
    /// When "started typing" was last sent.
    pub started: Instant,
    pub last_edit: Instant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Selection {
    pub message: MessageRef,
    /// Scroll so the message is on screen on the next frame.
    pub reveal: bool,
}

/// Facts about where faro runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Environment {
    /// Running over SSH: links are copied instead of opened (a browser would
    /// open on the remote machine, if at all). Read once at startup.
    pub over_ssh: bool,
    /// The terminal has focus (focus reporting; assumed until it says
    /// otherwise, and whenever a key or click arrives). D42.
    pub focused: bool,
}

/// A small modal over the message area.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Popup {
    /// What can be done with a message (`Enter` or right-click on it).
    Actions {
        target: MessageRef,
        items: Vec<MessageAction>,
        selected: usize,
    },
    /// Pick one of `REACTIONS`.
    React { target: MessageRef, selected: usize },
    /// What can be done with a conversation (`Space` or right-click in the
    /// sidebar, D41).
    Conversation {
        target: ConversationId,
        items: Vec<ConversationAction>,
        selected: usize,
    },
    /// Pick one of `MUTE_CHOICES`.
    Mute {
        target: ConversationId,
        selected: usize,
    },
}

/// Curating the conversation list (M4 task 5, D41).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConversationAction {
    Pin,
    Unpin,
    /// Move a pinned conversation up (earlier) or down the pinned list.
    MoveUp,
    MoveDown,
    /// Opens the mute picker.
    Mute,
    Unmute,
    Hide,
    Unhide,
    Archive,
    Unarchive,
    Allow,
    Disallow,
    /// Turn allowlist mode on (`true`) or off.
    AllowlistMode(bool),
}

impl ConversationAction {
    /// Menu label and the key that does the same in the sidebar.
    pub fn label(self) -> (&'static str, &'static str) {
        match self {
            Self::Pin => ("Pin", "p"),
            Self::Unpin => ("Unpin", "p"),
            Self::MoveUp => ("Move up", "K"),
            Self::MoveDown => ("Move down", "J"),
            Self::Mute => ("Mute…", "m"),
            Self::Unmute => ("Unmute", "m"),
            Self::Hide => ("Hide", "h"),
            Self::Unhide => ("Unhide", "h"),
            Self::Archive => ("Archive", "a"),
            Self::Unarchive => ("Unarchive", "a"),
            Self::Allow => ("Add to allowlist", "l"),
            Self::Disallow => ("Remove from allowlist", "l"),
            Self::AllowlistMode(true) => ("Turn allowlist mode on", "L"),
            Self::AllowlistMode(false) => ("Turn allowlist mode off", "L"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MessageAction {
    Reply,
    React,
    Copy,
    OpenLink(String),
    Retry,
}

impl MessageAction {
    /// Menu label and the key that does the same from the message pane.
    pub fn label(&self) -> (String, &'static str) {
        match self {
            Self::Reply => ("Reply".to_owned(), "r"),
            Self::React => ("React…".to_owned(), "+"),
            Self::Copy => ("Copy text".to_owned(), "y"),
            Self::OpenLink(url) => (format!("Open {url}"), "o"),
            Self::Retry => ("Retry sending".to_owned(), ""),
        }
    }
}

/// A scroll position: the bottom row of the message view is `offset` rows above
/// the last row of `message` (negative: below it). The renderer normalises it
/// so `0 <= offset < height of that message`, or back to `None` at the bottom.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Anchor {
    pub message: MessageRef,
    pub offset: isize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SidebarScroll {
    /// First row shown (rows include section headers).
    pub offset: usize,
    /// Scroll so the open conversation is visible on the next frame.
    pub reveal: bool,
}

/// The sidebar's two lists (D41).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SidebarView {
    /// Pinned and recent conversations.
    Chats,
    /// Archived, hidden and (in allowlist mode) not-allowed conversations.
    Archive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConversationList {
    pub view: SidebarView,
    /// Allowlist mode: only allowed (and pinned) conversations are in `Chats`.
    pub allowlist: bool,
}

/// Where a conversation is listed, in display order (D41).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Section {
    Pinned,
    Recent,
    Archived,
    /// Comes back to `Recent` when a message arrives.
    Hidden,
    /// Allowlist mode is on and this conversation isn't on the list.
    NotAllowed,
}

impl Section {
    pub fn view(self) -> SidebarView {
        match self {
            Self::Pinned | Self::Recent => SidebarView::Chats,
            Self::Archived | Self::Hidden | Self::NotAllowed => SidebarView::Archive,
        }
    }
}

/// What's on a sidebar row (reported by the renderer for mouse hits).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SidebarRow {
    /// A section header or a note.
    Label,
    Conversation(ConversationId),
    /// "▸ Archive" / "◂ Chats": switches the list.
    SwitchView,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Conversations,
    Messages,
    Input,
}

/// What the last frame looked like, reported by the renderer (`ui::render`)
/// and stored by the reducer (`AppEvent::Drawn`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Geometry {
    /// Inside the sidebar's border; `None` when hidden.
    pub sidebar: Option<Rect>,
    /// What's on each sidebar row from the top.
    pub sidebar_rows: Vec<SidebarRow>,
    /// The sidebar offset actually used (clamped, selection revealed).
    pub sidebar_offset: usize,
    /// The message history area.
    pub history: Option<Rect>,
    /// The input panel, border included.
    pub input: Option<Rect>,
    /// The normalised scroll position used for the frame.
    pub scroll: Option<Anchor>,
    /// The first loaded message is on screen.
    pub at_top: bool,
    /// The message on each row of the history area from the top (`None`:
    /// blank, or a date or unread rule).
    pub history_rows: Vec<Option<MessageRef>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Typist {
    pub who: ServiceId,
    pub since: Instant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Panels {
    pub left: bool,
    pub right: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    pub text: String,
    pub level: NoticeLevel,
    /// Disappears by itself at this time (info notices).
    pub until: Option<Instant>,
}

impl Notice {
    pub fn info(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            level: NoticeLevel::Info,
            until: Some(Instant::now() + INFO_NOTICE_TIME),
        }
    }

    pub fn warning(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            level: NoticeLevel::Warning,
            until: None,
        }
    }

    pub fn error(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            level: NoticeLevel::Error,
            until: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoticeLevel {
    Info,
    Warning,
    Error,
}

impl State {
    pub fn new(me: ServiceId, contacts: Vec<Contact>, groups: Vec<Group>) -> Self {
        Self {
            me,
            contacts: contacts.into_iter().map(|c| (c.service_id, c)).collect(),
            groups: groups.into_iter().map(|g| (g.id, g)).collect(),
            conversations: Vec::new(),
            loading: true,
            open: None,
            connection: ConnectionState::Connecting,
            typing: HashMap::new(),
            panels: Panels {
                left: true,
                right: false,
            },
            focus: Focus::Input,
            sidebar: SidebarScroll {
                offset: 0,
                reveal: true,
            },
            list: ConversationList {
                view: SidebarView::Chats,
                allowlist: false,
            },
            geometry: Geometry::default(),
            notice: None,
            popup: None,
            env: Environment {
                over_ssh: false,
                focused: true,
            },
            keymap: Keymap::default(),
            last_sent_at: Timestamp::from_millis(0),
            quit_armed: false,
            quit: false,
        }
    }

    /// Conversations listed in the sidebar's current view, in display order
    /// (section by section, D41).
    pub fn visible_conversations(&self) -> impl Iterator<Item = &ConversationSummary> {
        self.listed_in(self.list.view)
    }

    /// Conversations listed in `view`, in display order. Empty conversations
    /// aren't listed (DESIGN.md §7.7) unless open, pinned or allowed.
    pub fn listed_in(&self, view: SidebarView) -> impl Iterator<Item = &ConversationSummary> {
        let sections: &[Section] = match view {
            SidebarView::Chats => &[Section::Pinned, Section::Recent],
            SidebarView::Archive => &[Section::Archived, Section::Hidden, Section::NotAllowed],
        };
        let open = self.open.as_ref().map(|o| o.id);
        sections.iter().flat_map(move |&section| {
            self.conversations.iter().filter(move |c| {
                self.section(c) == section
                    && (c.last_activity.is_some()
                        || Some(c.id) == open
                        || c.state.pinned.is_some()
                        || (self.list.allowlist && c.state.allowed))
            })
        })
    }

    /// Where `summary` is listed.
    pub fn section(&self, summary: &ConversationSummary) -> Section {
        let state = &summary.state;
        if state.archived {
            Section::Archived
        } else if state.hidden {
            Section::Hidden
        } else if state.pinned.is_some() {
            Section::Pinned
        } else if self.list.allowlist && !state.allowed {
            Section::NotAllowed
        } else {
            Section::Recent
        }
    }

    pub fn summary(&self, id: ConversationId) -> Option<&ConversationSummary> {
        self.conversations.iter().find(|c| c.id == id)
    }

    /// Unread messages in the main list, muted conversations excepted.
    pub fn unread_total(&self, now: Timestamp) -> u32 {
        self.listed_in(SidebarView::Chats)
            .filter(|c| !c.state.muted_at(now))
            .map(|c| c.unread)
            .sum()
    }

    /// Name to show for a conversation: known title, then phone number, then a
    /// short form of the ID.
    pub fn conversation_title(&self, id: ConversationId) -> String {
        let summary = self.summary(id);
        if let Some(title) = summary.and_then(|s| s.title.as_deref()) {
            return title.to_owned();
        }
        match id {
            ConversationId::Direct(peer) => {
                if let Some(phone) = summary.and_then(|s| s.phone.as_ref()) {
                    return phone.expose().to_owned();
                }
                self.person_name(peer)
            }
            ConversationId::Group(group) => self
                .groups
                .get(&group)
                .map_or_else(|| "Unnamed group".to_owned(), |g| g.title.clone()),
        }
    }

    /// Name to show for a message's sender.
    pub fn sender_name(&self, message: &Message) -> String {
        if matches!(message.direction, Direction::Outgoing(_)) || message.sender == self.me {
            return "You".to_owned();
        }
        self.person_name(message.sender)
    }

    /// "You", contact name, phone number, or the first characters of the ID.
    pub fn person_name(&self, id: ServiceId) -> String {
        if id == self.me {
            return "You".to_owned();
        }
        let contact = self.contacts.get(&id);
        if let Some(name) = contact.and_then(Contact::display_name) {
            return name.to_owned();
        }
        if let Some(phone) = contact.and_then(|c| c.phone.as_ref()) {
            return phone.expose().to_owned();
        }
        id.uuid().simple().to_string().chars().take(8).collect()
    }

    /// The open conversation's newest messages are in front of the user: the
    /// terminal has focus, the history has loaded, and the view is at the
    /// bottom. Only then are messages read (D42).
    pub fn seeing_open(&self) -> bool {
        self.env.focused
            && self
                .open
                .as_ref()
                .is_some_and(|o| !o.loading && o.scroll.is_none())
    }

    pub fn typists(&self, id: ConversationId) -> &[Typist] {
        self.typing.get(&id).map_or(&[], Vec::as_slice)
    }

    /// When the reducer next needs a `Tick` (a typing indicator expiring, a
    /// draft to save, a notice to clear, "stopped typing" to send).
    pub fn next_deadline(&self) -> Option<Instant> {
        let typing = self
            .typing
            .values()
            .flatten()
            .map(|t| t.since + TYPING_TIMEOUT);
        let draft = self
            .open
            .as_ref()
            .and_then(|o| o.draft_edited)
            .map(|t| t + DRAFT_SAVE_DELAY);
        let notice = self.notice.as_ref().and_then(|n| n.until);
        let typing_sent = self
            .open
            .as_ref()
            .and_then(|o| o.typing_sent)
            .map(|t| t.last_edit + TYPING_PAUSE);
        typing.chain(draft).chain(notice).chain(typing_sent).min()
    }
}

/// Same order as the store: pinned first (by position), then most recent
/// activity, then empty conversations.
pub fn sort_conversations(conversations: &mut [ConversationSummary]) {
    conversations.sort_by(|a, b| {
        let key = |c: &ConversationSummary| {
            (
                c.state.pinned.is_none(),
                c.state.pinned,
                c.last_activity.is_none(),
                std::cmp::Reverse(c.last_activity),
            )
        };
        key(a)
            .cmp(&key(b))
            .then_with(|| a.id.to_string().cmp(&b.id.to_string()))
    });
}
