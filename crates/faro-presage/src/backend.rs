//! The [`Backend`] on a linked presage account, and the receive loop.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, SystemTime};

use faro_core::backend::{
    Account, Backend, BackendError, BackendEvent, BackendEvents, Capabilities, Capability,
    EVENT_CHANNEL_CAPACITY, OutgoingEdit, OutgoingMessage, Result,
};
use faro_core::domain::{
    AttachmentRef, ConnectionState, Contact, ConversationId, Group, GroupId, MessageRef,
    PhoneNumber, ServiceId, TypingState,
};
use futures::StreamExt as _;
use presage::libsignal_service::content::ContentBody;
use presage::libsignal_service::prelude::ServiceError;
use presage::libsignal_service::zkgroup::profiles::ProfileKey;
use presage::model::messages::Received;
use presage::store::{ContentsStore as _, Thread};
use presage_store_sqlite::SqliteStoreError;
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::Linked;
use crate::convert::{self, MasterKey};

/// Backoff between reconnection attempts: doubles from the first to the last.
const BACKOFF: (Duration, Duration) = (Duration::from_secs(1), Duration::from_secs(60));
/// How long a profile fetch may take before a contact is shown without it.
const PROFILE_TIMEOUT: Duration = Duration::from_secs(5);
/// presage fetches a new sender's profile in the background; look again then.
const PROFILE_RECHECK: Duration = Duration::from_secs(5);

/// A linked Signal account. Cheap to clone: presage's manager shares its
/// connections and store.
#[derive(Clone)]
pub struct PresageBackend {
    manager: Linked,
    me: ServiceId,
    /// Master key and revision of each group seen, by faro's group ID.
    groups: Rc<RefCell<HashMap<GroupId, (MasterKey, u32)>>>,
    /// For contacts whose profile arrives later.
    events: mpsc::Sender<BackendEvent>,
}

/// Start receiving for a linked account. Must run inside a `LocalSet`.
pub fn start(manager: Linked) -> (PresageBackend, BackendEvents) {
    let me = ServiceId::Aci(manager.registration_data().service_ids.aci);
    let (events, receiver) = mpsc::channel(EVENT_CHANNEL_CAPACITY);
    let backend = PresageBackend {
        manager,
        me,
        groups: Rc::default(),
        events: events.clone(),
    };
    tokio::task::spawn_local(receive(backend.clone(), events));
    (backend, receiver)
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .ok()
        .and_then(|d| u64::try_from(d.as_millis()).ok())
        .unwrap_or(0)
}

#[expect(
    clippy::wildcard_enum_match_arm,
    reason = "presage::Error is #[non_exhaustive]; anything else is a protocol error"
)]
fn backend_error(error: &presage::Error<SqliteStoreError>) -> BackendError {
    match error {
        presage::Error::ServiceError(service) => service_error(service),
        presage::Error::NotYetRegisteredError | presage::Error::RelinkNecessary => {
            BackendError::Unlinked
        }
        presage::Error::Store(store) => BackendError::Storage(store.to_string()),
        presage::Error::UnknownGroup | presage::Error::UnknownRecipient => BackendError::NotFound,
        presage::Error::Timeout(_)
        | presage::Error::IoError(_)
        | presage::Error::MessagePipeInterruptedError => BackendError::Network(error.to_string()),
        other => BackendError::Protocol(other.to_string()),
    }
}

fn service_error(error: &ServiceError) -> BackendError {
    if matches!(error, ServiceError::Unauthorized) {
        BackendError::Unlinked
    } else if let ServiceError::RateLimitExceeded { retry_after, .. } = error {
        BackendError::RateLimited {
            retry_after: retry_after
                .and_then(|d| u64::try_from(d.num_seconds()).ok())
                .map(Duration::from_secs),
        }
    } else if matches!(
        error,
        ServiceError::Timeout { .. }
            | ServiceError::IO(_)
            | ServiceError::WsError(_)
            | ServiceError::WsClosing { .. }
            | ServiceError::SendError { .. }
    ) {
        BackendError::Network(error.to_string())
    } else {
        BackendError::Protocol(error.to_string())
    }
}

impl PresageBackend {
    fn group_context(&self, group: GroupId) -> Result<presage::proto::GroupContextV2> {
        let (key, revision) = self
            .groups
            .borrow()
            .get(&group)
            .copied()
            .ok_or(BackendError::NotFound)?;
        Ok(convert::group_context(&key, revision))
    }

    /// Send any content to a conversation.
    async fn deliver(
        &self,
        to: ConversationId,
        content: impl Into<ContentBody>,
        sent_at: u64,
    ) -> Result<()> {
        let mut manager = self.manager.clone();
        let result = match to {
            ConversationId::Direct(peer) => {
                Box::pin(manager.send_message(convert::signal_id(peer), content, sent_at)).await
            }
            ConversationId::Group(group) => {
                let key = self
                    .groups
                    .borrow()
                    .get(&group)
                    .map(|(key, _)| *key)
                    .ok_or(BackendError::NotFound)?;
                Box::pin(manager.send_message_to_group(&key, content, sent_at)).await
            }
        };
        result.map_err(|e| {
            tracing::warn!(error = %e, "sending failed");
            backend_error(&e)
        })
    }

    fn group_of(&self, to: ConversationId) -> Result<Option<presage::proto::GroupContextV2>> {
        match to {
            ConversationId::Direct(_) => Ok(None),
            ConversationId::Group(group) => self.group_context(group).map(Some),
        }
    }

    /// A contact as faro sees it: the address-book name from the phone's
    /// contact sync, the profile name if presage has the profile.
    async fn contact(&self, id: ServiceId) -> Contact {
        let store = self.manager.store();
        let uuid = id.uuid();
        let synced = store
            .contact_by_id(&convert::signal_id(id))
            .await
            .ok()
            .flatten();
        let key = store
            .profile_key(&convert::signal_id(id))
            .await
            .ok()
            .flatten();
        let profile = match key {
            Some(key) => store.profile(uuid, key).await.ok().flatten(),
            None => None,
        };
        Contact {
            service_id: id,
            name: synced
                .as_ref()
                .map(|c| c.name.clone())
                .filter(|n| !n.trim().is_empty()),
            profile_name: profile.and_then(|p| p.name).map(|name| {
                match name.family_name.filter(|f| !f.is_empty()) {
                    Some(family) => format!("{} {family}", name.given_name),
                    None => name.given_name,
                }
            }),
            phone: synced.and_then(|c| c.phone_number).map(|p| {
                PhoneNumber::new(
                    p.format()
                        .mode(presage::libsignal_service::prelude::phonenumber::Mode::E164)
                        .to_string(),
                )
            }),
        }
    }

    /// Fetch a profile from Signal's servers if presage doesn't have it yet.
    /// `true` if one was fetched.
    async fn fetch_profile(&self, id: ServiceId) -> bool {
        let ServiceId::Aci(uuid) = id else {
            return false;
        };
        let store = self.manager.store();
        let Ok(Some(key)) = store.profile_key(&convert::signal_id(id)).await else {
            return false;
        };
        if matches!(store.profile(uuid, key).await, Ok(Some(_))) {
            return false;
        }
        let mut manager = self.manager.clone();
        let fetch = manager
            .retrieve_profile_by_uuid(presage::libsignal_service::protocol::Aci::from(uuid), key);
        match Box::pin(tokio::time::timeout(PROFILE_TIMEOUT, fetch)).await {
            Ok(Ok(_)) => true,
            Ok(Err(error)) => {
                tracing::debug!(%error, "fetching a profile failed");
                false
            }
            Err(_) => {
                tracing::debug!("fetching a profile timed out");
                false
            }
        }
    }

    /// Fetch missing profiles in the background and announce the contacts
    /// that got one, so startup never waits for the network.
    fn refresh_profiles(&self, ids: Vec<ServiceId>) {
        let backend = self.clone();
        tokio::task::spawn_local(async move {
            for id in ids {
                if backend.fetch_profile(id).await {
                    let contact = backend.contact(id).await;
                    if backend
                        .events
                        .send(BackendEvent::ContactUpdated(contact))
                        .await
                        .is_err()
                    {
                        return;
                    }
                }
            }
        });
    }

    async fn groups_from_store(&self) -> Result<Vec<Group>> {
        let groups = self
            .manager
            .store()
            .groups()
            .await
            .map_err(|e| BackendError::Storage(e.to_string()))?;
        let mut found = Vec::new();
        for entry in groups {
            let Ok((key, group)) = entry else { continue };
            let id = convert::group_id(&key);
            self.groups.borrow_mut().insert(id, (key, group.revision));
            for member in &group.members {
                self.save_profile_key(Uuid::from(member.aci), member.profile_key)
                    .await;
            }
            found.push(Group {
                id,
                title: group.title,
                members: group
                    .members
                    .iter()
                    .map(|m| ServiceId::Aci(Uuid::from(m.aci)))
                    .collect(),
            });
        }
        Ok(found)
    }

    /// Remember someone's profile key (a group's member list has every
    /// member's), so their profile name can be fetched. presage itself only
    /// learns keys from the phone's contact sync and from messages (D64).
    async fn save_profile_key(&self, uuid: Uuid, key: ProfileKey) {
        if ServiceId::Aci(uuid) == self.me {
            return;
        }
        let mut store = self.manager.store().clone();
        let known = store
            .profile_key(&convert::signal_id(ServiceId::Aci(uuid)))
            .await
            .ok()
            .flatten();
        if known.is_some_and(|k| k.bytes == key.bytes) {
            return;
        }
        if let Err(error) = store.upsert_profile_key(&uuid, key).await {
            tracing::debug!(%error, "saving a profile key failed");
        }
    }
}

impl Backend for PresageBackend {
    fn capabilities(&self) -> Capabilities {
        Capabilities::NONE
            .with(Capability::Edit)
            .with(Capability::DeleteForEveryone)
    }

    fn whoami(&self) -> impl Future<Output = Result<Account>> {
        let data = self.manager.registration_data();
        std::future::ready(Ok(Account {
            service_id: self.me,
            phone: Some(PhoneNumber::new(
                data.phone_number
                    .format()
                    .mode(presage::libsignal_service::prelude::phonenumber::Mode::E164)
                    .to_string(),
            )),
        }))
    }

    /// Contacts synced from the phone and the members of every group.
    async fn contacts(&self) -> Result<Vec<Contact>> {
        let synced = self
            .manager
            .store()
            .contacts()
            .await
            .map_err(|e| BackendError::Storage(e.to_string()))?;
        let mut ids: Vec<ServiceId> = synced
            .filter_map(std::result::Result::ok)
            .map(|c| ServiceId::Aci(c.uuid))
            .collect();
        for group in self.groups_from_store().await? {
            ids.extend(group.members);
        }
        ids.sort();
        ids.dedup();
        ids.retain(|id| *id != self.me);
        let mut contacts = Vec::with_capacity(ids.len());
        for id in &ids {
            contacts.push(self.contact(*id).await);
        }
        self.refresh_profiles(ids);
        Ok(contacts)
    }

    async fn groups(&self) -> Result<Vec<Group>> {
        self.groups_from_store().await
    }

    /// What presage kept in these conversations' threads (it saves every
    /// message it receives or sends), oldest first.
    async fn stored_events(&self, conversations: &[ConversationId]) -> Result<Vec<BackendEvent>> {
        let store = self.manager.store();
        let mut events = Vec::new();
        for &conversation in conversations {
            let thread = match conversation {
                ConversationId::Direct(peer) => Thread::Contact(convert::signal_id(peer)),
                ConversationId::Group(group) => {
                    let Some((key, _)) = self.groups.borrow().get(&group).copied() else {
                        continue;
                    };
                    Thread::Group(key)
                }
            };
            let contents = store
                .messages(&thread, ..)
                .await
                .map_err(|e| BackendError::Storage(e.to_string()))?;
            let mut contents: Vec<_> = contents.filter_map(std::result::Result::ok).collect();
            // presage lists the newest first.
            contents.reverse();
            for content in &contents {
                events.extend(convert::stored(content, self.me));
            }
        }
        Ok(events)
    }

    async fn send(&self, to: ConversationId, msg: OutgoingMessage) -> Result<()> {
        let message = convert::text(&msg, self.group_of(to)?);
        self.deliver(to, message, msg.sent_at.as_millis()).await
    }

    async fn react(
        &self,
        conversation: ConversationId,
        target: MessageRef,
        emoji: Option<String>,
    ) -> Result<()> {
        let now = now_millis();
        let message = convert::reaction(target, emoji, now, self.group_of(conversation)?);
        self.deliver(conversation, message, now).await
    }

    async fn edit(
        &self,
        conversation: ConversationId,
        target: MessageRef,
        edit: OutgoingEdit,
    ) -> Result<()> {
        let message = convert::edit(target, &edit, self.group_of(conversation)?);
        self.deliver(conversation, message, edit.edited_at.as_millis())
            .await
    }

    async fn delete_for_everyone(
        &self,
        conversation: ConversationId,
        target: MessageRef,
    ) -> Result<()> {
        let now = now_millis();
        let message = convert::delete(target, now, self.group_of(conversation)?);
        self.deliver(conversation, message, now).await
    }

    /// One read receipt per sender, for all their messages at once.
    async fn mark_read(&self, messages: &[MessageRef]) -> Result<()> {
        let mut by_sender: HashMap<ServiceId, Vec<u64>> = HashMap::new();
        for message in messages {
            by_sender
                .entry(message.sender)
                .or_default()
                .push(message.sent_at.as_millis());
        }
        for (sender, sent_at) in by_sender {
            if sender == self.me {
                continue;
            }
            let receipt = convert::read_receipt(sent_at);
            self.deliver(ConversationId::Direct(sender), receipt, now_millis())
                .await?;
        }
        Ok(())
    }

    async fn send_typing(&self, to: ConversationId, state: TypingState) -> Result<()> {
        let now = now_millis();
        let group = match to {
            ConversationId::Direct(_) => None,
            ConversationId::Group(group) => Some(group),
        };
        self.deliver(to, convert::typing(state, now, group), now)
            .await
    }

    fn fetch_attachment(
        &self,
        _attachment: &AttachmentRef,
    ) -> impl Future<Output = Result<PathBuf>> {
        std::future::ready(Err(BackendError::Unsupported(Capability::Attachments)))
    }
}

/// What the receive loop remembers between messages.
#[derive(Default)]
struct Seen {
    /// People announced to the app this run (as `ContactUpdated`).
    people: HashSet<ServiceId>,
}

/// Receive until the app goes away, reconnecting with backoff.
async fn receive(backend: PresageBackend, events: mpsc::Sender<BackendEvent>) {
    let mut seen = Seen::default();
    let mut backoff = BACKOFF.0;
    loop {
        if events
            .send(BackendEvent::Connection(ConnectionState::Connecting))
            .await
            .is_err()
        {
            return;
        }
        let mut manager = backend.manager.clone();
        match Box::pin(manager.receive_messages()).await {
            Ok(mut stream) => {
                tracing::info!("connected to Signal");
                if events
                    .send(BackendEvent::Connection(ConnectionState::Connected))
                    .await
                    .is_err()
                {
                    return;
                }
                backoff = BACKOFF.0;
                while let Some(received) = stream.next().await {
                    if !on_received(&backend, &events, &mut seen, received).await {
                        return;
                    }
                }
                tracing::info!("disconnected from Signal");
            }
            Err(error) => {
                let error = backend_error(&error);
                tracing::warn!(%error, "connecting to Signal failed");
                if error == BackendError::Unlinked {
                    let _ = events
                        .send(BackendEvent::Connection(ConnectionState::Unlinked))
                        .await;
                    let _ = events.send(BackendEvent::Error(error)).await;
                    return;
                }
                if events.send(BackendEvent::Error(error)).await.is_err() {
                    return;
                }
            }
        }
        if events
            .send(BackendEvent::Connection(ConnectionState::Disconnected))
            .await
            .is_err()
        {
            return;
        }
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(BACKOFF.1);
    }
}

/// Handle one item from presage. `false` once the app is gone.
async fn on_received(
    backend: &PresageBackend,
    events: &mpsc::Sender<BackendEvent>,
    seen: &mut Seen,
    received: Received,
) -> bool {
    match received {
        Received::QueueEmpty => {
            tracing::debug!("caught up with queued messages");
            true
        }
        Received::DecryptionError(_) => {
            tracing::warn!("a message couldn't be decrypted");
            true
        }
        Received::Contacts => {
            tracing::info!("contacts synced from the phone");
            let Ok(contacts) = backend.contacts().await else {
                return true;
            };
            for contact in contacts {
                seen.people.insert(contact.service_id);
                if events
                    .send(BackendEvent::ContactUpdated(contact))
                    .await
                    .is_err()
                {
                    return false;
                }
            }
            true
        }
        Received::Content(content) => {
            if let Some((key, revision)) = convert::group_of(&content)
                && !announce_group(backend, events, key, revision).await
            {
                return false;
            }
            let sender = convert::service_id(&content.metadata.sender);
            if sender != backend.me && seen.people.insert(sender) {
                if events
                    .send(BackendEvent::ContactUpdated(backend.contact(sender).await))
                    .await
                    .is_err()
                {
                    return false;
                }
                recheck_profile(backend.clone(), events.clone(), sender);
            }
            for event in convert::events(&content, backend.me) {
                if events.send(event).await.is_err() {
                    return false;
                }
            }
            true
        }
    }
}

/// Tell the app about a group the first time it's seen and when it changes.
async fn announce_group(
    backend: &PresageBackend,
    events: &mpsc::Sender<BackendEvent>,
    key: MasterKey,
    revision: u32,
) -> bool {
    let id = convert::group_id(&key);
    let known = backend.groups.borrow().get(&id).map(|(_, r)| *r);
    if known.is_some_and(|r| r >= revision) {
        return true;
    }
    // presage stored the group (fetched from the server) before handing
    // the message over.
    let Ok(Some(group)) = backend.manager.store().group(key).await else {
        return true;
    };
    for member in &group.members {
        backend
            .save_profile_key(Uuid::from(member.aci), member.profile_key)
            .await;
    }
    backend
        .groups
        .borrow_mut()
        .insert(id, (key, group.revision.max(revision)));
    events
        .send(BackendEvent::GroupUpdated(Group {
            id,
            title: group.title,
            members: group
                .members
                .iter()
                .map(|m| ServiceId::Aci(Uuid::from(m.aci)))
                .collect(),
        }))
        .await
        .is_ok()
}

/// presage fetches a new sender's profile in the background: announce the
/// contact again once it has had time to.
fn recheck_profile(backend: PresageBackend, events: mpsc::Sender<BackendEvent>, id: ServiceId) {
    tokio::task::spawn_local(async move {
        tokio::time::sleep(PROFILE_RECHECK).await;
        backend.fetch_profile(id).await;
        let _ = events
            .send(BackendEvent::ContactUpdated(backend.contact(id).await))
            .await;
    });
}
