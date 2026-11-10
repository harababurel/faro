//! Link-time message history transfer (M11, D74, D164).
//!
//! When `faro link` advertises `capabilities=backup5` in the QR provisioning
//! URL and the user chooses *Transfer message history* on their phone, Signal's
//! `ProvisionMessage` supplies a single-use 32-byte `ephemeral_backup_key`.
//! The phone then builds a `backup.proto` archive, gzip-compresses it,
//! encrypts it with keys derived from `(ephemeral_backup_key, aci)` and
//! uploads it to Signal's transit CDN, notifying the new device via
//! `GET /v1/devices/transfer_archive`.
//!
//! All decryption, protobuf decoding and import into `presage.db` and
//! `faro.db` live here, using `presage`'s and `libsignal-service`'s public
//! APIs without forking either upstream repository.

use std::collections::HashMap;
use std::io::Read as _;
use std::time::{Duration, Instant};

use faro_core::backend::{BackendEvent, IdentityUpdate};
use faro_core::domain::{
    CallKind, CallState, Contact, ConversationId, DeliveryStatus, Direction, Group, GroupId,
    Mention, Message, MessageBody, MessageRef, PhoneNumber, Quote, Reaction, ServiceId, StyleSpan,
    SystemEvent, Timestamp, VerificationState,
};
use faro_core::guard::Allowlist;
use faro_core::store::{ConversationState, NotifyLevel, RemoteTranscription, Store};
use futures::AsyncReadExt as _;
use presage::libsignal_service::attachment_cipher::decrypt_in_place;
use presage::libsignal_service::configuration::Endpoint;
use presage::libsignal_service::groups_v2::Role;
use presage::libsignal_service::libsignal_account_keys::BackupKey;
use presage::libsignal_service::messagepipe::ServiceCredentials;
use presage::libsignal_service::prelude::{SessionStoreExt as _, Timer};
use presage::libsignal_service::proto::attachment_pointer::AttachmentIdentifier;
use presage::libsignal_service::proto::{AttachmentPointer, Verified};
use presage::libsignal_service::protocol::{
    Aci, DeviceId, IdentityKey, IdentityKeyStore as _, ProtocolAddress,
};
use presage::libsignal_service::push_service::{HttpAuthOverride, PushService};
use presage::libsignal_service::utils::TryIntoE164 as _;
use presage::libsignal_service::zkgroup::profiles::ProfileKey;
use presage::store::{ContentsStore as _, Store as _};
use presage_store_sqlite::SqliteStore;
use prost::Message as _;
use reqwest::Method;
use uuid::Uuid;

use crate::{Linked, convert};

/// Unencrypted header magic prefix used by some forward-secrecy backup files.
const SBACKUP_HEADER_MAGIC: &[u8; 8] = b"SBACKUP\x01";

/// HKDF info label for deriving `(hmac_key, aes_key)` from `ephemeral_backup_key`
/// and the 16-byte `backup_id` (`libsignal-message-backup`).
const HKDF_INFO_MESSAGE_BACKUP: &[u8] = b"20241007_SIGNAL_BACKUP_ENCRYPT_MESSAGE_BACKUP:";

/// Maximum uncompressed archive size we will read into memory (512 MiB).
const MAX_ARCHIVE_BYTES: u64 = 512 * 1024 * 1024;

/// Overall time `faro link` waits for the phone to finish uploading the
/// transfer archive before continuing without history.
const TRANSFER_TIMEOUT: Duration = Duration::from_secs(300);

/// Per-request HTTP timeout on the long-polling `transfer_archive` endpoint.
const POLL_REQUEST_TIMEOUT: Duration = Duration::from_secs(65);

/// Delay before retrying when `transfer_archive` returns immediately without a
/// payload.
const POLL_RETRY_DELAY: Duration = Duration::from_secs(2);

/// Progress milestones during `faro link` history transfer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferStage {
    /// Waiting for the primary device to build and upload the archive.
    WaitingForUpload,
    /// Downloading and decrypting the archive from the transit CDN.
    Downloading,
}

/// Outcome of polling and importing the link-time transfer archive.
#[derive(Debug)]
pub enum TransferOutcome {
    /// The phone uploaded a history archive and it was decrypted and parsed.
    Transferred(TransferredHistory),
    /// The user chose not to transfer history (or the phone skipped upload).
    ContinueWithoutUpload,
    /// The phone could not complete the transfer in time and asked the new
    /// device to relink.
    RelinkRequested,
}

/// Summary of what [`TransferredHistory::save_to_faro_store`] imported.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TransferSummary {
    pub conversations: usize,
    pub messages: usize,
    pub contacts: usize,
    pub groups: usize,
}

#[derive(Debug, thiserror::Error)]
pub enum TransferError {
    #[error("extracting registration credentials: {0}")]
    Credentials(String),
    #[error("talking to Signal's transfer archive endpoint: {0}")]
    Network(String),
    #[error("downloading the transfer archive from CDN: {0}")]
    Download(String),
    #[error("the transfer archive failed authentication or decryption")]
    Decrypt,
    #[error("decompressing the transfer archive: {0}")]
    Decompress(std::io::Error),
    #[error("the transfer archive exceeds the 512 MiB safety limit")]
    ArchiveTooLarge,
    #[error("decoding the transfer archive protobuf stream: {0}")]
    Protobuf(String),
}

/// Parsed contents of a link-time transfer archive, ready to write into
/// `presage.db` and `faro.db`.
#[derive(Debug, Default)]
pub struct TransferredHistory {
    pub backup_time_ms: u64,
    pub(crate) contacts: Vec<ParsedContact>,
    pub(crate) groups: Vec<ParsedGroup>,
    pub events: Vec<BackendEvent>,
    pub conversations: Vec<(ConversationId, ConversationState)>,
    pub nicknames: Vec<(ServiceId, String)>,
}

#[derive(Debug, Clone)]
pub(crate) struct ParsedContact {
    pub service_id: ServiceId,
    pub aci: Option<Uuid>,
    pub name: Option<String>,
    pub profile_name: Option<String>,
    pub phone: Option<PhoneNumber>,
    pub nickname: Option<String>,
    pub hidden: bool,
    pub profile_key: Option<[u8; 32]>,
    pub identity_key: Option<Vec<u8>>,
    pub verified: VerificationState,
    pub expire_timer: u32,
    pub expire_timer_version: u32,
}

#[derive(Debug, Clone)]
pub(crate) struct ParsedGroup {
    pub master_key: convert::MasterKey,
    pub id: GroupId,
    pub title: String,
    pub avatar: String,
    pub revision: u32,
    pub members: Vec<ServiceId>,
    pub member_entries: Vec<(Uuid, i32)>,
    pub disappearing_messages_timer: Option<u32>,
    pub invite_link_password: Vec<u8>,
    pub description: Option<String>,
}

/// Derive the 32-byte HMAC-SHA256 key and 32-byte AES-256-CBC key used to
/// authenticate and decrypt a Signal backup / link-time transfer archive.
pub fn derive_backup_keys(ephemeral_backup_key: &[u8; 32], aci: Uuid) -> ([u8; 32], [u8; 32]) {
    let key: BackupKey = BackupKey(*ephemeral_backup_key);
    let backup_id = key.derive_backup_id(&Aci::from(aci));
    let hkdf = hkdf::Hkdf::<sha2::Sha256>::new(None, ephemeral_backup_key);
    let mut okm = [0u8; 64];
    // `expand_multi_info` only fails when `okm.len() > 255 * 32`.
    let _ = hkdf.expand_multi_info(&[HKDF_INFO_MESSAGE_BACKUP, &backup_id.0], &mut okm);
    let mut hmac_key = [0u8; 32];
    let mut aes_key = [0u8; 32];
    if let Some(first) = okm.get(..32) {
        hmac_key.copy_from_slice(first);
    }
    if let Some(second) = okm.get(32..64) {
        aes_key.copy_from_slice(second);
    }
    (hmac_key, aes_key)
}

/// Authenticate, decrypt, gunzip and decode a transfer archive into a
/// [`TransferredHistory`] for account `me`.
pub fn decrypt_and_decode(
    mut encrypted: Vec<u8>,
    hmac_key: &[u8; 32],
    aes_key: &[u8; 32],
    me: ServiceId,
) -> Result<TransferredHistory, TransferError> {
    if encrypted.starts_with(SBACKUP_HEADER_MAGIC) {
        let header_len = sbackup_header_len(&encrypted).ok_or(TransferError::Decrypt)?;
        encrypted.drain(..header_len);
    }
    // At least 16-byte IV + 16-byte AES block + 32-byte HMAC-SHA256.
    if encrypted.len() < 64 {
        return Err(TransferError::Decrypt);
    }
    // `attachment_cipher::decrypt_in_place` expects `(aes_key || hmac_key)`.
    let mut cipher_key = [0u8; 64];
    if let Some(first) = cipher_key.get_mut(..32) {
        first.copy_from_slice(aes_key);
    }
    if let Some(second) = cipher_key.get_mut(32..64) {
        second.copy_from_slice(hmac_key);
    }
    decrypt_in_place(cipher_key, &mut encrypted).map_err(|_| TransferError::Decrypt)?;
    let mut decoder =
        flate2::read::GzDecoder::new(encrypted.as_slice()).take(MAX_ARCHIVE_BYTES + 1);
    let mut decompressed = Vec::new();
    decoder
        .read_to_end(&mut decompressed)
        .map_err(TransferError::Decompress)?;
    if u64::try_from(decompressed.len()).unwrap_or(u64::MAX) > MAX_ARCHIVE_BYTES {
        return Err(TransferError::ArchiveTooLarge);
    }
    decode_frames(&decompressed, me)
}

/// If `bytes` begins with `SBACKUP\x01`, return the byte offset where the
/// encrypted payload starts (after the 8-byte magic and varint-delimited
/// `MetadataPb` header).
fn sbackup_header_len(bytes: &[u8]) -> Option<usize> {
    let mut rest = bytes.get(SBACKUP_HEADER_MAGIC.len()..)?;
    let before = rest.len();
    let meta_len = usize::try_from(prost::encoding::decode_varint(&mut rest).ok()?).ok()?;
    let varint_len = before.checked_sub(rest.len())?;
    let total = SBACKUP_HEADER_MAGIC
        .len()
        .checked_add(varint_len)?
        .checked_add(meta_len)?;
    (total <= bytes.len()).then_some(total)
}

/// Decode a decompressed varint-delimited `BackupInfo` + `Frame*` stream.
pub fn decode_frames(mut buf: &[u8], me: ServiceId) -> Result<TransferredHistory, TransferError> {
    let info_bytes = read_varint_frame(&mut buf)?
        .ok_or_else(|| TransferError::Protobuf("missing BackupInfo frame".into()))?;
    let info = proto::BackupInfo::decode(info_bytes)
        .map_err(|e| TransferError::Protobuf(format!("BackupInfo: {e}")))?;

    let mut builder = HistoryBuilder::new(me, info.backup_time_ms);
    while let Some(frame_bytes) = read_varint_frame(&mut buf)? {
        let frame = proto::Frame::decode(frame_bytes)
            .map_err(|e| TransferError::Protobuf(format!("Frame: {e}")))?;
        builder.on_frame(frame);
    }
    Ok(builder.finish())
}

fn read_varint_frame<'a>(buf: &mut &'a [u8]) -> Result<Option<&'a [u8]>, TransferError> {
    if buf.is_empty() {
        return Ok(None);
    }
    let len = prost::encoding::decode_varint(buf)
        .map_err(|e| TransferError::Protobuf(format!("frame length varint: {e}")))?;
    let len = usize::try_from(len)
        .map_err(|_| TransferError::Protobuf("frame length overflow".into()))?;
    let Some(frame) = buf.get(..len) else {
        return Err(TransferError::Protobuf(
            "truncated frame in backup stream".into(),
        ));
    };
    *buf = buf.get(len..).unwrap_or_default();
    Ok(Some(frame))
}

#[derive(Debug, serde::Deserialize)]
struct TransferArchiveResponse {
    #[serde(default)]
    cdn: u32,
    #[serde(default)]
    key: Option<String>,
    #[serde(default)]
    error: Option<String>,
}

#[derive(serde::Deserialize)]
struct RegistrationPassword {
    password: String,
}

fn build_push_service(linked: &Linked) -> Result<PushService, TransferError> {
    let data = linked.registration_data();
    let value =
        serde_json::to_value(data).map_err(|e| TransferError::Credentials(e.to_string()))?;
    let parsed: RegistrationPassword =
        serde_json::from_value(value).map_err(|e| TransferError::Credentials(e.to_string()))?;
    let phonenumber = (&data.phone_number)
        .try_into_e164()
        .map_err(|_| TransferError::Credentials("invalid phone number".into()))?;
    let credentials = ServiceCredentials {
        aci: Some(data.service_ids.aci),
        pni: Some(data.service_ids.pni),
        phonenumber,
        password: Some(parsed.password),
        device_id: data.device_id.and_then(|d| d.try_into().ok()),
    };
    Ok(PushService::new(
        data.signal_servers,
        Some(credentials),
        concat!("faro/", env!("CARGO_PKG_VERSION")),
    ))
}

/// Poll `GET /v1/devices/transfer_archive`, download the archive from the
/// transit CDN, decrypt it with `ephemeral_backup_key`, and decode its frames.
pub async fn fetch_transfer_archive(
    linked: &Linked,
    ephemeral_backup_key: &[u8; 32],
    mut on_stage: impl FnMut(TransferStage),
) -> Result<TransferOutcome, TransferError> {
    let mut push = build_push_service(linked)?;
    let aci = linked.registration_data().service_ids.aci;
    let me = ServiceId::Aci(aci);

    on_stage(TransferStage::WaitingForUpload);
    let deadline = Instant::now() + TRANSFER_TIMEOUT;
    let (cdn, key) = loop {
        let now = Instant::now();
        if now >= deadline {
            tracing::warn!("timed out waiting for the phone to upload the transfer archive");
            return Ok(TransferOutcome::ContinueWithoutUpload);
        }
        let wait = POLL_REQUEST_TIMEOUT.min(deadline.saturating_duration_since(now));
        let request = push
            .request(
                Method::GET,
                Endpoint::service("/v1/devices/transfer_archive"),
                HttpAuthOverride::NoOverride,
            )
            .map_err(|e| TransferError::Network(e.to_string()))?
            .timeout(wait);

        let response = match request.send().await {
            Ok(resp) => resp,
            Err(error) if error.is_timeout() => continue,
            Err(error) => return Err(TransferError::Network(error.to_string())),
        };
        let status = response.status();
        if status == reqwest::StatusCode::NO_CONTENT || status == reqwest::StatusCode::ACCEPTED {
            tokio::time::sleep(POLL_RETRY_DELAY).await;
            continue;
        }
        if !status.is_success() {
            return Err(TransferError::Network(format!("HTTP {status}")));
        }
        let bytes = response
            .bytes()
            .await
            .map_err(|e| TransferError::Network(e.to_string()))?;
        if bytes.is_empty() {
            tokio::time::sleep(POLL_RETRY_DELAY).await;
            continue;
        }
        let meta: TransferArchiveResponse =
            serde_json::from_slice(&bytes).map_err(|e| TransferError::Network(e.to_string()))?;
        match meta.error.as_deref() {
            Some("RELINK_REQUESTED") => return Ok(TransferOutcome::RelinkRequested),
            Some("CONTINUE_WITHOUT_UPLOAD") => return Ok(TransferOutcome::ContinueWithoutUpload),
            Some(other) => {
                return Err(TransferError::Network(format!(
                    "phone reported transfer error: {other}"
                )));
            }
            None => {}
        }
        if let Some(key) = meta.key.filter(|k| !k.is_empty()) {
            break (meta.cdn, key);
        }
        tokio::time::sleep(POLL_RETRY_DELAY).await;
    };

    on_stage(TransferStage::Downloading);
    let pointer = AttachmentPointer {
        cdn_number: Some(cdn),
        attachment_identifier: Some(AttachmentIdentifier::CdnKey(key)),
        ..AttachmentPointer::default()
    };
    let mut download = push
        .get_attachment(&pointer)
        .await
        .map_err(|e| TransferError::Download(e.to_string()))?;
    let mut encrypted = Vec::new();
    download
        .stream
        .read_to_end(&mut encrypted)
        .await
        .map_err(|e| TransferError::Download(e.to_string()))?;
    let (hmac_key, aes_key) = derive_backup_keys(ephemeral_backup_key, aci);
    let history = decrypt_and_decode(encrypted, &hmac_key, &aes_key, me)?;
    Ok(TransferOutcome::Transferred(history))
}

impl TransferredHistory {
    /// Save transferred groups, contacts, profile keys and identity keys into
    /// `presage.db` so `faro ids` and `PresageBackend` know them right away.
    pub async fn save_to_presage_store(
        &self,
        store: &mut SqliteStore,
        me: ServiceId,
    ) -> Result<(), crate::Error> {
        for group in &self.groups {
            let members = group
                .member_entries
                .iter()
                .map(|&(uuid, role)| presage::model::groups::Member {
                    aci: Aci::from(uuid),
                    role: if role == 2 {
                        Role::Administrator
                    } else {
                        Role::Default
                    },
                    profile_key: ProfileKey::create([0u8; 32]),
                    joined_at_revision: 0,
                    label: None,
                    label_emoji: None,
                })
                .collect();
            let stored = presage::model::groups::Group {
                title: group.title.clone(),
                avatar: group.avatar.clone(),
                disappearing_messages_timer: group
                    .disappearing_messages_timer
                    .filter(|&d| d > 0)
                    .map(|duration| Timer { duration }),
                access_control: None,
                revision: group.revision,
                members,
                pending_members: vec![],
                requesting_members: vec![],
                invite_link_password: group.invite_link_password.clone(),
                description: group.description.clone(),
            };
            store.save_group(group.master_key, stored).await?;
        }

        let mut protocol_store = store.aci_protocol_store();
        for contact in &self.contacts {
            let Some(uuid) = contact.aci else {
                continue;
            };
            if ServiceId::Aci(uuid) == me {
                continue;
            }
            if let Some(key_bytes) = contact.profile_key {
                store
                    .upsert_profile_key(&uuid, ProfileKey::create(key_bytes))
                    .await?;
            }
            if let Some(key_bytes) = &contact.identity_key
                && let Ok(identity_key) = IdentityKey::decode(key_bytes)
                && let Ok(device_id) = DeviceId::try_from(1u32)
            {
                let address = ProtocolAddress::new(Aci::from(uuid).service_id_string(), device_id);
                let _ = protocol_store.save_identity(&address, &identity_key).await;
            }
            let phone_number = contact.phone.as_ref().and_then(|p| p.expose().parse().ok());
            let mut verified = Verified::default();
            verified.set_state(convert::proto_verification_state(contact.verified));
            if let Some(key_bytes) = &contact.identity_key {
                verified.identity_key = Some(key_bytes.clone());
            }
            let stored = presage::model::contacts::Contact {
                uuid,
                phone_number,
                name: contact.name.clone().unwrap_or_default(),
                verified,
                profile_key: contact.profile_key.map_or_else(Vec::new, |k| k.to_vec()),
                expire_timer: contact.expire_timer,
                expire_timer_version: contact.expire_timer_version.max(1),
                inbox_position: 0,
                avatar: None,
            };
            store.save_contact(&stored).await?;
        }
        Ok(())
    }

    /// Apply transferred contacts, groups, messages, receipts, read state,
    /// nicknames and conversation curation state to `faro.db`, filtered
    /// through `allowlist` when one is active.
    pub async fn save_to_faro_store(
        &self,
        store: &Store,
        linked_store: &SqliteStore,
        me: ServiceId,
        allowlist: Option<&Allowlist>,
    ) -> Result<TransferSummary, faro_core::store::StoreError> {
        let mut events = self.events.clone();
        // Add safety number / verification updates computed from the identity
        // keys saved in `presage.db` so `faro.db` starts with verified badges.
        let protocol_store = linked_store.aci_protocol_store();
        let local = convert::signal_id(me);
        for contact in &self.contacts {
            if contact.service_id == me || contact.identity_key.is_none() {
                continue;
            }
            let peer = convert::signal_id(contact.service_id);
            if let Ok(safety_number) = protocol_store.compute_safety_number(&local, &peer).await {
                events.push(BackendEvent::IdentityUpdated(IdentityUpdate {
                    service_id: contact.service_id,
                    safety_number,
                    verified: Some(contact.verified),
                }));
            }
        }

        let events = match allowlist {
            Some(list) => list.filter(events),
            None => events,
        };

        let mut summary = TransferSummary::default();
        for event in events {
            match &event {
                BackendEvent::Message(_) => summary.messages += 1,
                BackendEvent::ContactUpdated(_) => summary.contacts += 1,
                BackendEvent::GroupUpdated(_) => summary.groups += 1,
                BackendEvent::Connection(_)
                | BackendEvent::DeliveryUpdate { .. }
                | BackendEvent::Reaction { .. }
                | BackendEvent::Edited { .. }
                | BackendEvent::Deleted { .. }
                | BackendEvent::ReadElsewhere(_)
                | BackendEvent::IdentityUpdated(_)
                | BackendEvent::DisappearingTimer { .. }
                | BackendEvent::BlockedSynced { .. }
                | BackendEvent::MessageRequestResponded { .. }
                | BackendEvent::Typing { .. }
                | BackendEvent::Error(_) => {}
            }
            store.apply(event).await?;
        }

        for (conversation, state) in &self.conversations {
            if allowlist.is_some_and(|list| !list.allows(*conversation)) {
                continue;
            }
            summary.conversations += 1;
            store
                .set_conversation_state(*conversation, state.clone())
                .await?;
        }

        for (person, nickname) in &self.nicknames {
            if allowlist.is_some_and(|list| !list.allows(ConversationId::Direct(*person))) {
                continue;
            }
            store.set_nickname(*person, Some(nickname.clone())).await?;
        }

        Ok(summary)
    }
}

// ---- Frame mapping ----

#[derive(Debug, Clone)]
enum RecipientEntry {
    Self_,
    Contact(usize),
    Group(usize),
}

#[derive(Debug, Clone)]
struct ChatEntry {
    conversation: ConversationId,
    recipient_id: u64,
    state: ConversationState,
}

struct HistoryBuilder {
    me: ServiceId,
    backup_time_ms: u64,
    recipients: HashMap<u64, RecipientEntry>,
    contacts: Vec<ParsedContact>,
    groups: Vec<ParsedGroup>,
    chats: HashMap<u64, ChatEntry>,
    message_events: Vec<BackendEvent>,
    read_messages: Vec<MessageRef>,
}

impl HistoryBuilder {
    fn new(me: ServiceId, backup_time_ms: u64) -> Self {
        Self {
            me,
            backup_time_ms,
            recipients: HashMap::new(),
            contacts: Vec::new(),
            groups: Vec::new(),
            chats: HashMap::new(),
            message_events: Vec::new(),
            read_messages: Vec::new(),
        }
    }

    fn on_frame(&mut self, frame: proto::Frame) {
        let Some(item) = frame.item else {
            return;
        };
        match item {
            proto::frame::Item::Recipient(recipient) => self.on_recipient(recipient),
            proto::frame::Item::Chat(chat) => self.on_chat(&chat),
            proto::frame::Item::ChatItem(chat_item) => self.on_chat_item(chat_item),
            proto::frame::Item::Account(_) => {}
        }
    }

    fn on_recipient(&mut self, recipient: proto::Recipient) {
        let Some(dest) = recipient.destination else {
            return;
        };
        match dest {
            proto::recipient::Destination::Self_(_) => {
                self.recipients.insert(recipient.id, RecipientEntry::Self_);
            }
            proto::recipient::Destination::Contact(contact) => {
                self.on_contact(recipient.id, contact);
            }
            proto::recipient::Destination::Group(group) => {
                self.on_group(recipient.id, group);
            }
        }
    }

    fn on_contact(&mut self, recipient_id: u64, contact: proto::Contact) {
        let aci = contact
            .aci
            .as_deref()
            .and_then(|b| Uuid::from_slice(b).ok());
        let pni = contact
            .pni
            .as_deref()
            .and_then(|b| Uuid::from_slice(b).ok());
        let Some(service_id) = aci.map(ServiceId::Aci).or_else(|| pni.map(ServiceId::Pni)) else {
            return;
        };
        let name = join_name(&contact.system_given_name, &contact.system_family_name)
            .or_else(|| non_empty(&contact.system_nickname));
        let profile_name = join_name(
            contact.profile_given_name.as_deref().unwrap_or_default(),
            contact.profile_family_name.as_deref().unwrap_or_default(),
        );
        let phone = contact
            .e164
            .filter(|&n| n > 0)
            .map(|n| PhoneNumber::new(format!("+{n}")));
        let nickname = contact
            .nickname
            .as_ref()
            .and_then(|n| join_name(&n.given, &n.family));
        let hidden = matches!(contact.visibility, 1 | 2);
        let profile_key = contact
            .profile_key
            .as_deref()
            .and_then(|b| <[u8; 32]>::try_from(b).ok());
        let identity_key = contact.identity_key.filter(|k| !k.is_empty());
        let verified = if contact.identity_state == 1 {
            VerificationState::Verified
        } else {
            VerificationState::Default
        };
        let idx = self.contacts.len();
        self.contacts.push(ParsedContact {
            service_id,
            aci,
            name,
            profile_name,
            phone,
            nickname,
            hidden,
            profile_key,
            identity_key,
            verified,
            expire_timer: 0,
            expire_timer_version: 1,
        });
        self.recipients
            .insert(recipient_id, RecipientEntry::Contact(idx));
    }

    fn on_group(&mut self, recipient_id: u64, group: proto::Group) {
        let Ok(master_key) = <[u8; 32]>::try_from(group.master_key.as_slice()) else {
            return;
        };
        let id = convert::group_id(&master_key);
        let snapshot = group.snapshot.unwrap_or_default();
        let title = snapshot
            .title
            .and_then(|b| match b.content {
                Some(proto::group::group_attribute_blob::Content::Title(t)) => Some(t),
                Some(
                    proto::group::group_attribute_blob::Content::Avatar(_)
                    | proto::group::group_attribute_blob::Content::DisappearingMessagesDuration(_)
                    | proto::group::group_attribute_blob::Content::DescriptionText(_),
                )
                | None => None,
            })
            .unwrap_or_default();
        let description = snapshot.description.and_then(|b| match b.content {
            Some(proto::group::group_attribute_blob::Content::DescriptionText(d)) => non_empty(&d),
            Some(
                proto::group::group_attribute_blob::Content::Title(_)
                | proto::group::group_attribute_blob::Content::Avatar(_)
                | proto::group::group_attribute_blob::Content::DisappearingMessagesDuration(_),
            )
            | None => None,
        });
        let disappearing_messages_timer =
            snapshot
                .disappearing_messages_timer
                .and_then(|b| match b.content {
                    Some(
                        proto::group::group_attribute_blob::Content::DisappearingMessagesDuration(
                            secs,
                        ),
                    ) => Some(secs),
                    Some(
                        proto::group::group_attribute_blob::Content::Title(_)
                        | proto::group::group_attribute_blob::Content::Avatar(_)
                        | proto::group::group_attribute_blob::Content::DescriptionText(_),
                    )
                    | None => None,
                });
        let mut members = Vec::new();
        let mut member_entries = Vec::new();
        for member in snapshot.members {
            if let Ok(uuid) = Uuid::from_slice(&member.user_id) {
                members.push(ServiceId::Aci(uuid));
                member_entries.push((uuid, member.role));
            }
        }
        let idx = self.groups.len();
        self.groups.push(ParsedGroup {
            master_key,
            id,
            title,
            avatar: snapshot.avatar_url,
            revision: snapshot.version,
            members,
            member_entries,
            disappearing_messages_timer,
            invite_link_password: snapshot.invite_link_password,
            description,
        });
        self.recipients
            .insert(recipient_id, RecipientEntry::Group(idx));
    }

    fn on_chat(&mut self, chat: &proto::Chat) {
        let (conversation, hidden) = match self.recipients.get(&chat.recipient_id) {
            Some(RecipientEntry::Self_) => (ConversationId::Direct(self.me), false),
            Some(&RecipientEntry::Contact(idx)) => {
                let Some(contact) = self.contacts.get_mut(idx) else {
                    return;
                };
                if let Some(ms) = chat.expiration_timer_ms {
                    contact.expire_timer = u32::try_from(ms / 1000).unwrap_or(u32::MAX);
                    contact.expire_timer_version = chat.expire_timer_version.max(1);
                }
                (ConversationId::Direct(contact.service_id), contact.hidden)
            }
            Some(&RecipientEntry::Group(idx)) => {
                let Some(group) = self.groups.get(idx) else {
                    return;
                };
                (ConversationId::Group(group.id), false)
            }
            None => return,
        };
        let muted_until = chat.mute_until_ms.filter(|&ms| ms > 0).map(|ms| {
            let capped = ms.min(ConversationState::MUTED_FOREVER.as_millis());
            Timestamp::from_millis(capped)
        });
        let notify = if chat.dont_notify_for_mentions_if_muted {
            NotifyLevel::All
        } else {
            NotifyLevel::default()
        };
        let state = ConversationState {
            hidden,
            archived: chat.archived,
            pinned: chat.pinned_order,
            muted_until,
            draft: None,
            allowed: false,
            notify,
            remote_transcription: RemoteTranscription::Disallowed,
            request: false,
            blocked: false,
        };
        self.chats.insert(
            chat.id,
            ChatEntry {
                conversation,
                recipient_id: chat.recipient_id,
                state,
            },
        );
    }

    fn recipient_service_id(&self, recipient_id: u64) -> Option<ServiceId> {
        match self.recipients.get(&recipient_id)? {
            RecipientEntry::Self_ => Some(self.me),
            RecipientEntry::Contact(idx) => Some(self.contacts.get(*idx)?.service_id),
            RecipientEntry::Group(_) => None,
        }
    }

    fn on_chat_item(&mut self, item: proto::ChatItem) {
        let Some(chat) = self.chats.get(&item.chat_id) else {
            return;
        };
        let conversation = chat.conversation;
        let chat_recipient_id = chat.recipient_id;
        let Some(sender) = self.recipient_service_id(item.author_id) else {
            return;
        };

        // When `revisions` is non-empty, `revisions[0].date_sent` is the
        // original send timestamp (`sent_at`), and `item.date_sent` is the
        // latest edit timestamp (`edited_at`).
        let (sent_at_ms, edited_at) = match item.revisions.first() {
            Some(oldest) => (
                oldest.date_sent,
                Some(Timestamp::from_millis(item.date_sent)),
            ),
            None => (item.date_sent, None),
        };
        let sent_at = Timestamp::from_millis(sent_at_ms);

        let mut receipt_updates = Vec::new();
        let direction = match item.directional_details {
            Some(proto::chat_item::DirectionalDetails::Incoming(incoming)) => {
                if incoming.read {
                    self.read_messages.push(MessageRef { sender, sent_at });
                }
                Direction::Incoming
            }
            Some(proto::chat_item::DirectionalDetails::Outgoing(outgoing)) => {
                let (status, updates) = self.outgoing_status(
                    &outgoing.send_status,
                    MessageRef {
                        sender: self.me,
                        sent_at,
                    },
                    chat_recipient_id,
                );
                receipt_updates = updates;
                Direction::Outgoing(status)
            }
            Some(proto::chat_item::DirectionalDetails::Directionless(_)) => {
                if sender == self.me {
                    Direction::Outgoing(DeliveryStatus::Sent)
                } else {
                    Direction::Incoming
                }
            }
            None => return,
        };

        let Some(content) = item.item else {
            return;
        };
        if let proto::chat_item::Item::UpdateMessage(update) = content {
            for msg in self.update_messages(&update, conversation, sender, sent_at, direction) {
                self.message_events.push(BackendEvent::Message(msg));
            }
            return;
        }
        let expires_in_secs = item
            .expires_in_ms
            .map(|ms| u32::try_from(ms / 1_000).unwrap_or(u32::MAX))
            .filter(|&s| s > 0);
        let expire_started_at = expires_in_secs.and_then(|_| {
            item.expire_start_date
                .filter(|&ms| ms > 0)
                .map(Timestamp::from_millis)
                .or_else(|| matches!(direction, Direction::Outgoing(_)).then_some(sent_at))
        });
        let Some(mut message) =
            self.message_from_item(content, conversation, sender, sent_at, direction, edited_at)
        else {
            return;
        };
        message.expires_in_secs = expires_in_secs;
        message.expire_started_at = expire_started_at;

        self.message_events.push(BackendEvent::Message(message));
        self.message_events.extend(receipt_updates);
    }

    fn message_from_item(
        &self,
        content: proto::chat_item::Item,
        conversation: ConversationId,
        sender: ServiceId,
        sent_at: Timestamp,
        direction: Direction,
        edited_at: Option<Timestamp>,
    ) -> Option<Message> {
        match content {
            proto::chat_item::Item::Standard(standard) => {
                let body = standard.text.as_ref().and_then(backup_body);
                let mentions = standard
                    .text
                    .as_ref()
                    .map(|t| backup_mentions(&t.body, &t.body_ranges))
                    .unwrap_or_default();
                let quote = standard.quote.as_ref().and_then(|q| self.backup_quote(q));
                let attachments: Vec<_> = standard
                    .attachments
                    .iter()
                    .filter_map(backup_attachment)
                    .collect();
                if body.is_none() && attachments.is_empty() {
                    return None;
                }
                let reactions = self.backup_reactions(&standard.reactions);
                Some(Message {
                    conversation,
                    sender,
                    sent_at,
                    direction,
                    body,
                    mentions,
                    quote,
                    attachments,
                    reactions,
                    edited_at,
                    deleted: false,
                    expires_in_secs: None,
                    expire_started_at: None,
                    system_event: None,
                })
            }
            proto::chat_item::Item::Sticker(sticker_msg) => {
                let sticker = sticker_msg.sticker?;
                let emoji = sticker.emoji.as_deref().unwrap_or("");
                let body = Some(MessageBody::new(format!("[sticker] {emoji}").trim_end()));
                let attachments = sticker
                    .data
                    .as_ref()
                    .and_then(|fp| file_pointer_to_attachment_pointer(fp, 0))
                    .map(|p| convert::sticker_attachment(&p))
                    .into_iter()
                    .collect();
                let reactions = self.backup_reactions(&sticker_msg.reactions);
                Some(Message {
                    conversation,
                    sender,
                    sent_at,
                    direction,
                    body,
                    mentions: vec![],
                    quote: None,
                    attachments,
                    reactions,
                    edited_at: None,
                    deleted: false,
                    expires_in_secs: None,
                    expire_started_at: None,
                    system_event: None,
                })
            }
            proto::chat_item::Item::RemoteDeleted(_) => Some(Message {
                conversation,
                sender,
                sent_at,
                direction,
                body: None,
                mentions: vec![],
                quote: None,
                attachments: vec![],
                reactions: vec![],
                edited_at: None,
                deleted: true,
                expires_in_secs: None,
                expire_started_at: None,
                system_event: None,
            }),
            proto::chat_item::Item::UpdateMessage(_) => None,
        }
    }

    fn update_messages(
        &self,
        update: &proto::ChatUpdateMessage,
        conversation: ConversationId,
        default_sender: ServiceId,
        sent_at: Timestamp,
        default_direction: Direction,
    ) -> Vec<Message> {
        let Some(body) = &update.update else {
            return vec![];
        };
        let direction_for = |who: ServiceId| {
            if who == self.me {
                Direction::Outgoing(DeliveryStatus::Sent)
            } else {
                Direction::Incoming
            }
        };
        match body {
            proto::chat_update_message::Update::ExpirationTimerChange(timer) => {
                let seconds = u32::try_from(timer.expires_in_ms / 1_000)
                    .ok()
                    .filter(|&s| s > 0);
                vec![Message::system(
                    conversation,
                    default_sender,
                    sent_at,
                    default_direction,
                    SystemEvent::DisappearingTimer { seconds },
                )]
            }
            proto::chat_update_message::Update::IndividualCall(call) => {
                let kind = if call.r#type == 2 {
                    CallKind::Video
                } else {
                    CallKind::Voice
                };
                let state = if call.state == 1 {
                    CallState::Accepted
                } else {
                    CallState::Missed
                };
                let direction = if call.direction == 2 {
                    Direction::Outgoing(DeliveryStatus::Sent)
                } else {
                    Direction::Incoming
                };
                let call_ts = if call.started_call_timestamp > 0 {
                    Timestamp::from_millis(call.started_call_timestamp)
                } else {
                    sent_at
                };
                vec![Message::system(
                    conversation,
                    default_sender,
                    call_ts,
                    direction,
                    SystemEvent::Call { kind, state },
                )]
            }
            proto::chat_update_message::Update::GroupCall(call) => {
                let state = if call.state == 4 || call.state == 5 {
                    CallState::Accepted
                } else {
                    CallState::Missed
                };
                let who = call
                    .ringer_recipient_id
                    .and_then(|id| self.recipient_service_id(id))
                    .unwrap_or(default_sender);
                let call_ts = if call.started_call_timestamp > 0 {
                    Timestamp::from_millis(call.started_call_timestamp)
                } else {
                    sent_at
                };
                vec![Message::system(
                    conversation,
                    who,
                    call_ts,
                    direction_for(who),
                    SystemEvent::Call {
                        kind: CallKind::Group,
                        state,
                    },
                )]
            }
            proto::chat_update_message::Update::GroupChange(change) => {
                self.group_change_messages(change, conversation, default_sender, sent_at)
            }
        }
    }

    fn group_change_messages(
        &self,
        change: &proto::GroupChangeChatUpdate,
        conversation: ConversationId,
        default_sender: ServiceId,
        sent_at: Timestamp,
    ) -> Vec<Message> {
        use proto::group_change_chat_update::update::Body;

        let direction_for = |who: ServiceId| {
            if who == self.me {
                Direction::Outgoing(DeliveryStatus::Sent)
            } else {
                Direction::Incoming
            }
        };
        let aci_or_default = |bytes: Option<&[u8]>| {
            bytes
                .and_then(|b| Uuid::from_slice(b).ok())
                .map_or(default_sender, ServiceId::Aci)
        };
        let mut out = Vec::new();
        for (idx, entry) in change.updates.iter().enumerate() {
            let Some(update_body) = &entry.body else {
                continue;
            };
            let ts = Timestamp::from_millis(sent_at.as_millis().saturating_add(idx as u64));
            let parsed = match update_body {
                Body::GroupNameUpdate(rename) => rename
                    .new_group_name
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(|title| {
                        (
                            aci_or_default(rename.updater_aci.as_deref()),
                            SystemEvent::GroupRenamed {
                                title: title.to_owned(),
                            },
                        )
                    }),
                Body::GroupExpirationTimerUpdate(timer) => {
                    let seconds = u32::try_from(timer.expires_in_ms / 1_000)
                        .ok()
                        .filter(|&s| s > 0);
                    Some((
                        aci_or_default(timer.updater_aci.as_deref()),
                        SystemEvent::DisappearingTimer { seconds },
                    ))
                }
                Body::GroupMemberAddedUpdate(added) => {
                    Uuid::from_slice(&added.new_member_aci).ok().map(|u| {
                        (
                            aci_or_default(added.updater_aci.as_deref()),
                            SystemEvent::MembersJoined {
                                members: vec![ServiceId::Aci(u)],
                            },
                        )
                    })
                }
                Body::GroupMemberJoinedUpdate(joined)
                | Body::GroupMemberJoinedByLinkUpdate(joined) => {
                    Uuid::from_slice(&joined.new_member_aci).ok().map(|u| {
                        let member = ServiceId::Aci(u);
                        (
                            member,
                            SystemEvent::MembersJoined {
                                members: vec![member],
                            },
                        )
                    })
                }
                Body::GroupMemberRemovedUpdate(removed) => {
                    Uuid::from_slice(&removed.removed_aci).ok().map(|u| {
                        (
                            aci_or_default(removed.remover_aci.as_deref()),
                            SystemEvent::MembersLeft {
                                members: vec![ServiceId::Aci(u)],
                            },
                        )
                    })
                }
                Body::GroupMemberLeftUpdate(left) => Uuid::from_slice(&left.aci).ok().map(|u| {
                    let member = ServiceId::Aci(u);
                    (
                        member,
                        SystemEvent::MembersLeft {
                            members: vec![member],
                        },
                    )
                }),
            };
            if let Some((who, event)) = parsed {
                out.push(Message::system(
                    conversation,
                    who,
                    ts,
                    direction_for(who),
                    event,
                ));
            }
        }
        out
    }

    fn outgoing_status(
        &self,
        statuses: &[proto::SendStatus],
        message: MessageRef,
        chat_recipient_id: u64,
    ) -> (DeliveryStatus, Vec<BackendEvent>) {
        // Note to Self has no external recipients; treat as Read.
        if matches!(
            self.recipients.get(&chat_recipient_id),
            Some(RecipientEntry::Self_)
        ) && statuses.is_empty()
        {
            return (DeliveryStatus::Read, Vec::new());
        }
        let mut summary = DeliveryStatus::Sent;
        let mut any_non_failed = false;
        let mut any_failed = false;
        let mut updates = Vec::new();

        for entry in statuses {
            let status = match &entry.delivery_status {
                Some(
                    proto::send_status::DeliveryStatus::Read(_)
                    | proto::send_status::DeliveryStatus::Viewed(_),
                ) => {
                    any_non_failed = true;
                    Some(DeliveryStatus::Read)
                }
                Some(proto::send_status::DeliveryStatus::Delivered(_)) => {
                    any_non_failed = true;
                    Some(DeliveryStatus::Delivered)
                }
                Some(
                    proto::send_status::DeliveryStatus::Sent(_)
                    | proto::send_status::DeliveryStatus::Pending(_),
                )
                | None => {
                    any_non_failed = true;
                    Some(DeliveryStatus::Sent)
                }
                Some(proto::send_status::DeliveryStatus::Skipped(_)) => None,
                Some(proto::send_status::DeliveryStatus::Failed(_)) => {
                    any_failed = true;
                    None
                }
            };
            if let Some(st) = status {
                summary = summary.update(st);
                if matches!(st, DeliveryStatus::Delivered | DeliveryStatus::Read)
                    && let Some(by) = self.recipient_service_id(entry.recipient_id)
                    && by != self.me
                {
                    let at = if entry.timestamp > 0 {
                        entry.timestamp
                    } else {
                        message.sent_at.as_millis()
                    };
                    updates.push(BackendEvent::DeliveryUpdate {
                        message,
                        status: st,
                        by: Some(by),
                        updated_at: Some(Timestamp::from_millis(at)),
                    });
                }
            }
        }
        if !any_non_failed && any_failed {
            summary = DeliveryStatus::Failed;
        }
        (summary, updates)
    }

    fn backup_quote(&self, quote: &proto::Quote) -> Option<Quote> {
        let sent_at = Timestamp::from_millis(quote.target_sent_timestamp?);
        let sender = self.recipient_service_id(quote.author_id)?;
        let preview = quote.text.as_ref().and_then(backup_body);
        Some(Quote {
            target: MessageRef { sender, sent_at },
            preview,
        })
    }

    fn backup_reactions(&self, reactions: &[proto::Reaction]) -> Vec<Reaction> {
        let mut sorted: Vec<&proto::Reaction> =
            reactions.iter().filter(|r| !r.emoji.is_empty()).collect();
        sorted.sort_by_key(|r| (r.sort_order, r.sent_timestamp));
        let mut out: Vec<Reaction> = Vec::with_capacity(sorted.len());
        for reaction in sorted {
            let Some(sender) = self.recipient_service_id(reaction.author_id) else {
                continue;
            };
            if let Some(existing) = out.iter_mut().find(|r| r.sender == sender) {
                existing.emoji.clone_from(&reaction.emoji);
            } else {
                out.push(Reaction {
                    sender,
                    emoji: reaction.emoji.clone(),
                });
            }
        }
        out
    }

    fn finish(self) -> TransferredHistory {
        let mut events = Vec::with_capacity(
            self.groups.len() + self.contacts.len() + self.message_events.len() + 1,
        );
        // Groups first so `Allowlist::filter` learns members of allowed groups
        // before filtering contacts, messages and read state.
        for group in &self.groups {
            events.push(BackendEvent::GroupUpdated(Group {
                id: group.id,
                title: group.title.clone(),
                members: group.members.clone(),
            }));
            if let Some(secs) = group.disappearing_messages_timer.filter(|&d| d > 0) {
                events.push(BackendEvent::DisappearingTimer {
                    conversation: ConversationId::Group(group.id),
                    seconds: Some(secs),
                });
            }
        }
        for contact in &self.contacts {
            if contact.service_id == self.me {
                continue;
            }
            events.push(BackendEvent::ContactUpdated(Contact {
                service_id: contact.service_id,
                name: contact.name.clone(),
                profile_name: contact.profile_name.clone(),
                phone: contact.phone.clone(),
            }));
            if contact.expire_timer > 0 {
                events.push(BackendEvent::DisappearingTimer {
                    conversation: ConversationId::Direct(contact.service_id),
                    seconds: Some(contact.expire_timer),
                });
            }
        }
        events.extend(self.message_events);
        if !self.read_messages.is_empty() {
            events.push(BackendEvent::ReadElsewhere(self.read_messages));
        }

        let mut conversations: Vec<(ConversationId, ConversationState)> = self
            .chats
            .into_values()
            .filter(|c| {
                c.state.hidden
                    || c.state.archived
                    || c.state.pinned.is_some()
                    || c.state.muted_until.is_some()
            })
            .map(|c| (c.conversation, c.state))
            .collect();
        conversations.sort_by_key(|(id, _)| *id);

        let nicknames: Vec<(ServiceId, String)> = self
            .contacts
            .iter()
            .filter_map(|c| Some((c.service_id, c.nickname.clone()?)))
            .collect();

        TransferredHistory {
            backup_time_ms: self.backup_time_ms,
            contacts: self.contacts,
            groups: self.groups,
            events,
            conversations,
            nicknames,
        }
    }
}

fn non_empty(text: &str) -> Option<String> {
    let trimmed = text.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

fn join_name(given: &str, family: &str) -> Option<String> {
    match (given.trim(), family.trim()) {
        ("", "") => None,
        (g, "") => Some(g.to_owned()),
        ("", f) => Some(f.to_owned()),
        (g, f) => Some(format!("{g} {f}")),
    }
}

fn backup_body(text: &proto::Text) -> Option<MessageBody> {
    if text.body.is_empty() {
        return None;
    }
    Some(MessageBody::with_styles(
        text.body.as_str(),
        backup_styles(&text.body, &text.body_ranges),
    ))
}

fn backup_styles(body: &str, ranges: &[proto::BodyRange]) -> Vec<StyleSpan> {
    ranges
        .iter()
        .filter_map(|range| {
            let Some(proto::body_range::AssociatedValue::Style(raw)) = &range.associated_value
            else {
                return None;
            };
            let style = convert::text_style(*raw)?;
            let start_utf16 = usize::try_from(range.start).ok()?;
            let len_utf16 = usize::try_from(range.length).ok()?;
            let end_utf16 = start_utf16.checked_add(len_utf16)?;
            let start = convert::byte_offset(body, start_utf16)?;
            let end = convert::byte_offset(body, end_utf16)?;
            let len = end.checked_sub(start)?;
            (len > 0).then_some(StyleSpan { start, len, style })
        })
        .collect()
}

fn backup_mentions(body: &str, ranges: &[proto::BodyRange]) -> Vec<Mention> {
    ranges
        .iter()
        .filter_map(|range| {
            let aci_bytes = match &range.associated_value {
                Some(proto::body_range::AssociatedValue::MentionAci(bytes)) => bytes.as_slice(),
                Some(proto::body_range::AssociatedValue::Style(_)) | None => return None,
            };
            let who = ServiceId::Aci(Uuid::from_slice(aci_bytes).ok()?);
            let start_utf16 = usize::try_from(range.start).ok()?;
            let len_utf16 = usize::try_from(range.length).ok()?;
            let end_utf16 = start_utf16.checked_add(len_utf16)?;
            let start = convert::byte_offset(body, start_utf16)?;
            let end = convert::byte_offset(body, end_utf16)?;
            let len = end.checked_sub(start)?;
            (len > 0).then_some(Mention { who, start, len })
        })
        .collect()
}

fn backup_attachment(att: &proto::MessageAttachment) -> Option<faro_core::domain::AttachmentRef> {
    let pointer_proto = att.pointer.as_ref()?;
    // `MessageAttachment.Flag.VOICE_MESSAGE = 1` maps to `AttachmentPointer.Flags.VOICE_MESSAGE = 1`.
    let flags = u32::from(att.flag == 1);
    let pointer = file_pointer_to_attachment_pointer(pointer_proto, flags)?;
    Some(convert::attachment(&pointer))
}

fn file_pointer_to_attachment_pointer(
    fp: &proto::FilePointer,
    flags: u32,
) -> Option<AttachmentPointer> {
    let locator = fp.locator_info.as_ref()?;
    let cdn_key = locator.transit_cdn_key.clone().filter(|k| !k.is_empty())?;
    if locator.key.is_empty() {
        return None;
    }
    let digest = match &locator.integrity_check {
        Some(proto::file_pointer::locator_info::IntegrityCheck::EncryptedDigest(d))
            if !d.is_empty() =>
        {
            Some(d.clone())
        }
        Some(
            proto::file_pointer::locator_info::IntegrityCheck::PlaintextHash(_)
            | proto::file_pointer::locator_info::IntegrityCheck::EncryptedDigest(_),
        )
        | None => None,
    };
    Some(AttachmentPointer {
        cdn_number: locator.transit_cdn_number,
        attachment_identifier: Some(AttachmentIdentifier::CdnKey(cdn_key)),
        content_type: fp.content_type.clone(),
        key: Some(locator.key.clone()),
        size: (locator.size > 0).then_some(locator.size),
        digest,
        incremental_mac: fp.incremental_mac.clone().filter(|m| !m.is_empty()),
        chunk_size: fp.incremental_mac_chunk_size.filter(|&c| c > 0),
        file_name: fp.file_name.clone().filter(|n| !n.is_empty()),
        flags: (flags > 0).then_some(flags),
        width: fp.width.filter(|&w| w > 0),
        height: fp.height.filter(|&h| h > 0),
        caption: fp.caption.clone().filter(|c| !c.is_empty()),
        blur_hash: fp.blur_hash.clone().filter(|b| !b.is_empty()),
        upload_timestamp: locator.transit_tier_upload_timestamp,
        ..AttachmentPointer::default()
    })
}

// ---- Subset of Signal's `backup.proto` ----
//
// Defined with `#[derive(prost::Message)]` so unknown fields and new oneof
// variants added by future Signal releases are skipped without failing the
// import.

mod proto {
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct BackupInfo {
        #[prost(uint64, tag = "1")]
        pub version: u64,
        #[prost(uint64, tag = "2")]
        pub backup_time_ms: u64,
        #[prost(bytes = "vec", tag = "3")]
        pub media_root_backup_key: Vec<u8>,
        #[prost(string, tag = "4")]
        pub current_app_version: String,
        #[prost(string, tag = "5")]
        pub first_app_version: String,
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct Frame {
        #[prost(oneof = "frame::Item", tags = "1, 2, 3, 4")]
        pub item: Option<frame::Item>,
    }

    pub mod frame {
        #[derive(Clone, PartialEq, prost::Oneof)]
        pub enum Item {
            #[prost(message, tag = "1")]
            Account(super::AccountData),
            #[prost(message, tag = "2")]
            Recipient(super::Recipient),
            #[prost(message, tag = "3")]
            Chat(super::Chat),
            #[prost(message, tag = "4")]
            ChatItem(super::ChatItem),
        }
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct AccountData {
        #[prost(bytes = "vec", tag = "1")]
        pub profile_key: Vec<u8>,
        #[prost(string, optional, tag = "2")]
        pub username: Option<String>,
        #[prost(string, tag = "4")]
        pub given_name: String,
        #[prost(string, tag = "5")]
        pub family_name: String,
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct Recipient {
        #[prost(uint64, tag = "1")]
        pub id: u64,
        #[prost(oneof = "recipient::Destination", tags = "2, 3, 5")]
        pub destination: Option<recipient::Destination>,
    }

    pub mod recipient {
        #[derive(Clone, PartialEq, prost::Oneof)]
        pub enum Destination {
            #[prost(message, tag = "2")]
            Contact(super::Contact),
            #[prost(message, tag = "3")]
            Group(super::Group),
            #[prost(message, tag = "5")]
            Self_(super::SelfRecipient),
        }
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct SelfRecipient {}

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct Contact {
        #[prost(bytes = "vec", optional, tag = "1")]
        pub aci: Option<Vec<u8>>,
        #[prost(bytes = "vec", optional, tag = "2")]
        pub pni: Option<Vec<u8>>,
        #[prost(string, optional, tag = "3")]
        pub username: Option<String>,
        #[prost(uint64, optional, tag = "4")]
        pub e164: Option<u64>,
        #[prost(int32, tag = "6")]
        pub visibility: i32,
        #[prost(bytes = "vec", optional, tag = "9")]
        pub profile_key: Option<Vec<u8>>,
        #[prost(string, optional, tag = "11")]
        pub profile_given_name: Option<String>,
        #[prost(string, optional, tag = "12")]
        pub profile_family_name: Option<String>,
        #[prost(bytes = "vec", optional, tag = "14")]
        pub identity_key: Option<Vec<u8>>,
        #[prost(int32, tag = "15")]
        pub identity_state: i32,
        #[prost(message, optional, tag = "16")]
        pub nickname: Option<contact::Name>,
        #[prost(string, tag = "18")]
        pub system_given_name: String,
        #[prost(string, tag = "19")]
        pub system_family_name: String,
        #[prost(string, tag = "20")]
        pub system_nickname: String,
    }

    pub mod contact {
        #[derive(Clone, PartialEq, prost::Message)]
        pub struct Name {
            #[prost(string, tag = "1")]
            pub given: String,
            #[prost(string, tag = "2")]
            pub family: String,
        }
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct Group {
        #[prost(bytes = "vec", tag = "1")]
        pub master_key: Vec<u8>,
        #[prost(message, optional, tag = "5")]
        pub snapshot: Option<group::GroupSnapshot>,
    }

    pub mod group {
        #[derive(Clone, PartialEq, prost::Message)]
        pub struct GroupSnapshot {
            #[prost(message, optional, tag = "2")]
            pub title: Option<GroupAttributeBlob>,
            #[prost(string, tag = "3")]
            pub avatar_url: String,
            #[prost(message, optional, tag = "4")]
            pub disappearing_messages_timer: Option<GroupAttributeBlob>,
            #[prost(uint32, tag = "6")]
            pub version: u32,
            #[prost(message, repeated, tag = "7")]
            pub members: Vec<Member>,
            #[prost(bytes = "vec", tag = "10")]
            pub invite_link_password: Vec<u8>,
            #[prost(message, optional, tag = "11")]
            pub description: Option<GroupAttributeBlob>,
        }

        #[derive(Clone, PartialEq, prost::Message)]
        pub struct GroupAttributeBlob {
            #[prost(oneof = "group_attribute_blob::Content", tags = "1, 2, 3, 4")]
            pub content: Option<group_attribute_blob::Content>,
        }

        pub mod group_attribute_blob {
            #[derive(Clone, PartialEq, prost::Oneof)]
            pub enum Content {
                #[prost(string, tag = "1")]
                Title(String),
                #[prost(bytes, tag = "2")]
                Avatar(Vec<u8>),
                #[prost(uint32, tag = "3")]
                DisappearingMessagesDuration(u32),
                #[prost(string, tag = "4")]
                DescriptionText(String),
            }
        }

        #[derive(Clone, PartialEq, prost::Message)]
        pub struct Member {
            #[prost(bytes = "vec", tag = "1")]
            pub user_id: Vec<u8>,
            #[prost(int32, tag = "2")]
            pub role: i32,
        }
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct Chat {
        #[prost(uint64, tag = "1")]
        pub id: u64,
        #[prost(uint64, tag = "2")]
        pub recipient_id: u64,
        #[prost(bool, tag = "3")]
        pub archived: bool,
        #[prost(uint32, optional, tag = "4")]
        pub pinned_order: Option<u32>,
        #[prost(uint64, optional, tag = "5")]
        pub expiration_timer_ms: Option<u64>,
        #[prost(uint64, optional, tag = "6")]
        pub mute_until_ms: Option<u64>,
        #[prost(bool, tag = "7")]
        pub marked_unread: bool,
        #[prost(bool, tag = "8")]
        pub dont_notify_for_mentions_if_muted: bool,
        #[prost(uint32, tag = "10")]
        pub expire_timer_version: u32,
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct ChatItem {
        #[prost(uint64, tag = "1")]
        pub chat_id: u64,
        #[prost(uint64, tag = "2")]
        pub author_id: u64,
        #[prost(uint64, tag = "3")]
        pub date_sent: u64,
        #[prost(uint64, optional, tag = "4")]
        pub expire_start_date: Option<u64>,
        #[prost(uint64, optional, tag = "5")]
        pub expires_in_ms: Option<u64>,
        #[prost(message, repeated, tag = "6")]
        pub revisions: Vec<ChatItem>,
        #[prost(oneof = "chat_item::DirectionalDetails", tags = "8, 9, 10")]
        pub directional_details: Option<chat_item::DirectionalDetails>,
        #[prost(oneof = "chat_item::Item", tags = "11, 13, 14, 15")]
        pub item: Option<chat_item::Item>,
    }

    pub mod chat_item {
        #[derive(Clone, PartialEq, prost::Message)]
        pub struct IncomingMessageDetails {
            #[prost(uint64, tag = "1")]
            pub date_received: u64,
            #[prost(uint64, optional, tag = "2")]
            pub date_server_sent: Option<u64>,
            #[prost(bool, tag = "3")]
            pub read: bool,
            #[prost(bool, tag = "4")]
            pub sealed_sender: bool,
        }

        #[derive(Clone, PartialEq, prost::Message)]
        pub struct OutgoingMessageDetails {
            #[prost(message, repeated, tag = "1")]
            pub send_status: Vec<super::SendStatus>,
            #[prost(uint64, tag = "2")]
            pub date_received: u64,
        }

        #[derive(Clone, PartialEq, prost::Message)]
        pub struct DirectionlessMessageDetails {}

        #[derive(Clone, PartialEq, prost::Oneof)]
        pub enum DirectionalDetails {
            #[prost(message, tag = "8")]
            Incoming(IncomingMessageDetails),
            #[prost(message, tag = "9")]
            Outgoing(OutgoingMessageDetails),
            #[prost(message, tag = "10")]
            Directionless(DirectionlessMessageDetails),
        }

        #[derive(Clone, PartialEq, prost::Oneof)]
        pub enum Item {
            #[prost(message, tag = "11")]
            Standard(super::StandardMessage),
            #[prost(message, tag = "13")]
            Sticker(super::StickerMessage),
            #[prost(message, tag = "14")]
            RemoteDeleted(super::RemoteDeletedMessage),
            #[prost(message, tag = "15")]
            UpdateMessage(super::ChatUpdateMessage),
        }
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct ChatUpdateMessage {
        #[prost(oneof = "chat_update_message::Update", tags = "2, 3, 10, 11")]
        pub update: Option<chat_update_message::Update>,
    }

    pub mod chat_update_message {
        #[derive(Clone, PartialEq, prost::Oneof)]
        pub enum Update {
            #[prost(message, tag = "2")]
            ExpirationTimerChange(super::ExpirationTimerChatUpdate),
            #[prost(message, tag = "3")]
            GroupChange(super::GroupChangeChatUpdate),
            #[prost(message, tag = "10")]
            IndividualCall(super::IndividualCall),
            #[prost(message, tag = "11")]
            GroupCall(super::GroupCall),
        }
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct ExpirationTimerChatUpdate {
        #[prost(uint64, tag = "1")]
        pub expires_in_ms: u64,
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct IndividualCall {
        #[prost(int32, tag = "2")]
        pub r#type: i32,
        #[prost(int32, tag = "3")]
        pub direction: i32,
        #[prost(int32, tag = "4")]
        pub state: i32,
        #[prost(uint64, tag = "5")]
        pub started_call_timestamp: u64,
        #[prost(bool, tag = "6")]
        pub read: bool,
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct GroupCall {
        #[prost(int32, tag = "2")]
        pub state: i32,
        #[prost(uint64, optional, tag = "3")]
        pub ringer_recipient_id: Option<u64>,
        #[prost(uint64, tag = "5")]
        pub started_call_timestamp: u64,
        #[prost(uint64, optional, tag = "6")]
        pub ended_call_timestamp: Option<u64>,
        #[prost(bool, tag = "7")]
        pub read: bool,
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct GroupChangeChatUpdate {
        #[prost(message, repeated, tag = "1")]
        pub updates: Vec<group_change_chat_update::Update>,
    }

    pub mod group_change_chat_update {
        #[derive(Clone, PartialEq, prost::Message)]
        pub struct Update {
            #[prost(oneof = "update::Body", tags = "2, 6, 8, 9, 10, 11, 22")]
            pub body: Option<update::Body>,
        }

        pub mod update {
            #[expect(
                clippy::enum_variant_names,
                reason = "prost oneof variant names match Signal backup.proto"
            )]
            #[derive(Clone, PartialEq, prost::Oneof)]
            pub enum Body {
                #[prost(message, tag = "2")]
                GroupNameUpdate(super::GroupNameUpdate),
                #[prost(message, tag = "6")]
                GroupExpirationTimerUpdate(super::GroupExpirationTimerUpdate),
                #[prost(message, tag = "8")]
                GroupMemberAddedUpdate(super::GroupMemberAddedUpdate),
                #[prost(message, tag = "9")]
                GroupMemberJoinedUpdate(super::GroupMemberJoinedUpdate),
                #[prost(message, tag = "10")]
                GroupMemberJoinedByLinkUpdate(super::GroupMemberJoinedUpdate),
                #[prost(message, tag = "11")]
                GroupMemberRemovedUpdate(super::GroupMemberRemovedUpdate),
                #[prost(message, tag = "22")]
                GroupMemberLeftUpdate(super::GroupMemberLeftUpdate),
            }
        }

        #[derive(Clone, PartialEq, prost::Message)]
        pub struct GroupNameUpdate {
            #[prost(bytes = "vec", optional, tag = "1")]
            pub updater_aci: Option<Vec<u8>>,
            #[prost(string, optional, tag = "2")]
            pub new_group_name: Option<String>,
        }

        #[derive(Clone, PartialEq, prost::Message)]
        pub struct GroupExpirationTimerUpdate {
            #[prost(uint64, tag = "1")]
            pub expires_in_ms: u64,
            #[prost(bytes = "vec", optional, tag = "2")]
            pub updater_aci: Option<Vec<u8>>,
        }

        #[derive(Clone, PartialEq, prost::Message)]
        pub struct GroupMemberAddedUpdate {
            #[prost(bytes = "vec", optional, tag = "1")]
            pub updater_aci: Option<Vec<u8>>,
            #[prost(bytes = "vec", tag = "2")]
            pub new_member_aci: Vec<u8>,
        }

        #[derive(Clone, PartialEq, prost::Message)]
        pub struct GroupMemberJoinedUpdate {
            #[prost(bytes = "vec", tag = "1")]
            pub new_member_aci: Vec<u8>,
        }

        #[derive(Clone, PartialEq, prost::Message)]
        pub struct GroupMemberRemovedUpdate {
            #[prost(bytes = "vec", optional, tag = "1")]
            pub remover_aci: Option<Vec<u8>>,
            #[prost(bytes = "vec", tag = "2")]
            pub removed_aci: Vec<u8>,
        }

        #[derive(Clone, PartialEq, prost::Message)]
        pub struct GroupMemberLeftUpdate {
            #[prost(bytes = "vec", tag = "1")]
            pub aci: Vec<u8>,
        }
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct SendStatus {
        #[prost(uint64, tag = "1")]
        pub recipient_id: u64,
        #[prost(uint64, tag = "2")]
        pub timestamp: u64,
        #[prost(oneof = "send_status::DeliveryStatus", tags = "3, 4, 5, 6, 7, 8, 9")]
        pub delivery_status: Option<send_status::DeliveryStatus>,
    }

    pub mod send_status {
        #[derive(Clone, PartialEq, prost::Message)]
        pub struct Pending {}
        #[derive(Clone, PartialEq, prost::Message)]
        pub struct Sent {}
        #[derive(Clone, PartialEq, prost::Message)]
        pub struct Delivered {}
        #[derive(Clone, PartialEq, prost::Message)]
        pub struct Read {}
        #[derive(Clone, PartialEq, prost::Message)]
        pub struct Viewed {}
        #[derive(Clone, PartialEq, prost::Message)]
        pub struct Skipped {}
        #[derive(Clone, PartialEq, prost::Message)]
        pub struct Failed {}

        #[derive(Clone, PartialEq, prost::Oneof)]
        pub enum DeliveryStatus {
            #[prost(message, tag = "3")]
            Pending(Pending),
            #[prost(message, tag = "4")]
            Sent(Sent),
            #[prost(message, tag = "5")]
            Delivered(Delivered),
            #[prost(message, tag = "6")]
            Read(Read),
            #[prost(message, tag = "7")]
            Viewed(Viewed),
            #[prost(message, tag = "8")]
            Skipped(Skipped),
            #[prost(message, tag = "9")]
            Failed(Failed),
        }
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct Text {
        #[prost(string, tag = "1")]
        pub body: String,
        #[prost(message, repeated, tag = "2")]
        pub body_ranges: Vec<BodyRange>,
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct StandardMessage {
        #[prost(message, optional, tag = "1")]
        pub quote: Option<Quote>,
        #[prost(message, optional, tag = "2")]
        pub text: Option<Text>,
        #[prost(message, repeated, tag = "3")]
        pub attachments: Vec<MessageAttachment>,
        #[prost(message, repeated, tag = "6")]
        pub reactions: Vec<Reaction>,
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct StickerMessage {
        #[prost(message, optional, tag = "1")]
        pub sticker: Option<Sticker>,
        #[prost(message, repeated, tag = "2")]
        pub reactions: Vec<Reaction>,
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct Sticker {
        #[prost(string, optional, tag = "4")]
        pub emoji: Option<String>,
        #[prost(message, optional, tag = "5")]
        pub data: Option<FilePointer>,
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct RemoteDeletedMessage {}

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct MessageAttachment {
        #[prost(message, optional, tag = "1")]
        pub pointer: Option<FilePointer>,
        #[prost(int32, tag = "2")]
        pub flag: i32,
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct FilePointer {
        #[prost(string, optional, tag = "4")]
        pub content_type: Option<String>,
        #[prost(bytes = "vec", optional, tag = "5")]
        pub incremental_mac: Option<Vec<u8>>,
        #[prost(uint32, optional, tag = "6")]
        pub incremental_mac_chunk_size: Option<u32>,
        #[prost(string, optional, tag = "7")]
        pub file_name: Option<String>,
        #[prost(uint32, optional, tag = "8")]
        pub width: Option<u32>,
        #[prost(uint32, optional, tag = "9")]
        pub height: Option<u32>,
        #[prost(string, optional, tag = "10")]
        pub caption: Option<String>,
        #[prost(string, optional, tag = "11")]
        pub blur_hash: Option<String>,
        #[prost(message, optional, tag = "13")]
        pub locator_info: Option<file_pointer::LocatorInfo>,
    }

    pub mod file_pointer {
        #[derive(Clone, PartialEq, prost::Message)]
        pub struct LocatorInfo {
            #[prost(bytes = "vec", tag = "1")]
            pub key: Vec<u8>,
            #[prost(uint32, tag = "3")]
            pub size: u32,
            #[prost(string, optional, tag = "4")]
            pub transit_cdn_key: Option<String>,
            #[prost(uint32, optional, tag = "5")]
            pub transit_cdn_number: Option<u32>,
            #[prost(uint64, optional, tag = "6")]
            pub transit_tier_upload_timestamp: Option<u64>,
            #[prost(oneof = "locator_info::IntegrityCheck", tags = "10, 11")]
            pub integrity_check: Option<locator_info::IntegrityCheck>,
        }

        pub mod locator_info {
            #[derive(Clone, PartialEq, prost::Oneof)]
            pub enum IntegrityCheck {
                #[prost(bytes, tag = "10")]
                PlaintextHash(Vec<u8>),
                #[prost(bytes, tag = "11")]
                EncryptedDigest(Vec<u8>),
            }
        }
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct Quote {
        #[prost(uint64, optional, tag = "1")]
        pub target_sent_timestamp: Option<u64>,
        #[prost(uint64, tag = "2")]
        pub author_id: u64,
        #[prost(message, optional, tag = "3")]
        pub text: Option<Text>,
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct BodyRange {
        #[prost(uint32, tag = "1")]
        pub start: u32,
        #[prost(uint32, tag = "2")]
        pub length: u32,
        #[prost(oneof = "body_range::AssociatedValue", tags = "3, 4")]
        pub associated_value: Option<body_range::AssociatedValue>,
    }

    pub mod body_range {
        #[derive(Clone, PartialEq, prost::Oneof)]
        pub enum AssociatedValue {
            #[prost(bytes, tag = "3")]
            MentionAci(Vec<u8>),
            #[prost(int32, tag = "4")]
            Style(i32),
        }
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct Reaction {
        #[prost(string, tag = "1")]
        pub emoji: String,
        #[prost(uint64, tag = "2")]
        pub author_id: u64,
        #[prost(uint64, tag = "3")]
        pub sent_timestamp: u64,
        #[prost(uint64, tag = "4")]
        pub sort_order: u64,
    }
}

#[cfg(test)]
#[expect(
    clippy::disallowed_methods,
    reason = "#[tokio::test] expands to block_on; tests may block"
)]
mod tests {
    use std::io::Write as _;

    use flate2::Compression;
    use flate2::write::GzEncoder;
    use presage::libsignal_service::attachment_cipher::encrypt_in_place;

    use super::*;

    const ME: u128 = 1;
    const ALICE: u128 = 2;
    const BOB: u128 = 3;

    fn write_varint_frame(out: &mut Vec<u8>, message: &impl prost::Message) {
        let bytes = message.encode_to_vec();
        prost::encoding::encode_varint(bytes.len() as u64, out);
        out.extend_from_slice(&bytes);
    }

    fn encrypt_archive(
        raw_frames: &[u8],
        ephemeral_key: &[u8; 32],
        aci: Uuid,
        with_sbackup_header: bool,
    ) -> Vec<u8> {
        let mut gz = GzEncoder::new(Vec::new(), Compression::fast());
        gz.write_all(raw_frames).unwrap();
        let mut buf = gz.finish().unwrap();

        let (hmac_key, aes_key) = derive_backup_keys(ephemeral_key, aci);
        let mut cipher_key = [0u8; 64];
        cipher_key[..32].copy_from_slice(&aes_key);
        cipher_key[32..64].copy_from_slice(&hmac_key);

        let iv = [0x42u8; 16];
        encrypt_in_place(iv, cipher_key, &mut buf);
        let ciphertext = buf;

        if !with_sbackup_header {
            return ciphertext;
        }
        let mut framed = SBACKUP_HEADER_MAGIC.to_vec();
        let fake_meta = [0x0a, 0x02, 0x08, 0x01];
        prost::encoding::encode_varint(fake_meta.len() as u64, &mut framed);
        framed.extend_from_slice(&fake_meta);
        framed.extend_from_slice(&ciphertext);
        framed
    }

    /// Key derivation matches `libsignal-message-backup`'s `MessageBackupKey`
    /// test vector (`key.rs`: `FAKE_ACCOUNT_ENTROPY_POOL` + `FAKE_ACI`).
    #[test]
    fn key_derivation_matches_libsignal_test_vector() {
        use presage::libsignal_service::libsignal_account_keys::AccountEntropyPool;

        let aep: AccountEntropyPool =
            "dtjs858asj6tv0jzsqrsmj0ubp335pisj98e9ssnss8myoc08drhtcktyawvx45l"
                .parse()
                .unwrap();
        let backup_key = BackupKey::derive_from_account_entropy_pool(&aep);
        let aci: Uuid = "659aa5f4-a28d-fcc1-1ea1-b997537a3d95".parse().unwrap();
        let (hmac_key, aes_key) = derive_backup_keys(&backup_key.0, aci);
        assert_eq!(
            hmac_key,
            [
                0xf4, 0x25, 0xe2, 0x2a, 0x60, 0x7c, 0x52, 0x97, 0x17, 0xe1, 0xe1, 0xb2, 0x9f, 0x9f,
                0xe1, 0x39, 0xf9, 0xd1, 0xc7, 0xe7, 0xd0, 0x1e, 0x37, 0x1c, 0x77, 0x53, 0xc5, 0x44,
                0xa3, 0x02, 0x66, 0x49,
            ]
        );
        assert_eq!(
            aes_key,
            [
                0xe1, 0x43, 0xf4, 0xad, 0x56, 0x68, 0xd8, 0xbf, 0xed, 0x2f, 0x88, 0x56, 0x2f, 0x06,
                0x93, 0xf5, 0x3b, 0xda, 0x2c, 0x0e, 0x55, 0xc9, 0xd7, 0x17, 0x30, 0xf3, 0x0e, 0x24,
                0x69, 0x5f, 0xd6, 0xdf,
            ]
        );
    }

    #[expect(
        clippy::too_many_lines,
        reason = "test fixture builder assembling a multi-frame protobuf stream"
    )]
    fn build_sample_recipients_and_chats(
        raw: &mut Vec<u8>,
        me_uuid: Uuid,
        alice_uuid: Uuid,
        bob_uuid: Uuid,
        group_master_key: [u8; 32],
    ) {
        write_varint_frame(
            raw,
            &proto::BackupInfo {
                version: 1,
                backup_time_ms: 1_700_000_000_000,
                media_root_backup_key: vec![0xaa; 32],
                current_app_version: "Signal 7.0".into(),
                first_app_version: "Signal 7.0".into(),
            },
        );
        write_varint_frame(
            raw,
            &proto::Frame {
                item: Some(proto::frame::Item::Recipient(proto::Recipient {
                    id: 1,
                    destination: Some(proto::recipient::Destination::Self_(
                        proto::SelfRecipient {},
                    )),
                })),
            },
        );
        write_varint_frame(
            raw,
            &proto::Frame {
                item: Some(proto::frame::Item::Recipient(proto::Recipient {
                    id: 2,
                    destination: Some(proto::recipient::Destination::Contact(proto::Contact {
                        aci: Some(alice_uuid.as_bytes().to_vec()),
                        pni: None,
                        username: Some("alice.01".into()),
                        e164: Some(447_700_900_111),
                        visibility: 0,
                        profile_key: Some(vec![0x33; 32]),
                        profile_given_name: Some("Alice".into()),
                        profile_family_name: Some("Wonder".into()),
                        identity_key: None,
                        identity_state: 1,
                        nickname: Some(proto::contact::Name {
                            given: "Ali".into(),
                            family: String::new(),
                        }),
                        system_given_name: "Alice".into(),
                        system_family_name: "W.".into(),
                        system_nickname: String::new(),
                    })),
                })),
            },
        );
        write_varint_frame(
            raw,
            &proto::Frame {
                item: Some(proto::frame::Item::Recipient(proto::Recipient {
                    id: 3,
                    destination: Some(proto::recipient::Destination::Contact(proto::Contact {
                        aci: Some(bob_uuid.as_bytes().to_vec()),
                        pni: None,
                        username: None,
                        e164: None,
                        visibility: 0,
                        profile_key: None,
                        profile_given_name: Some("Bob".into()),
                        profile_family_name: None,
                        identity_key: None,
                        identity_state: 0,
                        nickname: None,
                        system_given_name: String::new(),
                        system_family_name: String::new(),
                        system_nickname: String::new(),
                    })),
                })),
            },
        );
        write_varint_frame(
            raw,
            &proto::Frame {
                item: Some(proto::frame::Item::Recipient(proto::Recipient {
                    id: 4,
                    destination: Some(proto::recipient::Destination::Group(proto::Group {
                        master_key: group_master_key.to_vec(),
                        snapshot: Some(proto::group::GroupSnapshot {
                            title: Some(proto::group::GroupAttributeBlob {
                                content: Some(proto::group::group_attribute_blob::Content::Title(
                                    "Climbing".into(),
                                )),
                            }),
                            avatar_url: String::new(),
                            disappearing_messages_timer: None,
                            version: 4,
                            members: vec![
                                proto::group::Member {
                                    user_id: me_uuid.as_bytes().to_vec(),
                                    role: 2,
                                },
                                proto::group::Member {
                                    user_id: alice_uuid.as_bytes().to_vec(),
                                    role: 1,
                                },
                                proto::group::Member {
                                    user_id: bob_uuid.as_bytes().to_vec(),
                                    role: 1,
                                },
                            ],
                            invite_link_password: vec![],
                            description: None,
                        }),
                    })),
                })),
            },
        );
        write_varint_frame(
            raw,
            &proto::Frame {
                item: Some(proto::frame::Item::Chat(proto::Chat {
                    id: 10,
                    recipient_id: 2,
                    archived: false,
                    pinned_order: Some(0),
                    expiration_timer_ms: Some(3_600_000),
                    mute_until_ms: None,
                    marked_unread: false,
                    dont_notify_for_mentions_if_muted: false,
                    expire_timer_version: 2,
                })),
            },
        );
        write_varint_frame(
            raw,
            &proto::Frame {
                item: Some(proto::frame::Item::Chat(proto::Chat {
                    id: 20,
                    recipient_id: 4,
                    archived: true,
                    pinned_order: None,
                    expiration_timer_ms: None,
                    mute_until_ms: Some(u64::MAX),
                    marked_unread: false,
                    dont_notify_for_mentions_if_muted: false,
                    expire_timer_version: 1,
                })),
            },
        );
    }

    #[expect(
        clippy::too_many_lines,
        reason = "test fixture builder assembling incoming, group and outgoing message frames"
    )]
    fn build_sample_messages(raw: &mut Vec<u8>, me_uuid: Uuid) {
        write_varint_frame(
            raw,
            &proto::Frame {
                item: Some(proto::frame::Item::ChatItem(proto::ChatItem {
                    chat_id: 10,
                    author_id: 2,
                    date_sent: 1_500,
                    expire_start_date: Some(1_100),
                    expires_in_ms: Some(3_600_000),
                    revisions: vec![proto::ChatItem {
                        chat_id: 10,
                        author_id: 2,
                        date_sent: 1_000,
                        expire_start_date: None,
                        expires_in_ms: None,
                        revisions: vec![],
                        directional_details: None,
                        item: None,
                    }],
                    directional_details: Some(proto::chat_item::DirectionalDetails::Incoming(
                        proto::chat_item::IncomingMessageDetails {
                            date_received: 1_001,
                            date_server_sent: Some(1_000),
                            read: true,
                            sealed_sender: true,
                        },
                    )),
                    item: Some(proto::chat_item::Item::Standard(proto::StandardMessage {
                        quote: None,
                        text: Some(proto::Text {
                            body: "edited hello".into(),
                            body_ranges: vec![],
                        }),
                        attachments: vec![proto::MessageAttachment {
                            pointer: Some(proto::FilePointer {
                                content_type: Some("audio/aac".into()),
                                incremental_mac: None,
                                incremental_mac_chunk_size: None,
                                file_name: None,
                                width: None,
                                height: None,
                                caption: None,
                                blur_hash: None,
                                locator_info: Some(proto::file_pointer::LocatorInfo {
                                    key: vec![0x77; 64],
                                    size: 4096,
                                    transit_cdn_key: Some("cdn-voice-1".into()),
                                    transit_cdn_number: Some(2),
                                    transit_tier_upload_timestamp: Some(1_000),
                                    integrity_check: Some(
                                        proto::file_pointer::locator_info::IntegrityCheck::EncryptedDigest(
                                            vec![0x88; 32],
                                        ),
                                    ),
                                }),
                            }),
                            flag: 1,
                        }],
                        reactions: vec![proto::Reaction {
                            emoji: "👍".into(),
                            author_id: 1,
                            sent_timestamp: 1_600,
                            sort_order: 1,
                        }],
                    })),
                })),
            },
        );
        write_varint_frame(
            raw,
            &proto::Frame {
                item: Some(proto::frame::Item::ChatItem(proto::ChatItem {
                    chat_id: 20,
                    author_id: 3,
                    date_sent: 2_000,
                    expire_start_date: None,
                    expires_in_ms: None,
                    revisions: vec![],
                    directional_details: Some(proto::chat_item::DirectionalDetails::Incoming(
                        proto::chat_item::IncomingMessageDetails {
                            date_received: 2_001,
                            date_server_sent: Some(2_000),
                            read: false,
                            sealed_sender: true,
                        },
                    )),
                    item: Some(proto::chat_item::Item::Standard(proto::StandardMessage {
                        quote: Some(proto::Quote {
                            target_sent_timestamp: Some(1_000),
                            author_id: 2,
                            text: Some(proto::Text {
                                body: "edited hello".into(),
                                body_ranges: vec![],
                            }),
                        }),
                        text: Some(proto::Text {
                            body: "hey \u{FFFC}!".into(),
                            body_ranges: vec![proto::BodyRange {
                                start: 4,
                                length: 1,
                                associated_value: Some(
                                    proto::body_range::AssociatedValue::MentionAci(
                                        me_uuid.as_bytes().to_vec(),
                                    ),
                                ),
                            }],
                        }),
                        attachments: vec![],
                        reactions: vec![],
                    })),
                })),
            },
        );
        write_varint_frame(
            raw,
            &proto::Frame {
                item: Some(proto::frame::Item::ChatItem(proto::ChatItem {
                    chat_id: 10,
                    author_id: 1,
                    date_sent: 3_000,
                    expire_start_date: None,
                    expires_in_ms: Some(3_600_000),
                    revisions: vec![],
                    directional_details: Some(proto::chat_item::DirectionalDetails::Outgoing(
                        proto::chat_item::OutgoingMessageDetails {
                            send_status: vec![proto::SendStatus {
                                recipient_id: 2,
                                timestamp: 3_100,
                                delivery_status: Some(proto::send_status::DeliveryStatus::Read(
                                    proto::send_status::Read {},
                                )),
                            }],
                            date_received: 3_000,
                        },
                    )),
                    item: Some(proto::chat_item::Item::Standard(proto::StandardMessage {
                        quote: None,
                        text: Some(proto::Text {
                            body: "on my way".into(),
                            body_ranges: vec![],
                        }),
                        attachments: vec![],
                        reactions: vec![],
                    })),
                })),
            },
        );
    }

    #[tokio::test]
    #[expect(
        clippy::too_many_lines,
        reason = "end-to-end test verifying both stores and allowlist filtering"
    )]
    async fn encrypted_archive_round_trips_into_presage_and_faro_stores() {
        let me_uuid = Uuid::from_u128(ME);
        let me = ServiceId::Aci(me_uuid);
        let alice_uuid = Uuid::from_u128(ALICE);
        let alice = ServiceId::Aci(alice_uuid);
        let bob_uuid = Uuid::from_u128(BOB);
        let bob = ServiceId::Aci(bob_uuid);
        let group_master_key = [0x55u8; 32];
        let group_id = convert::group_id(&group_master_key);

        let mut raw = Vec::new();
        build_sample_recipients_and_chats(
            &mut raw,
            me_uuid,
            alice_uuid,
            bob_uuid,
            group_master_key,
        );
        build_sample_messages(&mut raw, me_uuid);

        let ephemeral_key = [0x99u8; 32];
        for with_header in [false, true] {
            let encrypted = encrypt_archive(&raw, &ephemeral_key, me_uuid, with_header);
            let (hmac_key, aes_key) = derive_backup_keys(&ephemeral_key, me_uuid);
            let history = decrypt_and_decode(encrypted, &hmac_key, &aes_key, me).unwrap();

            let dir = tempfile::tempdir().unwrap();
            let mut presage_store = crate::open_store(&crate::store_file(dir.path()))
                .await
                .unwrap();
            history
                .save_to_presage_store(&mut presage_store, me)
                .await
                .unwrap();
            assert!(
                presage_store
                    .group(group_master_key)
                    .await
                    .unwrap()
                    .is_some()
            );
            assert!(
                presage_store
                    .contact_by_id(&convert::signal_id(alice))
                    .await
                    .unwrap()
                    .is_some()
            );

            let faro_store = Store::memory();
            let summary = history
                .save_to_faro_store(&faro_store, &presage_store, me, None)
                .await
                .unwrap();
            assert_eq!(summary.messages, 3);
            assert_eq!(summary.groups, 1);
            assert_eq!(summary.contacts, 2);
            assert_eq!(summary.conversations, 2);

            let direct_msgs = faro_store
                .messages(ConversationId::Direct(alice), None, 10)
                .await
                .unwrap();
            assert_eq!(direct_msgs.len(), 2);
            let first = &direct_msgs[0];
            assert_eq!(first.sent_at, Timestamp::from_millis(1_000));
            assert_eq!(first.edited_at, Some(Timestamp::from_millis(1_500)));
            assert_eq!(first.body, Some(MessageBody::new("edited hello")));
            assert_eq!(first.attachments.len(), 1);
            assert!(first.attachments[0].voice_note);
            assert!(convert::decode_pointer(&first.attachments[0].id).is_some());
            assert_eq!(first.reactions.len(), 1);
            assert_eq!(first.reactions[0].emoji, "👍");

            let second = &direct_msgs[1];
            assert_eq!(second.direction, Direction::Outgoing(DeliveryStatus::Read));
            let receipts = faro_store
                .message_receipts(second.reference())
                .await
                .unwrap();
            assert_eq!(receipts.len(), 1);
            assert_eq!(receipts[0].recipient, alice);
            assert_eq!(receipts[0].status, DeliveryStatus::Read);

            let summaries = faro_store.conversations().await.unwrap();
            let alice_summary = summaries
                .iter()
                .find(|s| s.id == ConversationId::Direct(alice))
                .unwrap();
            // Marked read in the archive (`incoming.read = true`), so unread is 0.
            assert_eq!(alice_summary.unread, 0);
            assert_eq!(alice_summary.disappearing_timer, Some(3_600));
            assert_eq!(alice_summary.state.pinned, Some(0));

            let group_summary = summaries
                .iter()
                .find(|s| s.id == ConversationId::Group(group_id))
                .unwrap();
            // Unread in the archive (`incoming.read = false`), so unread is 1.
            assert_eq!(group_summary.unread, 1);
            assert!(group_summary.state.archived);
            assert_eq!(
                group_summary.state.muted_until,
                Some(ConversationState::MUTED_FOREVER)
            );

            assert_eq!(
                faro_store
                    .nicknames()
                    .await
                    .unwrap()
                    .get(&alice)
                    .map(String::as_str),
                Some("Ali")
            );
            faro_store.close().await.unwrap();

            // With an allowlist that allows only the group (not Alice's direct
            // chat), only the group message and group members' contacts are
            // imported into `faro.db`.
            let guarded_store = Store::memory();
            let mut allowlist = Allowlist::default();
            allowlist.groups.insert(group_id);
            let guarded_summary = history
                .save_to_faro_store(&guarded_store, &presage_store, me, Some(&allowlist))
                .await
                .unwrap();
            assert_eq!(guarded_summary.messages, 1);
            assert_eq!(guarded_summary.groups, 1);
            // Both Alice and Bob are members of the allowed group, so their
            // contact details are admitted, while Alice's direct chat is not.
            assert_eq!(guarded_summary.contacts, 2);
            assert_eq!(guarded_summary.conversations, 1);
            let group_msgs = guarded_store
                .messages(ConversationId::Group(group_id), None, 10)
                .await
                .unwrap();
            assert_eq!(group_msgs.len(), 1);
            assert_eq!(group_msgs[0].sender, bob);
            assert_eq!(
                guarded_store
                    .messages(ConversationId::Direct(alice), None, 10)
                    .await
                    .unwrap()
                    .len(),
                0
            );
            guarded_store.close().await.unwrap();
        }
    }
}
