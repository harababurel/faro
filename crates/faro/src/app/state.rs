//! Everything the UI shows. Changed only by the reducer (`update.rs`); read by the
//! views (`ui/`).

use std::collections::HashMap;
use std::time::Duration;

use faro_core::backend::BackendEvent;
use faro_core::domain::{
    ConnectionState, Contact, ConversationId, Direction, Group, GroupId, Message, MessageRef,
    ServiceId,
};
use faro_core::store::ConversationSummary;
use ratatui::layout::Rect;
use tokio::time::Instant;

use super::input::Input;

/// Signal clients stop showing a typing indicator after this long without an update.
pub const TYPING_TIMEOUT: Duration = Duration::from_secs(15);
/// A draft is saved this long after the last edit (and on switching or quitting).
pub const DRAFT_SAVE_DELAY: Duration = Duration::from_secs(1);

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
    /// Where things were on the last frame, for mouse hits and page sizes.
    pub geometry: Geometry,
    pub notice: Option<Notice>,
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
    /// The conversation on each sidebar row from the top (`None`: a header).
    pub sidebar_rows: Vec<Option<ConversationId>>,
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoticeLevel {
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
            geometry: Geometry::default(),
            notice: None,
            quit_armed: false,
            quit: false,
        }
    }

    /// Conversations shown in the sidebar: not hidden, not archived, not empty
    /// (DESIGN.md §7.7), plus the open one.
    pub fn visible_conversations(&self) -> impl Iterator<Item = &ConversationSummary> {
        let open = self.open.as_ref().map(|o| o.id);
        self.conversations.iter().filter(move |c| {
            Some(c.id) == open
                || (!c.state.hidden && !c.state.archived && c.last_activity.is_some())
        })
    }

    pub fn summary(&self, id: ConversationId) -> Option<&ConversationSummary> {
        self.conversations.iter().find(|c| c.id == id)
    }

    pub fn unread_total(&self) -> u32 {
        self.visible_conversations().map(|c| c.unread).sum()
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

    pub fn typists(&self, id: ConversationId) -> &[Typist] {
        self.typing.get(&id).map_or(&[], Vec::as_slice)
    }

    /// When the reducer next needs a `Tick` (a typing indicator expiring, a
    /// draft to save).
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
        typing.chain(draft).min()
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
