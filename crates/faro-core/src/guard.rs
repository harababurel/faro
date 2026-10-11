//! The allowlist guard (D62): a wrapper around any [`Backend`] that lets
//! faro talk only to the people and groups listed in an allowlist file
//! (`--allowlist`). Used while a real account is being tested, so that faro
//! (and whoever develops it) never sends anything to anyone else, and never
//! sees or stores anyone else's messages.
//!
//! - **Out:** sending, reacting, editing, deleting and typing indicators to a
//!   conversation that isn't allowed fail with [`BackendError::NotAllowed`];
//!   read receipts go only to allowed people and members of allowed groups.
//! - **In:** events about other conversations (messages, edits, deletions,
//!   reactions, typing), other people's contact details and other groups are
//!   dropped before they reach the app, so they're never shown or stored.
//!   Read syncs from the phone keep only pointers to messages from people
//!   faro knows (D70). Only their number is logged, never who or what.
//!
//! The file:
//!
//! ```toml
//! # Signal service IDs (ACI UUIDs) of people faro may talk to directly.
//! people = ["6f0c4b2e-8d1a-4c3b-9e5f-0a1b2c3d4e5f"]
//! # Group IDs (64 hex digits) faro may talk to; their members' names show.
//! groups = []
//! ```

use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, Mutex, PoisonError};

use tokio::sync::mpsc;

use crate::backend::{
    Account, Backend, BackendError, BackendEvent, BackendEvents, Capabilities,
    EVENT_CHANNEL_CAPACITY, OutgoingEdit, OutgoingMessage, ReactionChange, Result,
};
use crate::domain::{
    AttachmentRef, Contact, ConversationId, Group, GroupId, MessageRef, ServiceId, TypingState,
};

/// Who faro may talk to.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Allowlist {
    pub people: BTreeSet<ServiceId>,
    pub groups: BTreeSet<GroupId>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct AllowlistError(String);

impl Allowlist {
    /// Read an allowlist file. Anything unexpected is an error: a guard that
    /// guesses is no guard.
    pub fn parse(text: &str) -> Result<Self, AllowlistError> {
        let doc: toml_edit::DocumentMut = text
            .parse()
            .map_err(|e| AllowlistError(format!("not valid TOML: {e}")))?;
        let mut list = Self::default();
        for (key, item) in doc.iter() {
            if key != "people" && key != "groups" {
                return Err(AllowlistError(format!(
                    "unknown key `{key}` (expected `people` and `groups`)"
                )));
            }
            let values = item
                .as_array()
                .ok_or_else(|| AllowlistError(format!("`{key}` must be a list of strings")))?;
            for value in values {
                let text = value
                    .as_str()
                    .ok_or_else(|| AllowlistError(format!("`{key}` must be a list of strings")))?;
                if key == "people" {
                    let id = text.parse().map_err(|_| {
                        AllowlistError(format!(
                            "`{text}` in `people` isn't a Signal service ID (an ACI UUID)"
                        ))
                    })?;
                    list.people.insert(id);
                } else {
                    let id = text.parse().map_err(|_| {
                        AllowlistError(format!(
                            "`{text}` in `groups` isn't a group ID (64 hex digits)"
                        ))
                    })?;
                    list.groups.insert(id);
                }
            }
        }
        Ok(list)
    }

    pub fn allows(&self, conversation: ConversationId) -> bool {
        match conversation {
            ConversationId::Direct(peer) => self.people.contains(&peer),
            ConversationId::Group(group) => self.groups.contains(&group),
        }
    }
}

/// The allowlist and the members of allowed groups, as last seen; shared by
/// the command side and the event filter.
#[derive(Debug)]
struct Shared {
    list: Allowlist,
    members: Mutex<HashMap<GroupId, Vec<ServiceId>>>,
}

impl Shared {
    fn learn(&self, group: &Group) {
        if self.list.groups.contains(&group.id) {
            self.members
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .insert(group.id, group.members.clone());
        }
    }

    /// Listed, or a member of an allowed group.
    fn knows(&self, who: ServiceId) -> bool {
        self.list.people.contains(&who)
            || self
                .members
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .values()
                .any(|members| members.contains(&who))
    }

    fn admits(&self, event: &BackendEvent) -> bool {
        match event {
            // About the connection, or the status of our own messages (by
            // timestamp only; one we don't have changes nothing).
            BackendEvent::Connection(_)
            | BackendEvent::Error(_)
            | BackendEvent::DeliveryUpdate { .. } => true,
            BackendEvent::Message(message) => self.list.allows(message.conversation),
            BackendEvent::Reaction { conversation, .. }
            | BackendEvent::Edited { conversation, .. }
            | BackendEvent::Deleted { conversation, .. }
            | BackendEvent::Typing { conversation, .. } => self.list.allows(*conversation),
            BackendEvent::ContactUpdated(contact) => self.knows(contact.service_id),
            BackendEvent::GroupUpdated(group) => {
                self.learn(group);
                self.list.groups.contains(&group.id)
            }
            // Pointers to messages read on the phone (D70): see `admit`.
            BackendEvent::ReadElsewhere(messages) => messages.iter().any(|m| self.knows(m.sender)),
        }
    }

    /// The event as the app may see it, if at all. Read syncs (D70) carry no
    /// content, and the store ignores messages it doesn't have (it never
    /// stored anyone else's), so they couldn't reveal or change anything
    /// about others anyway; still, only pointers to messages from people
    /// faro knows are kept, so other people's IDs never reach the app.
    fn admit(&self, event: BackendEvent) -> Option<BackendEvent> {
        if let BackendEvent::ReadElsewhere(mut messages) = event {
            messages.retain(|m| self.knows(m.sender));
            return (!messages.is_empty()).then_some(BackendEvent::ReadElsewhere(messages));
        }
        self.admits(&event).then_some(event)
    }

    fn check(&self, conversation: ConversationId) -> Result<()> {
        if self.list.allows(conversation) {
            Ok(())
        } else {
            tracing::warn!("allowlist: blocked an outgoing action to a conversation not listed");
            Err(BackendError::NotAllowed)
        }
    }
}

/// `inner`, allowed to talk only to `list`.
pub struct Guarded<B> {
    inner: B,
    shared: Arc<Shared>,
}

/// Wrap `backend` and its `events` in the allowlist guard. The events that
/// come out are only those about allowed conversations and people.
pub fn guard<B: Backend>(
    backend: B,
    events: BackendEvents,
    list: Allowlist,
) -> (Guarded<B>, BackendEvents) {
    tracing::info!(
        people = list.people.len(),
        groups = list.groups.len(),
        "allowlist guard on: everyone else is blocked"
    );
    let shared = Arc::new(Shared {
        list,
        members: Mutex::new(HashMap::new()),
    });
    let (tx, rx) = mpsc::channel(EVENT_CHANNEL_CAPACITY);
    // Runs until either side closes; the handle isn't needed.
    let _forwarder = crate::rt::spawn(forward(events, tx, Arc::clone(&shared)));
    (
        Guarded {
            inner: backend,
            shared,
        },
        rx,
    )
}

async fn forward(mut events: BackendEvents, tx: mpsc::Sender<BackendEvent>, shared: Arc<Shared>) {
    let mut dropped: u64 = 0;
    while let Some(event) = events.recv().await {
        if let Some(event) = shared.admit(event) {
            if tx.send(event).await.is_err() {
                break;
            }
        } else {
            dropped += 1;
            tracing::debug!(
                dropped,
                "allowlist: dropped an event about someone not listed"
            );
        }
    }
}

impl<B: Backend> Backend for Guarded<B> {
    fn capabilities(&self) -> Capabilities {
        self.inner.capabilities()
    }

    async fn whoami(&self) -> Result<Account> {
        self.inner.whoami().await
    }

    /// Allowed people and members of allowed groups (the groups are read
    /// first, to know who's in them).
    async fn contacts(&self) -> Result<Vec<Contact>> {
        self.groups().await?;
        let contacts = self.inner.contacts().await?;
        Ok(contacts
            .into_iter()
            .filter(|c| self.shared.knows(c.service_id))
            .collect())
    }

    async fn groups(&self) -> Result<Vec<Group>> {
        let groups = self.inner.groups().await?;
        for group in &groups {
            self.shared.learn(group);
        }
        Ok(groups
            .into_iter()
            .filter(|g| self.shared.list.groups.contains(&g.id))
            .collect())
    }

    /// Only allowed conversations are asked about, and what comes back is
    /// filtered like live events.
    async fn stored_events(&self, conversations: &[ConversationId]) -> Result<Vec<BackendEvent>> {
        let allowed: Vec<ConversationId> = conversations
            .iter()
            .copied()
            .filter(|c| self.shared.list.allows(*c))
            .collect();
        if allowed.is_empty() {
            return Ok(Vec::new());
        }
        let events = self.inner.stored_events(&allowed).await?;
        let total = events.len();
        let admitted: Vec<BackendEvent> = events
            .into_iter()
            .filter_map(|e| self.shared.admit(e))
            .collect();
        if admitted.len() < total {
            tracing::debug!(
                dropped = total - admitted.len(),
                "allowlist: dropped stored events about someone not listed"
            );
        }
        Ok(admitted)
    }

    async fn send(&self, to: ConversationId, msg: OutgoingMessage) -> Result<()> {
        self.shared.check(to)?;
        self.inner.send(to, msg).await
    }

    async fn react(
        &self,
        conversation: ConversationId,
        target: MessageRef,
        change: ReactionChange,
    ) -> Result<()> {
        self.shared.check(conversation)?;
        self.inner.react(conversation, target, change).await
    }

    async fn edit(
        &self,
        conversation: ConversationId,
        target: MessageRef,
        edit: OutgoingEdit,
    ) -> Result<()> {
        self.shared.check(conversation)?;
        self.inner.edit(conversation, target, edit).await
    }

    async fn delete_for_everyone(
        &self,
        conversation: ConversationId,
        target: MessageRef,
    ) -> Result<()> {
        self.shared.check(conversation)?;
        self.inner.delete_for_everyone(conversation, target).await
    }

    /// Receipts go to each message's sender: only to people faro knows.
    async fn mark_read(&self, messages: &[MessageRef]) -> Result<()> {
        let allowed: Vec<MessageRef> = messages
            .iter()
            .copied()
            .filter(|m| self.shared.knows(m.sender))
            .collect();
        if allowed.len() < messages.len() {
            tracing::warn!(
                blocked = messages.len() - allowed.len(),
                "allowlist: blocked read receipts to people not listed"
            );
        }
        if allowed.is_empty() {
            return Ok(());
        }
        self.inner.mark_read(&allowed).await
    }

    async fn send_typing(&self, to: ConversationId, state: TypingState) -> Result<()> {
        self.shared.check(to)?;
        self.inner.send_typing(to, state).await
    }

    async fn fetch_attachment(&self, attachment: &AttachmentRef) -> Result<Vec<u8>> {
        self.inner.fetch_attachment(attachment).await
    }
}

#[cfg(test)]
mod tests;
