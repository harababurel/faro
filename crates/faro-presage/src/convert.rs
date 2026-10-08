//! presage's contents ↔ faro's types. Pure functions, so the mapping is
//! tested with hand-built contents and no network.

use faro_core::backend::{
    BackendEvent, OutgoingAttachment, OutgoingEdit, OutgoingMessage, ReactionChange,
};
use faro_core::domain::{
    AttachmentId, AttachmentRef, AttachmentStatus, ConversationId, DeliveryStatus, Direction,
    GroupId, Mention, Message, MessageBody, MessageRef, Quote, ServiceId, Timestamp, TypingState,
};
use presage::libsignal_service::content::{Content, ContentBody};
use presage::libsignal_service::prelude::ProtobufMessage as _;
use presage::libsignal_service::protocol::{Aci, Pni, ServiceId as SignalId};
use presage::libsignal_service::sender::AttachmentSpec;
use presage::libsignal_service::zkgroup::groups::{GroupMasterKey, GroupSecretParams};
use presage::proto::body_range::AssociatedValue;
use presage::proto::data_message::{self, Delete};
use presage::proto::receipt_message;
use presage::proto::sync_message::{self, Sent};
use presage::proto::typing_message;
use presage::proto::{
    AttachmentPointer, BodyRange, DataMessage, EditMessage, GroupContextV2, ReceiptMessage,
    SyncMessage, TypingMessage,
};
use uuid::Uuid;

/// A group's master key: the secret that lets members read and send. faro's
/// [`GroupId`] is the public identifier derived from it (as Signal's own).
pub type MasterKey = [u8; 32];

pub fn service_id(id: &SignalId) -> ServiceId {
    match id {
        SignalId::Aci(aci) => ServiceId::Aci(Uuid::from(*aci)),
        SignalId::Pni(pni) => ServiceId::Pni(Uuid::from(*pni)),
    }
}

pub fn signal_id(id: ServiceId) -> SignalId {
    match id {
        ServiceId::Aci(uuid) => SignalId::Aci(Aci::from(uuid)),
        ServiceId::Pni(uuid) => SignalId::Pni(Pni::from(uuid)),
    }
}

pub fn group_id(master_key: &MasterKey) -> GroupId {
    let params = GroupSecretParams::derive_from_master_key(GroupMasterKey::new(*master_key));
    GroupId::from_bytes(params.get_group_identifier())
}

/// The group a data message belongs to, if any.
pub fn master_key(group: Option<&GroupContextV2>) -> Option<MasterKey> {
    group
        .and_then(|g| g.master_key.as_deref())
        .and_then(|key| key.try_into().ok())
}

/// The group (master key and revision) a content mentions, so the backend can
/// learn it and announce changes.
pub fn group_of(content: &Content) -> Option<(MasterKey, u32)> {
    let data = match &content.body {
        ContentBody::DataMessage(data) => Some(data),
        ContentBody::EditMessage(edit) => edit.data_message.as_ref(),
        ContentBody::SynchronizeMessage(SyncMessage {
            content: Some(sync_message::Content::Sent(sent)),
            ..
        }) => sent.message.as_ref().or(sent
            .edit_message
            .as_ref()
            .and_then(|e| e.data_message.as_ref())),
        ContentBody::SynchronizeMessage(_)
        | ContentBody::NullMessage(_)
        | ContentBody::CallMessage(_)
        | ContentBody::ReceiptMessage(_)
        | ContentBody::TypingMessage(_)
        | ContentBody::DecryptionErrorMessage(_)
        | ContentBody::StoryMessage(_) => None,
    }?;
    let group = data.group_v2.as_ref()?;
    Some((master_key(Some(group))?, group.revision.unwrap_or(0)))
}

/// What a received content means to faro. `me` is this account.
pub fn events(content: &Content, me: ServiceId) -> Vec<BackendEvent> {
    let sender = service_id(&content.metadata.sender);
    let direct = ConversationId::Direct(sender);
    match &content.body {
        ContentBody::DataMessage(data) => data_message(
            data,
            &Author {
                sender,
                direct,
                direction: Direction::Incoming,
            },
            u64::try_from(content.metadata.client_timestamp.timestamp_millis()).unwrap_or(0),
        ),
        ContentBody::EditMessage(edit) => edited(edit, sender, direct).into_iter().collect(),
        ContentBody::SynchronizeMessage(sync) => synced(sync, me),
        ContentBody::ReceiptMessage(receipt) => receipts(receipt, me),
        ContentBody::TypingMessage(typing) => vec![typed(typing, sender)],
        ContentBody::NullMessage(_)
        | ContentBody::CallMessage(_)
        | ContentBody::DecryptionErrorMessage(_)
        | ContentBody::StoryMessage(_) => vec![],
    }
}

/// What a content presage kept means to faro (D64). Like [`events`], except
/// that presage keeps what this device sent as plain contents from us (to
/// `metadata.destination`), and only what the store keeps is returned.
pub fn stored(content: &Content, me: ServiceId) -> Vec<BackendEvent> {
    let sender = service_id(&content.metadata.sender);
    let events = if sender == me {
        let direct = ConversationId::Direct(service_id(&content.metadata.destination));
        match &content.body {
            ContentBody::DataMessage(data) => data_message(
                data,
                &Author {
                    sender: me,
                    direct,
                    direction: Direction::Outgoing(DeliveryStatus::Sent),
                },
                u64::try_from(content.metadata.client_timestamp.timestamp_millis()).unwrap_or(0),
            ),
            ContentBody::EditMessage(edit) => edited(edit, me, direct).into_iter().collect(),
            ContentBody::SynchronizeMessage(sync) => synced(sync, me),
            // Receipts and typing indicators we sent mean nothing to us.
            ContentBody::ReceiptMessage(_)
            | ContentBody::TypingMessage(_)
            | ContentBody::NullMessage(_)
            | ContentBody::CallMessage(_)
            | ContentBody::DecryptionErrorMessage(_)
            | ContentBody::StoryMessage(_) => vec![],
        }
    } else {
        events(content, me)
    };
    events.into_iter().filter(kept).collect()
}

/// Whether the store keeps anything from this event.
fn kept(event: &BackendEvent) -> bool {
    match event {
        BackendEvent::Message(_)
        | BackendEvent::DeliveryUpdate { .. }
        | BackendEvent::Reaction { .. }
        | BackendEvent::Edited { .. }
        | BackendEvent::Deleted { .. }
        | BackendEvent::ReadElsewhere(_) => true,
        BackendEvent::Connection(_)
        | BackendEvent::Typing { .. }
        | BackendEvent::ContactUpdated(_)
        | BackendEvent::GroupUpdated(_)
        | BackendEvent::Error(_) => false,
    }
}

/// Who wrote a data message, and where it goes when it isn't a group's.
struct Author {
    sender: ServiceId,
    direct: ConversationId,
    direction: Direction,
}

fn conversation(data: &DataMessage, direct: ConversationId) -> ConversationId {
    master_key(data.group_v2.as_ref()).map_or(direct, |key| ConversationId::Group(group_id(&key)))
}

fn data_message(data: &DataMessage, author: &Author, fallback_sent_at: u64) -> Vec<BackendEvent> {
    let conversation = conversation(data, author.direct);
    if let Some(reaction) = &data.reaction {
        return reacted(reaction, conversation, author.sender)
            .into_iter()
            .collect();
    }
    if let Some(Delete {
        target_sent_timestamp: Some(target),
    }) = &data.delete
    {
        return vec![BackendEvent::Deleted {
            conversation,
            target: MessageRef {
                sender: author.sender,
                sent_at: Timestamp::from_millis(*target),
            },
        }];
    }
    let body = data
        .body
        .as_deref()
        .filter(|text| !text.is_empty())
        .map(MessageBody::new)
        .or_else(|| {
            data.sticker.as_ref().map(|sticker| {
                let emoji = sticker.emoji.as_deref().unwrap_or("");
                MessageBody::new(format!("[sticker] {emoji}").trim_end())
            })
        });
    let mut attachments: Vec<AttachmentRef> = data.attachments.iter().map(attachment).collect();
    if let Some(sticker) = data.sticker.as_ref().and_then(|s| s.data.as_ref()) {
        attachments.push(sticker_attachment(sticker));
    }
    if body.is_none() && attachments.is_empty() {
        // Profile key, timer and group changes, payments, calls…: nothing to show.
        return vec![];
    }
    vec![BackendEvent::Message(Message {
        conversation,
        sender: author.sender,
        sent_at: Timestamp::from_millis(data.timestamp.unwrap_or(fallback_sent_at)),
        direction: author.direction,
        body,
        mentions: data
            .body
            .as_deref()
            .map(|text| mentions(text, &data.body_ranges))
            .unwrap_or_default(),
        quote: data.quote.as_ref().and_then(quote),
        attachments,
        reactions: vec![],
        edited_at: None,
        deleted: false,
    })]
}

/// A message author given as a string or as 16 bytes (newer clients).
fn author(text: Option<&str>, binary: Option<&[u8]>) -> Option<ServiceId> {
    text.and_then(|t| t.parse().ok()).or_else(|| {
        binary
            .and_then(|b| Uuid::from_slice(b).ok())
            .map(ServiceId::Aci)
    })
}

/// The people a body mentions (D72), with Signal's UTF-16 offsets turned
/// into byte offsets of `body`. Text styles (bold, spoilers…) are left out.
fn mentions(body: &str, ranges: &[BodyRange]) -> Vec<Mention> {
    ranges
        .iter()
        .filter_map(|range| {
            let who = match range.associated_value.as_ref()? {
                AssociatedValue::MentionAci(aci) => author(Some(aci), None)?,
                AssociatedValue::MentionAciBinary(aci) => author(None, Some(aci))?,
                AssociatedValue::Style(_) => return None,
            };
            let start = usize::try_from(range.start?).ok()?;
            let end = start.checked_add(usize::try_from(range.length?).ok()?)?;
            let (start, end) = (byte_offset(body, start)?, byte_offset(body, end)?);
            Some(Mention {
                who,
                start,
                len: end.checked_sub(start)?,
            })
        })
        .collect()
}

/// Where the `utf16`th UTF-16 code unit of `text` starts, in bytes; `None`
/// past the end or inside a character.
fn byte_offset(text: &str, utf16: usize) -> Option<usize> {
    let mut units = 0;
    for (at, c) in text.char_indices() {
        if units == utf16 {
            return Some(at);
        }
        units += c.len_utf16();
    }
    (units == utf16).then_some(text.len())
}

fn quote(quote: &data_message::Quote) -> Option<Quote> {
    Some(Quote {
        target: MessageRef {
            sender: author(
                quote.author_aci.as_deref(),
                quote.author_aci_binary.as_deref(),
            )?,
            sent_at: Timestamp::from_millis(quote.id?),
        },
        preview: quote.text.as_deref().map(MessageBody::new),
    })
}

/// Encode a Signal [`AttachmentPointer`] (CDN identifier, key, digest, size)
/// into an [`AttachmentId`] (`p1:<hex>`, D75) so `fetch_attachment` can
/// download and decrypt it without calling into presage's private DB tables.
pub fn encode_pointer(pointer: &AttachmentPointer) -> AttachmentId {
    AttachmentId(format!("p1:{}", hex_encode(&pointer.encode_to_vec())))
}

/// Decode an [`AttachmentId`] produced by [`encode_pointer`].
pub fn decode_pointer(id: &AttachmentId) -> Option<AttachmentPointer> {
    let hex = id.0.strip_prefix("p1:")?;
    let bytes = hex_decode(hex)?;
    AttachmentPointer::decode(bytes.as_slice()).ok()
}

fn attachment(pointer: &AttachmentPointer) -> AttachmentRef {
    AttachmentRef {
        id: encode_pointer(pointer),
        content_type: pointer.content_type.clone().unwrap_or_default(),
        file_name: pointer.file_name.clone(),
        size: pointer.size.map(u64::from),
        width: pointer.width.filter(|&w| w > 0),
        height: pointer.height.filter(|&h| h > 0),
        status: AttachmentStatus::Remote,
    }
}

/// Map a sticker's [`AttachmentPointer`] as an image attachment with no file
/// name and default `512×512` dimensions when unset (D79).
fn sticker_attachment(pointer: &AttachmentPointer) -> AttachmentRef {
    let content_type = pointer
        .content_type
        .clone()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "image/webp".to_owned());
    AttachmentRef {
        id: encode_pointer(pointer),
        content_type,
        file_name: None,
        size: pointer.size.map(u64::from),
        width: pointer.width.filter(|&w| w > 0).or(Some(512)),
        height: pointer.height.filter(|&h| h > 0).or(Some(512)),
        status: AttachmentStatus::Remote,
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        let _ = write!(out, "{b:02x}");
    }
    out
}

fn hex_decode(hex: &str) -> Option<Vec<u8>> {
    if !hex.len().is_multiple_of(2) {
        return None;
    }
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok())
        .collect()
}

fn reacted(
    reaction: &data_message::Reaction,
    conversation: ConversationId,
    sender: ServiceId,
) -> Option<BackendEvent> {
    Some(BackendEvent::Reaction {
        conversation,
        target: MessageRef {
            sender: author(
                reaction.target_author_aci.as_deref(),
                reaction.target_author_aci_binary.as_deref(),
            )?,
            sent_at: Timestamp::from_millis(reaction.target_sent_timestamp?),
        },
        sender,
        emoji: if reaction.remove() {
            None
        } else {
            reaction.emoji.clone()
        },
    })
}

fn edited(edit: &EditMessage, sender: ServiceId, direct: ConversationId) -> Option<BackendEvent> {
    let data = edit.data_message.as_ref()?;
    Some(BackendEvent::Edited {
        conversation: conversation(data, direct),
        target: MessageRef {
            sender,
            sent_at: Timestamp::from_millis(edit.target_sent_timestamp?),
        },
        body: MessageBody::new(data.body.as_deref().unwrap_or("")),
        mentions: mentions(data.body.as_deref().unwrap_or(""), &data.body_ranges),
        edited_at: Timestamp::from_millis(data.timestamp?),
    })
}

/// What this account did on another device (the phone): messages it sent,
/// and messages it read there (D70).
fn synced(sync: &SyncMessage, me: ServiceId) -> Vec<BackendEvent> {
    // `read` sits outside the `content` oneof (protobuf can't put repeated
    // fields in one), so a sync carries either.
    let read: Vec<MessageRef> = sync
        .read
        .iter()
        .filter_map(|read| {
            Some(MessageRef {
                sender: author(
                    read.sender_aci.as_deref(),
                    read.sender_aci_binary.as_deref(),
                )?,
                sent_at: Timestamp::from_millis(read.timestamp?),
            })
        })
        .collect();
    if !read.is_empty() {
        return vec![BackendEvent::ReadElsewhere(read)];
    }
    let Some(sync_message::Content::Sent(sent)) = &sync.content else {
        // Contacts, keys, viewed…: handled elsewhere or not yet.
        return vec![];
    };
    let direct = sent_to(sent).map_or(ConversationId::Direct(me), ConversationId::Direct);
    let mut events = Vec::new();
    if let Some(data) = &sent.message {
        events.extend(data_message(
            data,
            &Author {
                sender: me,
                direct,
                direction: Direction::Outgoing(DeliveryStatus::Sent),
            },
            sent.timestamp.unwrap_or(0),
        ));
    }
    if let Some(edit) = &sent.edit_message {
        events.extend(edited(edit, me, direct));
    }
    events
}

fn sent_to(sent: &Sent) -> Option<ServiceId> {
    sent.parse_destination_service_id().as_ref().map(service_id)
}

fn receipts(receipt: &ReceiptMessage, me: ServiceId) -> Vec<BackendEvent> {
    let status = match receipt.r#type() {
        receipt_message::Type::Delivery => DeliveryStatus::Delivered,
        receipt_message::Type::Read | receipt_message::Type::Viewed => DeliveryStatus::Read,
    };
    receipt
        .timestamp
        .iter()
        .map(|sent_at| BackendEvent::DeliveryUpdate {
            message: MessageRef {
                sender: me,
                sent_at: Timestamp::from_millis(*sent_at),
            },
            status,
        })
        .collect()
}

fn typed(typing: &TypingMessage, sender: ServiceId) -> BackendEvent {
    let conversation = typing
        .group_id
        .as_deref()
        .and_then(|id| <[u8; GroupId::LEN]>::try_from(id).ok())
        .map_or(ConversationId::Direct(sender), |id| {
            ConversationId::Group(GroupId::from_bytes(id))
        });
    BackendEvent::Typing {
        conversation,
        sender,
        state: match typing.action() {
            typing_message::Action::Started => TypingState::Started,
            typing_message::Action::Stopped => TypingState::Stopped,
        },
    }
}

// ---- Outgoing ----

/// The group context every message to a group carries.
pub fn group_context(master_key: &MasterKey, revision: u32) -> GroupContextV2 {
    GroupContextV2 {
        master_key: Some(master_key.to_vec()),
        revision: Some(revision),
        ..GroupContextV2::default()
    }
}

pub fn text(
    msg: &OutgoingMessage,
    attachments: Vec<AttachmentPointer>,
    group: Option<GroupContextV2>,
) -> DataMessage {
    let body = if attachments.is_empty() || !msg.body.expose().is_empty() {
        Some(msg.body.expose().to_owned())
    } else {
        None
    };
    DataMessage {
        body,
        attachments,
        timestamp: Some(msg.sent_at.as_millis()),
        quote: msg.quote.as_ref().map(|q| data_message::Quote {
            id: Some(q.target.sent_at.as_millis()),
            author_aci: Some(q.target.sender.to_string()),
            text: q.preview.as_ref().map(|p| p.expose().to_owned()),
            r#type: Some(data_message::quote::Type::Normal.into()),
            ..data_message::Quote::default()
        }),
        group_v2: group,
        ..DataMessage::default()
    }
}

pub fn attachment_spec(att: &OutgoingAttachment, length: usize) -> AttachmentSpec {
    AttachmentSpec {
        content_type: att.content_type.clone(),
        length,
        file_name: att.file_name.clone(),
        preview: None,
        voice_note: None,
        borderless: None,
        width: att.width,
        height: att.height,
        caption: None,
        blur_hash: None,
    }
}

/// Our reaction, or its removal: a removal names the emoji it takes back, as
/// Signal's apps send it and need it (D73).
pub fn reaction(
    target: MessageRef,
    change: ReactionChange,
    sent_at: u64,
    group: Option<GroupContextV2>,
) -> DataMessage {
    let (emoji, remove) = match change {
        ReactionChange::Set(emoji) => (emoji, false),
        ReactionChange::Remove(emoji) => (emoji, true),
    };
    DataMessage {
        reaction: Some(data_message::Reaction {
            remove: Some(remove),
            emoji: Some(emoji),
            target_author_aci: Some(target.sender.to_string()),
            target_sent_timestamp: Some(target.sent_at.as_millis()),
            ..data_message::Reaction::default()
        }),
        timestamp: Some(sent_at),
        group_v2: group,
        ..DataMessage::default()
    }
}

pub fn edit(target: MessageRef, edit: &OutgoingEdit, group: Option<GroupContextV2>) -> EditMessage {
    EditMessage {
        target_sent_timestamp: Some(target.sent_at.as_millis()),
        data_message: Some(DataMessage {
            body: Some(edit.body.expose().to_owned()),
            timestamp: Some(edit.edited_at.as_millis()),
            group_v2: group,
            ..DataMessage::default()
        }),
    }
}

pub fn delete(target: MessageRef, sent_at: u64, group: Option<GroupContextV2>) -> DataMessage {
    DataMessage {
        delete: Some(Delete {
            target_sent_timestamp: Some(target.sent_at.as_millis()),
        }),
        timestamp: Some(sent_at),
        group_v2: group,
        ..DataMessage::default()
    }
}

pub fn read_receipt(sent_at: Vec<u64>) -> ReceiptMessage {
    ReceiptMessage {
        r#type: Some(receipt_message::Type::Read.into()),
        timestamp: sent_at,
    }
}

/// Tell this account's other devices (the phone) which messages were read
/// here, so they stop counting them as unread (D71). Only others' messages
/// by ACI: ours are never unread, and a read sync names senders by ACI.
pub fn read_sync(messages: &[MessageRef], me: ServiceId) -> SyncMessage {
    SyncMessage {
        read: messages
            .iter()
            .filter(|m| m.sender != me)
            .filter_map(|m| match m.sender {
                ServiceId::Aci(aci) => Some(sync_message::Read {
                    sender_aci: Some(aci.to_string()),
                    timestamp: Some(m.sent_at.as_millis()),
                    ..sync_message::Read::default()
                }),
                ServiceId::Pni(_) => None,
            })
            .collect(),
        ..SyncMessage::default()
    }
}

pub fn typing(state: TypingState, now: u64, group: Option<GroupId>) -> TypingMessage {
    TypingMessage {
        timestamp: Some(now),
        action: Some(
            match state {
                TypingState::Started => typing_message::Action::Started,
                TypingState::Stopped => typing_message::Action::Stopped,
            }
            .into(),
        ),
        group_id: group.map(|g| g.as_bytes().to_vec()),
    }
}

#[cfg(test)]
mod tests;
