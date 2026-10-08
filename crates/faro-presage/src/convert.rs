//! presage's contents ↔ faro's types. Pure functions, so the mapping is
//! tested with hand-built contents and no network.

use faro_core::backend::{BackendEvent, OutgoingEdit, OutgoingMessage};
use faro_core::domain::{
    AttachmentId, AttachmentRef, ConversationId, DeliveryStatus, Direction, GroupId, Message,
    MessageBody, MessageRef, Quote, ServiceId, Timestamp, TypingState,
};
use presage::libsignal_service::content::{Content, ContentBody};
use presage::libsignal_service::protocol::{Aci, Pni, ServiceId as SignalId};
use presage::libsignal_service::zkgroup::groups::{GroupMasterKey, GroupSecretParams};
use presage::proto::attachment_pointer::AttachmentIdentifier;
use presage::proto::data_message::{self, Delete};
use presage::proto::receipt_message;
use presage::proto::sync_message::{self, Sent};
use presage::proto::typing_message;
use presage::proto::{
    AttachmentPointer, DataMessage, EditMessage, GroupContextV2, ReceiptMessage, SyncMessage,
    TypingMessage,
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
        | BackendEvent::Deleted { .. } => true,
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
    let attachments: Vec<AttachmentRef> = data.attachments.iter().map(attachment).collect();
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

fn attachment(pointer: &AttachmentPointer) -> AttachmentRef {
    let id = match &pointer.attachment_identifier {
        Some(AttachmentIdentifier::CdnId(id)) => id.to_string(),
        Some(AttachmentIdentifier::CdnKey(key)) => key.clone(),
        None => String::new(),
    };
    AttachmentRef {
        id: AttachmentId(id),
        content_type: pointer.content_type.clone().unwrap_or_default(),
        file_name: pointer.file_name.clone(),
        size: pointer.size.map(u64::from),
    }
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
        edited_at: Timestamp::from_millis(data.timestamp?),
    })
}

/// Messages this account sent from another device (the phone).
fn synced(sync: &SyncMessage, me: ServiceId) -> Vec<BackendEvent> {
    let Some(sync_message::Content::Sent(sent)) = &sync.content else {
        // Read syncs, contacts, keys…: handled elsewhere or not yet.
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

pub fn text(msg: &OutgoingMessage, group: Option<GroupContextV2>) -> DataMessage {
    DataMessage {
        body: Some(msg.body.expose().to_owned()),
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

pub fn reaction(
    target: MessageRef,
    emoji: Option<String>,
    sent_at: u64,
    group: Option<GroupContextV2>,
) -> DataMessage {
    DataMessage {
        reaction: Some(data_message::Reaction {
            remove: Some(emoji.is_none()),
            emoji,
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
