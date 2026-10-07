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
use tokio::time::Instant;

/// Signal clients stop showing a typing indicator after this long without an update.
pub const TYPING_TIMEOUT: Duration = Duration::from_secs(15);

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
    /// Unsent input of conversations that aren't open (in memory until M4).
    pub drafts: HashMap<ConversationId, String>,
    pub connection: ConnectionState,
    pub typing: HashMap<ConversationId, Vec<Typist>>,
    pub panels: Panels,
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
    pub input: String,
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
            drafts: HashMap::new(),
            connection: ConnectionState::Connecting,
            typing: HashMap::new(),
            panels: Panels {
                left: true,
                right: false,
            },
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

    /// When the reducer next needs a `Tick` (a typing indicator expiring).
    pub fn next_deadline(&self) -> Option<Instant> {
        self.typing
            .values()
            .flatten()
            .map(|t| t.since + TYPING_TIMEOUT)
            .min()
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
