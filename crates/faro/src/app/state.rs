//! Everything the UI shows. Changed only by the reducer (`update.rs`); read by the
//! views (`ui/`).

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::time::Duration;

use faro_core::backend::BackendEvent;
use faro_core::config::{BubbleStyle, ColorChoice, Config};
use faro_core::domain::{
    AttachmentRef, BodyPart, ConnectionState, Contact, ConversationId, Direction, Group, GroupId,
    Message, MessageRef, Quote, ServiceId, Timestamp, mention_parts,
};
use faro_core::store::{ConversationSummary, NotifyLevel};
use ratatui::layout::Rect;
use tokio::time::Instant;

use super::input::Input;
use super::keymap::Keymap;
use super::palette::{Palette, PaletteCommand, SettingChoice};
use crate::theme::{ColorDepth, UserThemes};

/// Signal clients stop showing a typing indicator after this long without an update.
pub const TYPING_TIMEOUT: Duration = Duration::from_secs(15);
/// A draft is saved this long after the last edit (and on switching or quitting).
pub const DRAFT_SAVE_DELAY: Duration = Duration::from_secs(1);
/// After this long without an edit we tell the others we stopped typing (D42).
pub const TYPING_PAUSE: Duration = Duration::from_secs(3);
/// While typing goes on, "started typing" is repeated this often, so the
/// others' indicators don't time out (`TYPING_TIMEOUT`).
pub const TYPING_REFRESH: Duration = Duration::from_secs(10);
/// At most one terminal bell this often, so a flood doesn't ring on and on (D49).
pub const BELL_INTERVAL: Duration = Duration::from_secs(2);
/// Informational notices ("Copied") disappear after this long.
pub const INFO_NOTICE_TIME: Duration = Duration::from_secs(3);
/// Signal's default reactions, in its order.
pub const REACTIONS: [&str; 6] = ["❤️", "👍", "👎", "😂", "😮", "😢"];
/// At most this many attachments download at the same time (D75).
pub const MAX_DOWNLOADS_IN_FLIGHT: usize = 3;
/// Auto-download size ceiling: 25 MB (D75). Larger files wait for `d`.
pub const AUTO_DOWNLOAD_LIMIT: u64 = 25 * 1024 * 1024;
/// Maximum size of an outgoing attachment: 100 MB, Signal's limit (D77).
pub const MAX_ATTACHMENT_SIZE: u64 = 100 * 1024 * 1024;
/// The mute picker's choices: label and length (`None`: until unmuted).
pub const MUTE_CHOICES: [(&str, Option<Duration>); 5] = [
    ("1 hour", Some(Duration::from_hours(1))),
    ("8 hours", Some(Duration::from_hours(8))),
    ("1 day", Some(Duration::from_hours(24))),
    ("1 week", Some(Duration::from_hours(7 * 24))),
    ("Until unmuted", None),
];

/// An attachment queued to download (D75).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadJob {
    pub conversation: ConversationId,
    pub message: MessageRef,
    pub index: usize,
    pub attachment: AttachmentRef,
}

/// A local file attached to the next message in the open conversation (D77).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StagedAttachment {
    pub path: PathBuf,
    pub name: String,
    pub content_type: String,
    pub size: u64,
    pub width: Option<u32>,
    pub height: Option<u32>,
}

/// One entry in a folder listed for the palette's file picker (D77).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEntry {
    pub name: String,
    pub path: PathBuf,
    /// What query string entering this folder should set in the palette.
    pub dir_query: String,
    pub is_dir: bool,
    pub size: u64,
}

#[derive(Debug)]
pub struct State {
    /// This account.
    pub me: ServiceId,
    pub contacts: HashMap<ServiceId, Contact>,
    pub groups: HashMap<GroupId, Group>,
    /// Names the user gave people, shown instead of theirs (D65).
    pub nicknames: HashMap<ServiceId, String>,
    /// Names (lowercased) that more than one person goes by; those people
    /// get a short tag after their name (D65). Kept by `recount_names`.
    pub names_clash: HashSet<String>,
    /// Every known conversation, in display order (see `sort_conversations`).
    pub conversations: Vec<ConversationSummary>,
    /// The conversation list hasn't arrived from the store yet.
    pub loading: bool,
    pub open: Option<OpenConversation>,
    pub connection: ConnectionState,
    pub typing: HashMap<ConversationId, Vec<Typist>>,
    pub panels: Panels,
    pub layout: MessageLayout,
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
    /// The command palette, while open (D44); takes the keys.
    pub palette: Option<Palette>,
    /// Commands last run from the palette, most recent first.
    pub recent_commands: Vec<PaletteCommand>,
    /// `sent_at` of the last message sent from here; the next one is later, so
    /// two quick sends can't share an identity.
    pub last_sent_at: Timestamp,
    /// The config file as last read (or as changed from the palette), D48.
    /// Live values that can also change otherwise (`layout`, `list.allowlist`,
    /// `env.emoji`) are kept in step with it.
    pub config: Config,
    /// What was wrong with the config file last time, so the same problems
    /// aren't reported again on every reload.
    pub config_problems: Vec<String>,
    /// The user's theme files (D54), as last read.
    pub user_themes: UserThemes,
    /// Directory where downloaded and outgoing attachments are stored (D75, D77).
    pub attachments_dir: PathBuf,
    /// Attachments currently downloading (`<= MAX_DOWNLOADS_IN_FLIGHT`, D75).
    pub downloading: HashSet<(MessageRef, usize)>,
    /// Attachments waiting for a download slot (D75).
    pub download_queue: VecDeque<DownloadJob>,
    /// Attachments that should be copied to `~/Downloads/` as soon as their
    /// download finishes (when `s` was pressed on an undownloaded attachment).
    pub save_when_downloaded: HashSet<(MessageRef, usize)>,
    /// When the terminal bell last rang (`BELL_INTERVAL`, D49).
    pub last_bell: Option<Instant>,
    /// Ctrl+C was pressed once; a second press quits.
    pub quit_armed: bool,
    pub quit: bool,
    /// Off for a real account with no allowlist guard (`--no-allowlist`,
    /// D67); the footer says so.
    pub guard: Guard,
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
    /// Local file attached to the next message sent here (D77).
    pub staged_attachment: Option<StagedAttachment>,
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
    /// Clear the screen and draw everything again on the next frame (the
    /// app loop takes it).
    pub repaint: bool,
    pub emoji: EmojiSetting,
    /// Whether the terminal draws block, Powerline and Legacy Computing
    /// glyphs itself (`[messages] bubble_style = "auto"`, D58).
    pub glyphs: BlockGlyphs,
    /// Whether terminal graphics protocols can reach the user's terminal
    /// (`[media] inline_images = "auto"`, D78).
    pub graphics: GraphicsSupport,
    /// The colours the terminal shows, guessed at startup (D50); the
    /// config's `ui.colors` can override it (`State::colors`).
    pub colors: ColorDepth,
}

/// How emoji are drawn (D47).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EmojiMode {
    /// As they are: for terminals that measure emoji like faro (kitty,
    /// recent tmux).
    #[default]
    Full,
    /// Emoji whose width depends on the terminal are drawn in a simpler form
    /// with the same width everywhere (for mosh and other old-style terminals).
    Compatible,
}

/// What the terminal checks found at startup (D47, D58, D78).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Detected {
    pub emoji: EmojiSetting,
    pub glyphs: BlockGlyphs,
    pub graphics: GraphicsSupport,
}

/// Who draws the glyphs rounded bubbles are made of (D58).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BlockGlyphs {
    /// The font, which may not have them (tofu): not known to be safe.
    #[default]
    Font,
    /// The terminal itself, pixel-exact (kitty), for every terminal between
    /// faro and the user.
    Native,
}

/// Whether terminal graphics protocols (kitty, sixel, iTerm2) can reach the
/// user's terminal intact (`[media] inline_images = "auto"`, D78).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GraphicsSupport {
    /// Every hop between faro and the user supports graphics passthrough.
    #[default]
    Direct,
    /// mosh is in the way, tmux has no attached client, or tmux clients are
    /// on different terminal types: only Unicode half-blocks are safe.
    HalfblocksOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EmojiSetting {
    pub mode: EmojiMode,
    /// Follow mosh detection (`auto`); otherwise `mode` is fixed.
    pub auto: bool,
    /// Chosen with `--emoji` or `FARO_EMOJI`, which beat the config file.
    pub from_cli: bool,
    /// When the terminal was last checked (for mosh and for who draws block
    /// glyphs).
    pub checked_at: Option<Instant>,
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
    /// Larger inline preview of a downloaded image attachment (`v`, D75).
    ImageViewer { target: MessageRef, index: usize },
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
    /// Notify only for mentions, or for every message again (D49).
    MentionsOnly,
    AllMessages,
    Hide,
    Unhide,
    Archive,
    Unarchive,
    Allow,
    Disallow,
    /// Turn allowlist mode on (`true`) or off.
    AllowlistMode(bool),
    /// Give the person in a direct chat a nickname (D65).
    Nickname,
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
            Self::MentionsOnly => ("Notify for mentions only", "@"),
            Self::AllMessages => ("Notify for every message", "@"),
            Self::Hide => ("Hide", "h"),
            Self::Unhide => ("Unhide", "h"),
            Self::Archive => ("Archive", "a"),
            Self::Unarchive => ("Unarchive", "a"),
            Self::Allow => ("Add to allowlist", "l"),
            Self::Disallow => ("Remove from allowlist", "l"),
            Self::AllowlistMode(true) => ("Turn allowlist mode on", "L"),
            Self::AllowlistMode(false) => ("Turn allowlist mode off", "L"),
            Self::Nickname => ("Set a nickname…", ""),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MessageAction {
    Reply,
    React,
    Copy,
    OpenLink(String),
    DownloadAttachment { index: usize, name: String },
    ViewAttachment { index: usize, name: String },
    OpenAttachment { index: usize, name: String },
    SaveAttachment { index: usize, name: String },
    CopyAttachmentPath { index: usize, name: String },
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
            Self::DownloadAttachment { name, .. } => (format!("Download {name}"), "d"),
            Self::ViewAttachment { name, .. } => (format!("View {name}"), "v"),
            Self::OpenAttachment { name, .. } => (format!("Open {name} with system viewer"), "v"),
            Self::SaveAttachment { name, .. } => (format!("Save {name} to Downloads"), "s"),
            Self::CopyAttachmentPath { name, .. } => (format!("Copy path of {name}"), ""),
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
    /// The row highlighted while no conversation is open (D60): moved
    /// without opening anything, so nothing is read. See
    /// `State::highlighted`.
    pub cursor: Option<ConversationId>,
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
    /// The palette, while open.
    pub palette: Option<PaletteGeometry>,
    /// The logo was drawn moving (D61): it needs another frame soon.
    pub logo_animated: bool,
    /// Inline image slices that the frame wanted to draw but weren't in the
    /// cache yet (D78); decoded off-thread by `app::run`.
    pub images_wanted: Vec<super::images::ImageKey>,
}

/// Where the palette was drawn.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PaletteGeometry {
    /// The whole box, border included (clicks outside close it).
    pub area: Rect,
    /// The result rows.
    pub list: Rect,
    /// The first result shown.
    pub offset: usize,
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

/// How messages are laid out (DESIGN.md §7.3, D45); a setting since M5.
pub use faro_core::config::MessageLayout;

/// Whether a real account runs behind the allowlist guard (D62, D67). The
/// mock counts as `On`: there's nobody real to talk to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Guard {
    #[default]
    On,
    Off,
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
        let mut state = Self {
            me,
            contacts: contacts.into_iter().map(|c| (c.service_id, c)).collect(),
            groups: groups.into_iter().map(|g| (g.id, g)).collect(),
            nicknames: HashMap::new(),
            names_clash: HashSet::new(),
            conversations: Vec::new(),
            loading: true,
            open: None,
            connection: ConnectionState::Connecting,
            typing: HashMap::new(),
            panels: Panels {
                left: true,
                right: false,
            },
            layout: MessageLayout::default(),
            focus: Focus::Input,
            sidebar: SidebarScroll {
                offset: 0,
                reveal: true,
            },
            list: ConversationList {
                view: SidebarView::Chats,
                allowlist: false,
                cursor: None,
            },
            geometry: Geometry::default(),
            notice: None,
            popup: None,
            env: Environment {
                over_ssh: false,
                focused: true,
                repaint: false,
                emoji: EmojiSetting::default(),
                glyphs: BlockGlyphs::default(),
                graphics: GraphicsSupport::default(),
                colors: ColorDepth::default(),
            },
            keymap: Keymap::default(),
            palette: None,
            recent_commands: Vec::new(),
            last_sent_at: Timestamp::from_millis(0),
            config: Config::default(),
            config_problems: Vec::new(),
            user_themes: UserThemes::default(),
            attachments_dir: PathBuf::from("attachments"),
            downloading: HashSet::new(),
            download_queue: VecDeque::new(),
            save_when_downloaded: HashSet::new(),
            last_bell: None,
            quit_armed: bool::default(),
            quit: false,
            guard: Guard::On,
        };
        state.recount_names();
        state
    }

    /// With the nicknames the user gave people (D65).
    pub fn with_nicknames(mut self, nicknames: HashMap<ServiceId, String>) -> Self {
        self.nicknames = nicknames;
        self.recount_names();
        self
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

    /// Unread messages in conversations that notify (`notifies`): the
    /// footer's count and the window title's (D49).
    pub fn unread_total(&self, now: Timestamp) -> u32 {
        self.listed_in(SidebarView::Chats)
            .filter(|c| self.notifies(c, now))
            .map(|c| c.unread)
            .sum()
    }

    /// The window title (D49): "faro", with the unread total if there is
    /// one; `None` if the title is left to the terminal
    /// (`[notifications] title = false`).
    pub fn window_title(&self, now: Timestamp) -> Option<String> {
        self.config
            .notifications
            .title
            .then(|| match self.unread_total(now) {
                0 => "faro".to_owned(),
                unread => format!("faro ({unread})"),
            })
    }

    /// Whether new messages in `summary` ring the bell and count in the
    /// window title (D49): it's in the main list (not archived, hidden or
    /// left out by allowlist mode), not muted, and notifies for every
    /// message. "Mentions only" conversations ring for mentions of this
    /// account ([`State::rings_for`], D72) but don't count: the unread count
    /// doesn't know which messages mention us.
    pub fn notifies(&self, summary: &ConversationSummary, now: Timestamp) -> bool {
        self.listens(summary, now) && summary.state.notify == NotifyLevel::All
    }

    /// Whether `message`, new and unseen in `summary`, rings the bell: when
    /// the conversation [`notifies`](State::notifies), or is "Mentions only"
    /// and the message mentions this account (D72). A mute silences mentions
    /// too.
    pub fn rings_for(
        &self,
        summary: &ConversationSummary,
        message: &Message,
        now: Timestamp,
    ) -> bool {
        self.listens(summary, now)
            && match summary.state.notify {
                NotifyLevel::All => true,
                NotifyLevel::Mentions => message.mentions_person(self.me),
            }
    }

    /// In the main list and not muted: something in it may notify.
    fn listens(&self, summary: &ConversationSummary, now: Timestamp) -> bool {
        self.section(summary).view() == SidebarView::Chats && !summary.state.muted_at(now)
    }

    /// Name to show for a conversation: for a person, `person_name`; else
    /// the known title, then phone number, then a short form of the ID.
    pub fn conversation_title(&self, id: ConversationId) -> String {
        if let ConversationId::Direct(peer) = id
            && peer != self.me
        {
            return self.person_name(peer);
        }
        let summary = self.summary(id);
        if let Some(title) = summary.and_then(|s| s.title.as_deref()) {
            return title.to_owned();
        }
        match id {
            ConversationId::Direct(peer) => {
                if let Some(phone) = summary.and_then(|s| s.phone.as_ref()) {
                    return phone.expose().to_owned();
                }
                short_id(peer, 8)
            }
            ConversationId::Group(group) => self
                .groups
                .get(&group)
                .map_or_else(|| "Unnamed group".to_owned(), |g| g.title.clone()),
        }
    }

    /// A message's text as people read it: mentions written as `@Name`
    /// (D72). For copying and quoting; drawing styles the names too
    /// (`ui::messages::content`).
    pub fn readable_body(&self, message: &Message) -> Option<String> {
        let body = message.body.as_ref()?;
        let parts = mention_parts(body.expose(), &message.mentions);
        Some(
            parts
                .into_iter()
                .map(|part| match part {
                    BodyPart::Text(text) => text.to_owned(),
                    BodyPart::Mention(who) => self.mention_name(who),
                })
                .collect(),
        )
    }

    /// How a mention of `who` reads: `@` and their name ("@You" for us).
    pub fn mention_name(&self, who: ServiceId) -> String {
        format!("@{}", self.person_name(who))
    }

    /// Name to show for a message's sender.
    pub fn sender_name(&self, message: &Message) -> String {
        if matches!(message.direction, Direction::Outgoing(_)) || message.sender == self.me {
            return "You".to_owned();
        }
        self.person_name(message.sender)
    }

    /// "You"; else their name (`base_name`), with a short tag from their ID
    /// if someone else goes by the same name (D65); else the first
    /// characters of their ID.
    pub fn person_name(&self, id: ServiceId) -> String {
        if id == self.me {
            return "You".to_owned();
        }
        match self.base_name(id) {
            Some(name) if self.names_clash.contains(&name.to_lowercase()) => {
                format!("{name} · {}", short_id(id, 4))
            }
            Some(name) => name,
            None => short_id(id, 8),
        }
    }

    /// A person's name before clashes are told apart: their nickname (D65),
    /// else their address-book or profile name, else their phone number.
    pub fn base_name(&self, id: ServiceId) -> Option<String> {
        if let Some(nickname) = self.nicknames.get(&id) {
            return Some(nickname.clone());
        }
        let contact = self.contacts.get(&id);
        if let Some(name) = contact.and_then(Contact::display_name) {
            return Some(name.to_owned());
        }
        let summary = self.summary(ConversationId::Direct(id));
        summary.and_then(|s| s.title.clone()).or_else(|| {
            contact
                .and_then(|c| c.phone.as_ref())
                .or_else(|| summary.and_then(|s| s.phone.as_ref()))
                .map(|p| p.expose().to_owned())
        })
    }

    /// Everyone faro knows (contacts and people with a direct chat), but
    /// not this account.
    pub fn people(&self) -> Vec<ServiceId> {
        let mut people: Vec<ServiceId> = self.contacts.keys().copied().collect();
        people.extend(self.conversations.iter().filter_map(|c| match c.id {
            ConversationId::Direct(peer) => Some(peer),
            ConversationId::Group(_) => None,
        }));
        people.sort();
        people.dedup();
        people.retain(|id| *id != self.me);
        people
    }

    /// Find the names more than one person goes by again. Call after
    /// contacts, conversations or nicknames change.
    pub fn recount_names(&mut self) {
        let mut seen = HashSet::new();
        let mut clash = HashSet::new();
        for id in self.people() {
            if let Some(name) = self.base_name(id) {
                let key = name.to_lowercase();
                if !seen.insert(key.clone()) {
                    clash.insert(key);
                }
            }
        }
        self.names_clash = clash;
    }

    /// The conversation the sidebar highlights (and curation keys act on):
    /// the open one, else the cursor if it's listed, else the first listed
    /// (D60).
    pub fn highlighted(&self) -> Option<ConversationId> {
        if let Some(open) = &self.open {
            return Some(open.id);
        }
        let mut first = None;
        for summary in self.visible_conversations() {
            if Some(summary.id) == self.list.cursor {
                return Some(summary.id);
            }
            first = first.or(Some(summary.id));
        }
        first
    }

    /// The theme to draw with: the one being previewed in the palette
    /// (D54), else the config's.
    pub fn theme_name(&self) -> &str {
        self.palette
            .as_ref()
            .and_then(Palette::previewed_theme)
            .unwrap_or(&self.config.theme)
    }

    /// The setting value being tried on in the palette (D59).
    fn previewed(&self) -> Option<SettingChoice> {
        self.palette.as_ref().and_then(Palette::previewed_setting)
    }

    /// The colours to draw with: the config's `ui.colors` (or the value
    /// being previewed, D59), or what was detected (D50).
    pub fn colors(&self) -> ColorDepth {
        let choice = match self.previewed() {
            Some(SettingChoice::Colors(colors)) => colors,
            Some(_) | None => self.config.ui.colors,
        };
        match choice {
            ColorChoice::Auto => self.env.colors,
            ColorChoice::Truecolor => ColorDepth::Truecolor,
            ColorChoice::Colors256 => ColorDepth::Indexed256,
            ColorChoice::Colors16 => ColorDepth::Ansi16,
        }
    }

    /// The message layout to draw: the one being previewed (D59), else the
    /// one in use.
    pub fn message_layout(&self) -> MessageLayout {
        match self.previewed() {
            Some(SettingChoice::Layout(layout)) => layout,
            Some(_) | None => self.layout,
        }
    }

    /// The bubble style to draw: the one being previewed (D59), else the
    /// config's.
    pub fn bubble_style(&self) -> BubbleStyle {
        match self.previewed() {
            Some(SettingChoice::BubbleStyle(style)) => style,
            Some(_) | None => self.config.messages.bubble_style,
        }
    }

    /// Whether the logo should move (D61): it's on screen (nothing is open),
    /// the terminal has focus and `reduce_motion` is off. It also needs the
    /// last frame to have drawn its moving form (`Geometry::logo_animated`;
    /// small panes and 16 colours draw a still one).
    pub fn logo_moves(&self) -> bool {
        self.open.is_none() && self.env.focused && !self.config.reduce_motion
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

    /// Whether the `index`-th attachment on `message` is downloading or queued
    /// to download (D75).
    pub fn is_downloading(&self, message: MessageRef, index: usize) -> bool {
        let key = (message, index);
        self.downloading.contains(&key)
            || self
                .download_queue
                .iter()
                .any(|j| (j.message, j.index) == key)
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

/// The first `len` hex digits of someone's ID: who they are when faro knows
/// no name, or which one when names clash (D65).
fn short_id(id: ServiceId, len: usize) -> String {
    id.uuid().simple().to_string().chars().take(len).collect()
}
