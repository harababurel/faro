//! presage's contents ↔ faro's types. Pure functions, so the mapping is
//! tested with hand-built contents and no network.

use faro_core::backend::{
    BackendEvent, MessageRequestAction, OutgoingAttachment, OutgoingEdit, OutgoingMessage,
    ReactionChange,
};
use faro_core::domain::{
    AttachmentId, AttachmentRef, AttachmentStatus, CallKind, CallState, ConversationId,
    DeliveryStatus, Direction, GroupId, LinkPreview, Mention, Message, MessageBody, MessageRef,
    Quote, ServiceId, SystemEvent, Timestamp, TypingState, VerificationState,
};
use presage::libsignal_service::content::{Content, ContentBody};
use presage::libsignal_service::prelude::ProtobufMessage as _;
use presage::libsignal_service::protocol::{Aci, Pni, ServiceId as SignalId};
use presage::libsignal_service::sender::AttachmentSpec;
use presage::libsignal_service::zkgroup::groups::{GroupMasterKey, GroupSecretParams};
use presage::proto::body_range::AssociatedValue;
use presage::proto::call_message;
use presage::proto::data_message::{self, Delete};
use presage::proto::receipt_message;
use presage::proto::sync_message::{self, Sent};
use presage::proto::typing_message;
use presage::proto::{
    AttachmentPointer, BodyRange, CallMessage, DataMessage, EditMessage, GroupContextV2, Preview,
    ReceiptMessage, SyncMessage, TypingMessage, Verified, verified,
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
        // Sync messages come only from this account's own devices; one from
        // anyone else is forged (GHSA-wjhx-4prr-c426 let them through
        // libsignal-service on the PNI path), so it's dropped.
        ContentBody::SynchronizeMessage(sync) if sender == me => synced(sync, me),
        ContentBody::SynchronizeMessage(_) => {
            tracing::warn!(%sender, "dropped a sync message that isn't from this account");
            vec![]
        }
        ContentBody::ReceiptMessage(receipt) => receipts(
            receipt,
            sender,
            u64::try_from(content.metadata.client_timestamp.timestamp_millis()).unwrap_or(0),
            me,
        ),
        ContentBody::TypingMessage(typing) => vec![typed(typing, sender)],
        ContentBody::CallMessage(call) => call_message(
            call,
            sender,
            direct,
            Direction::Incoming,
            u64::try_from(content.metadata.client_timestamp.timestamp_millis()).unwrap_or(0),
        ),
        ContentBody::NullMessage(_)
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
            ContentBody::CallMessage(call) => call_message(
                call,
                me,
                direct,
                Direction::Outgoing(DeliveryStatus::Sent),
                u64::try_from(content.metadata.client_timestamp.timestamp_millis()).unwrap_or(0),
            ),
            // Receipts and typing indicators we sent mean nothing to us.
            ContentBody::ReceiptMessage(_)
            | ContentBody::TypingMessage(_)
            | ContentBody::NullMessage(_)
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
        | BackendEvent::ReadElsewhere(_)
        | BackendEvent::DisappearingTimer { .. }
        | BackendEvent::BlockedSynced { .. }
        | BackendEvent::MessageRequestResponded { .. } => true,
        BackendEvent::Connection(_)
        | BackendEvent::Typing { .. }
        | BackendEvent::ContactUpdated(_)
        | BackendEvent::IdentityUpdated(_)
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
    let previews = link_previews(&data.preview);
    let timer_update =
        (data.flags.unwrap_or(0) & (data_message::Flags::ExpirationTimerUpdate as u32)) != 0;
    let expires_in_secs = data.expire_timer.filter(|&s| s > 0);
    let sent_at = Timestamp::from_millis(data.timestamp.unwrap_or(fallback_sent_at));
    let mut out = Vec::new();
    if timer_update {
        out.push(BackendEvent::DisappearingTimer {
            conversation,
            seconds: expires_in_secs,
        });
        if body.is_none() && attachments.is_empty() && previews.is_empty() {
            out.push(BackendEvent::Message(Message::system(
                conversation,
                author.sender,
                sent_at,
                author.direction,
                SystemEvent::DisappearingTimer {
                    seconds: expires_in_secs,
                },
            )));
        }
    }
    if body.is_none() && attachments.is_empty() && previews.is_empty() {
        if data
            .group_call_update
            .as_ref()
            .and_then(|u| u.era_id.as_deref())
            .is_some_and(|e| !e.is_empty())
        {
            out.push(BackendEvent::Message(Message::system(
                conversation,
                author.sender,
                sent_at,
                author.direction,
                SystemEvent::Call {
                    kind: CallKind::Group,
                    state: CallState::Incoming,
                },
            )));
        }
        // Profile key, timer and group changes, payments, calls…: nothing else to show.
        return out;
    }
    let expire_started_at = expires_in_secs
        .and_then(|_| matches!(author.direction, Direction::Outgoing(_)).then_some(sent_at));
    out.push(BackendEvent::Message(Message {
        conversation,
        sender: author.sender,
        sent_at,
        direction: author.direction,
        body,
        mentions: data
            .body
            .as_deref()
            .map(|text| mentions(text, &data.body_ranges))
            .unwrap_or_default(),
        quote: data.quote.as_ref().and_then(quote),
        previews,
        attachments,
        reactions: vec![],
        edited_at: None,
        deleted: false,
        expires_in_secs,
        expire_started_at,
        system_event: None,
    }));
    out
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
pub(crate) fn byte_offset(text: &str, utf16: usize) -> Option<usize> {
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

/// Map Signal's `DataMessage.preview` entries into [`LinkPreview`]s (#02).
/// Remote thumbnail pointers (`Preview.image`) are deliberately ignored so
/// viewing a chat never fetches external preview images.
fn link_previews(previews: &[Preview]) -> Vec<LinkPreview> {
    previews
        .iter()
        .filter_map(|p| {
            let url = p.url.as_deref()?.trim();
            if url.is_empty() {
                return None;
            }
            let title = p
                .title
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned);
            let description = p
                .description
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned);
            if title.is_none() && description.is_none() {
                return None;
            }
            Some(LinkPreview {
                url: url.to_owned(),
                title,
                description,
            })
        })
        .collect()
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

pub(crate) fn attachment(pointer: &AttachmentPointer) -> AttachmentRef {
    let voice_note = (pointer.flags.unwrap_or(0) & 1) != 0;
    AttachmentRef {
        id: encode_pointer(pointer),
        content_type: pointer.content_type.clone().unwrap_or_default(),
        file_name: pointer.file_name.clone(),
        size: pointer.size.map(u64::from),
        width: pointer.width.filter(|&w| w > 0),
        height: pointer.height.filter(|&h| h > 0),
        status: AttachmentStatus::Remote,
        voice_note,
        duration_secs: None,
        waveform: None,
        transcript: None,
        transcript_source: None,
    }
}

/// Map a sticker's [`AttachmentPointer`] as an image attachment with no file
/// name and default `512×512` dimensions when unset (D79).
pub(crate) fn sticker_attachment(pointer: &AttachmentPointer) -> AttachmentRef {
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
        voice_note: false,
        duration_secs: None,
        waveform: None,
        transcript: None,
        transcript_source: None,
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
    if let Some(sync_message::Content::CallEvent(ev)) = &sync.content {
        return call_event_sync(ev, me).into_iter().collect();
    }
    if let Some(sync_message::Content::Blocked(blocked)) = &sync.content {
        return vec![blocked_sync(blocked)];
    }
    if let Some(sync_message::Content::MessageRequestResponse(resp)) = &sync.content {
        return message_request_response(resp).into_iter().collect();
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

fn call_message(
    call: &CallMessage,
    sender: ServiceId,
    direct: ConversationId,
    direction: Direction,
    timestamp_ms: u64,
) -> Vec<BackendEvent> {
    let sent_at = Timestamp::from_millis(timestamp_ms);
    if let Some(offer) = &call.offer {
        if direction != Direction::Incoming {
            return vec![];
        }
        let kind = match offer.r#type() {
            call_message::offer::Type::OfferVideoCall => CallKind::Video,
            call_message::offer::Type::OfferAudioCall => CallKind::Voice,
        };
        return vec![BackendEvent::Message(Message::system(
            direct,
            sender,
            sent_at,
            direction,
            SystemEvent::Call {
                kind,
                state: CallState::Incoming,
            },
        ))];
    }
    if let Some(hangup) = &call.hangup {
        let state = match hangup.r#type() {
            call_message::hangup::Type::HangupAccepted => CallState::Accepted,
            call_message::hangup::Type::HangupNormal
            | call_message::hangup::Type::HangupDeclined
            | call_message::hangup::Type::HangupBusy
            | call_message::hangup::Type::HangupNeedPermission => CallState::Missed,
        };
        return vec![BackendEvent::Message(Message::system(
            direct,
            sender,
            sent_at,
            direction,
            SystemEvent::Call {
                kind: CallKind::Voice,
                state,
            },
        ))];
    }
    if call.busy.is_some() {
        return vec![BackendEvent::Message(Message::system(
            direct,
            sender,
            sent_at,
            direction,
            SystemEvent::Call {
                kind: CallKind::Voice,
                state: CallState::Missed,
            },
        ))];
    }
    vec![]
}

fn call_event_sync(ev: &sync_message::CallEvent, me: ServiceId) -> Option<BackendEvent> {
    let conv_bytes = ev.conversation_id.as_deref()?;
    let conversation = if let Ok(uuid) = Uuid::from_slice(conv_bytes) {
        ConversationId::Direct(ServiceId::Aci(uuid))
    } else if let Ok(group_bytes) = <[u8; GroupId::LEN]>::try_from(conv_bytes) {
        ConversationId::Group(GroupId::from_bytes(group_bytes))
    } else {
        return None;
    };
    let kind = match ev.r#type() {
        sync_message::call_event::Type::AudioCall => CallKind::Voice,
        sync_message::call_event::Type::VideoCall => CallKind::Video,
        sync_message::call_event::Type::GroupCall | sync_message::call_event::Type::AdHocCall => {
            CallKind::Group
        }
        sync_message::call_event::Type::UnknownType => return None,
    };
    let state = match ev.event() {
        sync_message::call_event::Event::Accepted => CallState::Accepted,
        sync_message::call_event::Event::NotAccepted
        | sync_message::call_event::Event::Observed => CallState::Missed,
        sync_message::call_event::Event::Delete | sync_message::call_event::Event::UnknownEvent => {
            return None;
        }
    };
    let direction = match ev.direction() {
        sync_message::call_event::Direction::Outgoing => Direction::Outgoing(DeliveryStatus::Sent),
        sync_message::call_event::Direction::Incoming
        | sync_message::call_event::Direction::UnknownDirection => Direction::Incoming,
    };
    let sender =
        if let (Direction::Incoming, ConversationId::Direct(peer)) = (direction, conversation) {
            peer
        } else {
            me
        };
    let sent_at = Timestamp::from_millis(ev.timestamp?);
    Some(BackendEvent::Message(Message::system(
        conversation,
        sender,
        sent_at,
        direction,
        SystemEvent::Call { kind, state },
    )))
}

fn blocked_sync(blocked: &sync_message::Blocked) -> BackendEvent {
    let mut conversations = Vec::new();
    let mut push = |conv: ConversationId| {
        if !conversations.contains(&conv) {
            conversations.push(conv);
        }
    };
    for aci in &blocked.acis {
        if let Ok(uuid) = Uuid::parse_str(aci) {
            push(ConversationId::Direct(ServiceId::Aci(uuid)));
        }
    }
    for bytes in &blocked.acis_binary {
        if let Ok(uuid) = Uuid::from_slice(bytes) {
            push(ConversationId::Direct(ServiceId::Aci(uuid)));
        }
    }
    for group_bytes in &blocked.group_ids {
        if let Ok(arr) = <[u8; GroupId::LEN]>::try_from(group_bytes.as_slice()) {
            push(ConversationId::Group(GroupId::from_bytes(arr)));
        }
    }
    BackendEvent::BlockedSynced { conversations }
}

fn message_request_response(resp: &sync_message::MessageRequestResponse) -> Option<BackendEvent> {
    let conversation = if let Some(peer) = author(
        resp.thread_aci.as_deref(),
        resp.thread_aci_binary.as_deref(),
    ) {
        ConversationId::Direct(peer)
    } else if let Some(group_bytes) = resp.group_id.as_deref()
        && let Ok(arr) = <[u8; GroupId::LEN]>::try_from(group_bytes)
    {
        ConversationId::Group(GroupId::from_bytes(arr))
    } else {
        return None;
    };
    let action = match resp.r#type() {
        sync_message::message_request_response::Type::Accept => MessageRequestAction::Accept,
        sync_message::message_request_response::Type::Delete => MessageRequestAction::Delete,
        sync_message::message_request_response::Type::Block => MessageRequestAction::Block,
        sync_message::message_request_response::Type::BlockAndDelete => {
            MessageRequestAction::BlockAndDelete
        }
        sync_message::message_request_response::Type::Spam => MessageRequestAction::Spam,
        sync_message::message_request_response::Type::BlockAndSpam => {
            MessageRequestAction::BlockAndSpam
        }
        sync_message::message_request_response::Type::Unknown => return None,
    };
    Some(BackendEvent::MessageRequestResponded {
        conversation,
        action,
    })
}

fn sent_to(sent: &Sent) -> Option<ServiceId> {
    sent.parse_destination_service_id().as_ref().map(service_id)
}

fn receipts(
    receipt: &ReceiptMessage,
    sender: ServiceId,
    at: u64,
    me: ServiceId,
) -> Vec<BackendEvent> {
    let status = match receipt.r#type() {
        receipt_message::Type::Delivery => DeliveryStatus::Delivered,
        receipt_message::Type::Read | receipt_message::Type::Viewed => DeliveryStatus::Read,
    };
    let updated_at = Some(Timestamp::from_millis(at));
    receipt
        .timestamp
        .iter()
        .map(|sent_at| BackendEvent::DeliveryUpdate {
            message: MessageRef {
                sender: me,
                sent_at: Timestamp::from_millis(*sent_at),
            },
            status,
            by: Some(sender),
            updated_at,
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

/// Signal's `BodyRange`s for outgoing mentions, in UTF-16 code units.
pub fn body_ranges(text: &str, mentions: &[Mention]) -> Vec<BodyRange> {
    mentions
        .iter()
        .filter_map(|mention| {
            let ServiceId::Aci(aci) = mention.who else {
                return None;
            };
            let end = mention.start.checked_add(mention.len)?;
            let prefix = text.get(..mention.start)?;
            let slice = text.get(mention.start..end)?;
            let start = u32::try_from(prefix.encode_utf16().count()).ok()?;
            let length = u32::try_from(slice.encode_utf16().count()).ok()?;
            if length == 0 {
                return None;
            }
            Some(BodyRange {
                start: Some(start),
                length: Some(length),
                associated_value: Some(AssociatedValue::MentionAci(aci.to_string())),
            })
        })
        .collect()
}

pub fn text(
    msg: &OutgoingMessage,
    attachments: Vec<AttachmentPointer>,
    group: Option<GroupContextV2>,
) -> DataMessage {
    let inline = faro_core::domain::truncate_long_text(msg.body.expose());
    let body = if attachments.is_empty() || !inline.is_empty() {
        Some(inline.to_owned())
    } else {
        None
    };
    DataMessage {
        body,
        body_ranges: body_ranges(inline, &msg.mentions),
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

pub fn long_text_spec(length: usize) -> AttachmentSpec {
    AttachmentSpec {
        content_type: faro_core::domain::LONG_TEXT_MIME.to_owned(),
        length,
        file_name: None,
        preview: None,
        voice_note: None,
        borderless: None,
        width: None,
        height: None,
        caption: None,
        blur_hash: None,
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
            body_ranges: body_ranges(edit.body.expose(), &edit.mentions),
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

/// A disappearing-message timer update (`Flags::ExpirationTimerUpdate`, D165).
pub fn timer_update(
    seconds: Option<u32>,
    sent_at: u64,
    group: Option<GroupContextV2>,
) -> DataMessage {
    DataMessage {
        flags: Some(data_message::Flags::ExpirationTimerUpdate as u32),
        expire_timer: Some(seconds.unwrap_or(0)),
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

/// A safety-number verification change synced from one of this account's other
/// devices (D163).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedSync {
    pub peer: ServiceId,
    pub identity_key: Option<Vec<u8>>,
    pub state: VerificationState,
}

/// Extract a [`VerifiedSync`] when `content` is a verification sync message
/// from this account.
pub fn verified_sync(content: &Content, me: ServiceId) -> Option<VerifiedSync> {
    if service_id(&content.metadata.sender) != me {
        return None;
    }
    let ContentBody::SynchronizeMessage(SyncMessage {
        content: Some(sync_message::Content::Verified(verified)),
        ..
    }) = &content.body
    else {
        return None;
    };
    let peer = author(
        verified.destination_aci.as_deref(),
        verified.destination_aci_binary.as_deref(),
    )?;
    let state = verification_state(verified.state());
    Some(VerifiedSync {
        peer,
        identity_key: verified.identity_key.clone(),
        state,
    })
}

pub fn verification_state(state: verified::State) -> VerificationState {
    match state {
        verified::State::Default => VerificationState::Default,
        verified::State::Verified => VerificationState::Verified,
        verified::State::Unverified => VerificationState::Unverified,
    }
}

pub fn proto_verification_state(state: VerificationState) -> verified::State {
    match state {
        VerificationState::Default => verified::State::Default,
        VerificationState::Verified => verified::State::Verified,
        VerificationState::Unverified => verified::State::Unverified,
    }
}

/// Build a `SyncMessage::Verified` to tell this account's other devices that
/// `peer`'s verification state changed (D163).
pub fn verified_sync_message(
    peer: ServiceId,
    identity_key: Vec<u8>,
    state: VerificationState,
) -> Option<SyncMessage> {
    let ServiceId::Aci(aci) = peer else {
        return None;
    };
    Some(SyncMessage {
        content: Some(sync_message::Content::Verified(Verified {
            destination_aci: Some(aci.to_string()),
            destination_aci_binary: Some(aci.as_bytes().to_vec()),
            identity_key: Some(identity_key),
            state: Some(proto_verification_state(state).into()),
            null_message: None,
        })),
        ..SyncMessage::default()
    })
}

/// Ask the primary device to send its current blocklist (`SyncMessage::Blocked`, D176).
pub fn request_blocked_sync() -> SyncMessage {
    SyncMessage {
        content: Some(sync_message::Content::Request(sync_message::Request {
            r#type: Some(sync_message::request::Type::Blocked.into()),
        })),
        ..SyncMessage::default()
    }
}

/// Build a `SyncMessage::MessageRequestResponse` to sync an accept/delete/block/spam
/// decision across this account's linked devices (D176).
pub fn message_request_response_sync(
    conversation: ConversationId,
    action: MessageRequestAction,
) -> Option<SyncMessage> {
    let response_type = match action {
        MessageRequestAction::Accept => sync_message::message_request_response::Type::Accept,
        MessageRequestAction::Delete => sync_message::message_request_response::Type::Delete,
        MessageRequestAction::Block => sync_message::message_request_response::Type::Block,
        MessageRequestAction::BlockAndDelete => {
            sync_message::message_request_response::Type::BlockAndDelete
        }
        MessageRequestAction::Spam => sync_message::message_request_response::Type::Spam,
        MessageRequestAction::BlockAndSpam => {
            sync_message::message_request_response::Type::BlockAndSpam
        }
        MessageRequestAction::Unblock => return None,
    };
    let (thread_aci, thread_aci_binary, group_id) = match conversation {
        ConversationId::Direct(ServiceId::Aci(aci)) => {
            (Some(aci.to_string()), Some(aci.as_bytes().to_vec()), None)
        }
        ConversationId::Direct(ServiceId::Pni(_)) => return None,
        ConversationId::Group(group) => (None, None, Some(group.as_bytes().to_vec())),
    };
    Some(SyncMessage {
        content: Some(sync_message::Content::MessageRequestResponse(
            sync_message::MessageRequestResponse {
                thread_aci,
                thread_aci_binary,
                group_id,
                r#type: Some(response_type.into()),
            },
        )),
        ..SyncMessage::default()
    })
}

/// Build a `SyncMessage::Blocked` carrying this account's full blocked set (D176).
pub fn blocked_sync_message(conversations: &[ConversationId]) -> SyncMessage {
    let mut acis = Vec::new();
    let mut acis_binary = Vec::new();
    let mut group_ids = Vec::new();
    for &conversation in conversations {
        match conversation {
            ConversationId::Direct(ServiceId::Aci(aci)) => {
                acis.push(aci.to_string());
                acis_binary.push(aci.as_bytes().to_vec());
            }
            ConversationId::Direct(ServiceId::Pni(_)) => {}
            ConversationId::Group(group) => {
                group_ids.push(group.as_bytes().to_vec());
            }
        }
    }
    SyncMessage {
        content: Some(sync_message::Content::Blocked(sync_message::Blocked {
            numbers: Vec::new(),
            acis,
            group_ids,
            acis_binary,
        })),
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

/// System events for changes between a previously known group snapshot and a
/// newer revision (`old_revision < new_revision`, D167).
pub fn group_diff(
    old_title: &str,
    old_members: &[ServiceId],
    old_timer: Option<u32>,
    new_title: &str,
    new_members: &[ServiceId],
    new_timer: Option<u32>,
) -> Vec<SystemEvent> {
    let mut events = Vec::new();
    if old_title != new_title && !new_title.trim().is_empty() {
        events.push(SystemEvent::GroupRenamed {
            title: new_title.to_owned(),
        });
    }
    let joined: Vec<ServiceId> = new_members
        .iter()
        .copied()
        .filter(|m| !old_members.contains(m))
        .collect();
    if !joined.is_empty() {
        events.push(SystemEvent::MembersJoined { members: joined });
    }
    let left: Vec<ServiceId> = old_members
        .iter()
        .copied()
        .filter(|m| !new_members.contains(m))
        .collect();
    if !left.is_empty() {
        events.push(SystemEvent::MembersLeft { members: left });
    }
    if old_timer != new_timer {
        events.push(SystemEvent::DisappearingTimer { seconds: new_timer });
    }
    events
}

#[cfg(test)]
mod tests;
