use presage::libsignal_service::content::Metadata;
use presage::libsignal_service::protocol::DeviceId;
use presage::proto::data_message::Reaction;
use presage::proto::{SyncMessage, sync_message};

use super::*;

const ME: ServiceId = ServiceId::Aci(Uuid::from_u128(1));
const ALICE: ServiceId = ServiceId::Aci(Uuid::from_u128(2));
const KEY: MasterKey = [7; 32];

fn from(sender: ServiceId, body: impl Into<ContentBody>) -> Content {
    Content::from_body(
        body,
        Metadata {
            sender: signal_id(sender),
            destination: signal_id(ME),
            sender_device: DeviceId::try_from(1u32).unwrap(),
            pni_verified: None,
            client_timestamp: chrono_default(),
            server_timestamp: chrono_default(),
            needs_receipt: true,
            unidentified_sender: true,
            was_plaintext: false,
            server_guid: None,
        },
    )
}

/// The epoch, without naming chrono's types.
fn chrono_default<T: Default>() -> T {
    T::default()
}

fn text_from_alice(body: &str, sent_at: u64) -> DataMessage {
    DataMessage {
        body: Some(body.to_owned()),
        timestamp: Some(sent_at),
        ..DataMessage::default()
    }
}

fn only(events: Vec<BackendEvent>) -> BackendEvent {
    assert_eq!(events.len(), 1, "{events:?}");
    events.into_iter().next().unwrap()
}

#[test]
fn ids_round_trip() {
    assert_eq!(service_id(&signal_id(ALICE)), ALICE);
    let pni = ServiceId::Pni(Uuid::from_u128(3));
    assert_eq!(service_id(&signal_id(pni)), pni);
}

#[test]
fn group_ids_are_derived_not_the_secret() {
    let id = group_id(&KEY);
    assert_ne!(id.as_bytes(), &KEY);
    assert_eq!(id, group_id(&KEY), "stable");
    assert_ne!(id, group_id(&[8; 32]));
}

#[test]
fn incoming_text_is_a_message_in_the_senders_chat() {
    let content = from(ALICE, text_from_alice("hi", 1000));
    let BackendEvent::Message(message) = only(events(&content, ME)) else {
        panic!("expected a message");
    };
    assert_eq!(message.conversation, ConversationId::Direct(ALICE));
    assert_eq!(message.sender, ALICE);
    assert_eq!(message.sent_at, Timestamp::from_millis(1000));
    assert_eq!(message.direction, Direction::Incoming);
    assert_eq!(message.body, Some(MessageBody::new("hi")));
}

#[test]
fn group_messages_go_to_the_group() {
    let data = DataMessage {
        group_v2: Some(group_context(&KEY, 3)),
        ..text_from_alice("hi all", 1000)
    };
    let content = from(ALICE, data);
    assert_eq!(group_of(&content), Some((KEY, 3)));
    let BackendEvent::Message(message) = only(events(&content, ME)) else {
        panic!("expected a message");
    };
    assert_eq!(message.conversation, ConversationId::Group(group_id(&KEY)));
}

#[test]
fn quotes_keep_their_target() {
    let data = DataMessage {
        quote: Some(data_message::Quote {
            id: Some(500),
            author_aci: Some(ME.to_string()),
            text: Some("earlier".to_owned()),
            ..data_message::Quote::default()
        }),
        ..text_from_alice("reply", 1000)
    };
    let BackendEvent::Message(message) = only(events(&from(ALICE, data), ME)) else {
        panic!("expected a message");
    };
    let quote = message.quote.unwrap();
    assert_eq!(
        quote.target,
        MessageRef {
            sender: ME,
            sent_at: Timestamp::from_millis(500)
        }
    );
    assert_eq!(quote.preview, Some(MessageBody::new("earlier")));
}

#[test]
fn empty_data_messages_show_nothing() {
    // e.g. a profile key update
    let data = DataMessage {
        profile_key: Some(vec![1; 32]),
        timestamp: Some(5),
        ..DataMessage::default()
    };
    assert_eq!(events(&from(ALICE, data), ME), vec![]);
}

#[test]
fn reactions_and_removals() {
    let react = |remove| DataMessage {
        reaction: Some(Reaction {
            emoji: Some("👍".to_owned()),
            remove: Some(remove),
            target_author_aci: Some(ME.to_string()),
            target_sent_timestamp: Some(500),
            ..Reaction::default()
        }),
        timestamp: Some(1000),
        ..DataMessage::default()
    };
    let target = MessageRef {
        sender: ME,
        sent_at: Timestamp::from_millis(500),
    };
    assert_eq!(
        only(events(&from(ALICE, react(false)), ME)),
        BackendEvent::Reaction {
            conversation: ConversationId::Direct(ALICE),
            target,
            sender: ALICE,
            emoji: Some("👍".to_owned()),
        }
    );
    let BackendEvent::Reaction { emoji, .. } = only(events(&from(ALICE, react(true)), ME)) else {
        panic!("expected a reaction");
    };
    assert_eq!(emoji, None);
}

#[test]
fn binary_authors_are_understood() {
    let data = DataMessage {
        reaction: Some(Reaction {
            emoji: Some("❤".to_owned()),
            target_author_aci_binary: Some(ME.uuid().as_bytes().to_vec()),
            target_sent_timestamp: Some(500),
            ..Reaction::default()
        }),
        ..DataMessage::default()
    };
    let BackendEvent::Reaction { target, .. } = only(events(&from(ALICE, data), ME)) else {
        panic!("expected a reaction");
    };
    assert_eq!(target.sender, ME);
}

#[test]
fn deletions_and_edits_target_the_senders_message() {
    let delete = DataMessage {
        delete: Some(Delete {
            target_sent_timestamp: Some(500),
        }),
        timestamp: Some(1000),
        ..DataMessage::default()
    };
    let target = MessageRef {
        sender: ALICE,
        sent_at: Timestamp::from_millis(500),
    };
    assert_eq!(
        only(events(&from(ALICE, delete), ME)),
        BackendEvent::Deleted {
            conversation: ConversationId::Direct(ALICE),
            target,
        }
    );
    let edit = EditMessage {
        target_sent_timestamp: Some(500),
        data_message: Some(text_from_alice("fixed", 2000)),
    };
    assert_eq!(
        only(events(&from(ALICE, edit), ME)),
        BackendEvent::Edited {
            conversation: ConversationId::Direct(ALICE),
            target,
            body: MessageBody::new("fixed"),
            mentions: vec![],
            edited_at: Timestamp::from_millis(2000),
        }
    );
}

#[test]
fn mentions_point_at_their_placeholder_in_bytes() {
    let mention = |start, length, value| BodyRange {
        start: Some(start),
        length: Some(length),
        associated_value: Some(value),
    };
    // "🧗" is two UTF-16 units but four bytes; "é" one unit, two bytes.
    let mut data = text_from_alice("🧗 é \u{FFFC}, \u{FFFC}!", 1000);
    data.body_ranges = vec![
        mention(5, 1, AssociatedValue::MentionAci(ME.to_string())),
        mention(
            8,
            1,
            AssociatedValue::MentionAciBinary(ALICE.uuid().as_bytes().to_vec()),
        ),
        // Styles aren't mentions; ranges past the end or inside the
        // climber's surrogate pair are dropped.
        mention(0, 2, AssociatedValue::Style(1)),
        mention(30, 1, AssociatedValue::MentionAci(ME.to_string())),
        mention(1, 1, AssociatedValue::MentionAci(ME.to_string())),
    ];
    let BackendEvent::Message(message) = only(events(&from(ALICE, data), ME)) else {
        panic!("expected a message");
    };
    let body = message.body.as_ref().map_or("", MessageBody::expose);
    assert_eq!(
        message.mentions,
        vec![
            Mention {
                who: ME,
                start: 8,
                len: 3
            },
            Mention {
                who: ALICE,
                start: 13,
                len: 3
            },
        ]
    );
    for mention in &message.mentions {
        assert_eq!(
            body.get(mention.start..mention.start + mention.len),
            Some("\u{FFFC}")
        );
    }
    assert!(message.mentions_person(ME));
}

#[test]
fn messages_sent_from_the_phone_are_outgoing() {
    let sync = SyncMessage {
        content: Some(sync_message::Content::Sent(Sent {
            destination_service_id: Some(ALICE.to_string()),
            timestamp: Some(1000),
            message: Some(text_from_alice("from my phone", 1000)),
            ..Sent::default()
        })),
        ..SyncMessage::default()
    };
    let BackendEvent::Message(message) = only(events(&from(ME, sync), ME)) else {
        panic!("expected a message");
    };
    assert_eq!(message.conversation, ConversationId::Direct(ALICE));
    assert_eq!(message.sender, ME);
    assert_eq!(message.direction, Direction::Outgoing(DeliveryStatus::Sent));
}

#[test]
fn reads_on_the_phone_point_at_the_messages_read() {
    let sync = SyncMessage {
        read: vec![
            sync_message::Read {
                sender_aci: Some(ALICE.to_string()),
                timestamp: Some(1000),
                ..sync_message::Read::default()
            },
            sync_message::Read {
                sender_aci_binary: Some(ALICE.uuid().as_bytes().to_vec()),
                timestamp: Some(2000),
                ..sync_message::Read::default()
            },
            // Unusable entries are skipped.
            sync_message::Read {
                timestamp: Some(3000),
                ..sync_message::Read::default()
            },
        ],
        ..SyncMessage::default()
    };
    let read = |at| MessageRef {
        sender: ALICE,
        sent_at: Timestamp::from_millis(at),
    };
    assert_eq!(
        only(events(&from(ME, sync.clone()), ME)),
        BackendEvent::ReadElsewhere(vec![read(1000), read(2000)])
    );
    // Catching up keeps them too (the store applies them).
    assert_eq!(stored(&from(ME, sync), ME).len(), 1);
    let nothing = SyncMessage {
        read: vec![sync_message::Read::default()],
        ..SyncMessage::default()
    };
    assert_eq!(events(&from(ME, nothing), ME), vec![]);
}

#[test]
fn receipts_update_our_messages() {
    let receipt = ReceiptMessage {
        r#type: Some(receipt_message::Type::Read.into()),
        timestamp: vec![10, 20],
    };
    assert_eq!(
        events(&from(ALICE, receipt), ME),
        [10, 20]
            .map(|t| BackendEvent::DeliveryUpdate {
                message: MessageRef {
                    sender: ME,
                    sent_at: Timestamp::from_millis(t)
                },
                status: DeliveryStatus::Read,
            })
            .to_vec()
    );
}

#[test]
fn typing_in_a_group() {
    let id = group_id(&KEY);
    let content = from(ALICE, typing(TypingState::Started, 5, Some(id)));
    assert_eq!(
        only(events(&content, ME)),
        BackendEvent::Typing {
            conversation: ConversationId::Group(id),
            sender: ALICE,
            state: TypingState::Started,
        }
    );
}

#[test]
fn outgoing_text_carries_quote_and_group() {
    let msg = OutgoingMessage {
        sent_at: Timestamp::from_millis(42),
        body: MessageBody::new("hello"),
        quote: Some(Quote {
            target: MessageRef {
                sender: ALICE,
                sent_at: Timestamp::from_millis(7),
            },
            preview: None,
        }),
        attachments: vec![],
    };
    let data = text(&msg, vec![], Some(group_context(&KEY, 2)));
    assert_eq!(data.body.as_deref(), Some("hello"));
    assert_eq!(data.timestamp, Some(42));
    let quote = data.quote.unwrap();
    assert_eq!(quote.id, Some(7));
    assert_eq!(quote.author_aci, Some(ALICE.to_string()));
    assert_eq!(master_key(data.group_v2.as_ref()), Some(KEY));
}

#[test]
fn attachment_pointers_and_stickers_round_trip() {
    use presage::proto::attachment_pointer::AttachmentIdentifier;

    let pointer = AttachmentPointer {
        content_type: Some("image/png".to_owned()),
        key: Some(vec![0xab; 64]),
        size: Some(4096),
        digest: Some(vec![0xcd; 32]),
        file_name: Some("summit.png".to_owned()),
        width: Some(640),
        height: Some(480),
        attachment_identifier: Some(AttachmentIdentifier::CdnKey("cdn-key-1".to_owned())),
        ..AttachmentPointer::default()
    };
    let data = DataMessage {
        attachments: vec![pointer.clone()],
        timestamp: Some(1000),
        ..DataMessage::default()
    };
    let BackendEvent::Message(message) = only(events(&from(ALICE, data), ME)) else {
        panic!("expected a message");
    };
    assert_eq!(message.attachments.len(), 1);
    let att = &message.attachments[0];
    assert_eq!(att.content_type, "image/png");
    assert_eq!(att.file_name.as_deref(), Some("summit.png"));
    assert_eq!(att.size, Some(4096));
    assert_eq!(att.width, Some(640));
    assert_eq!(att.height, Some(480));
    assert_eq!(att.status, AttachmentStatus::Remote);
    assert_eq!(decode_pointer(&att.id), Some(pointer.clone()));

    // Sticker carries an AttachmentPointer with no file name and 512x512 fallback.
    let sticker_ptr = AttachmentPointer {
        content_type: None,
        width: None,
        height: None,
        file_name: None,
        ..pointer
    };
    let sticker_data = DataMessage {
        sticker: Some(data_message::Sticker {
            pack_id: Some(vec![1; 16]),
            pack_key: Some(vec![2; 32]),
            sticker_id: Some(3),
            data: Some(sticker_ptr.clone()),
            emoji: Some("🧗".to_owned()),
        }),
        timestamp: Some(2000),
        ..DataMessage::default()
    };
    let BackendEvent::Message(sticker_msg) = only(events(&from(ALICE, sticker_data), ME)) else {
        panic!("expected a sticker message");
    };
    assert_eq!(sticker_msg.body, Some(MessageBody::new("[sticker] 🧗")));
    assert_eq!(sticker_msg.attachments.len(), 1);
    let s_att = &sticker_msg.attachments[0];
    assert!(s_att.is_sticker());
    assert_eq!(s_att.content_type, "image/webp");
    assert_eq!(s_att.width, Some(512));
    assert_eq!(s_att.height, Some(512));
    assert_eq!(decode_pointer(&s_att.id), Some(sticker_ptr));

    // VoiceMessage flag (1) sets voice_note = true.
    let voice_ptr = AttachmentPointer {
        content_type: Some("audio/aac".to_owned()),
        flags: Some(1),
        size: Some(18_000),
        ..AttachmentPointer::default()
    };
    let voice_data = DataMessage {
        attachments: vec![voice_ptr],
        timestamp: Some(3000),
        ..DataMessage::default()
    };
    let BackendEvent::Message(voice_msg) = only(events(&from(ALICE, voice_data), ME)) else {
        panic!("expected a voice message");
    };
    assert!(voice_msg.attachments[0].voice_note);
    assert!(voice_msg.attachments[0].is_voice_note());
}

#[test]
fn outgoing_reactions_and_removals_name_the_emoji() {
    let target = MessageRef {
        sender: ALICE,
        sent_at: Timestamp::from_millis(7),
    };
    let set = reaction(target, ReactionChange::Set("👍".into()), 9, None)
        .reaction
        .unwrap();
    assert_eq!(set.remove, Some(false));
    assert_eq!(set.emoji.as_deref(), Some("👍"));
    assert_eq!(set.target_author_aci, Some(ALICE.to_string()));

    let removed = reaction(target, ReactionChange::Remove("👍".into()), 9, None)
        .reaction
        .unwrap();
    assert_eq!(removed.remove, Some(true));
    assert_eq!(
        removed.emoji.as_deref(),
        Some("👍"),
        "Signal's apps need it"
    );
    assert_eq!(removed.target_sent_timestamp, Some(7));
}

#[test]
fn what_this_device_sent_is_kept_as_outgoing_to_the_recipient() {
    let mut content = from(ME, text_from_alice("sent by faro", 1000));
    content.metadata.destination = signal_id(ALICE);
    let BackendEvent::Message(message) = only(stored(&content, ME)) else {
        panic!("expected a message");
    };
    assert_eq!(message.conversation, ConversationId::Direct(ALICE));
    assert_eq!(message.sender, ME);
    assert_eq!(message.direction, Direction::Outgoing(DeliveryStatus::Sent));
}

#[test]
fn kept_contents_yield_only_what_the_store_keeps() {
    // Receipts we sent, and typing, mean nothing to the store.
    let receipt = ReceiptMessage {
        r#type: Some(receipt_message::Type::Read.into()),
        timestamp: vec![10],
    };
    let mut ours = from(ME, receipt.clone());
    ours.metadata.destination = signal_id(ALICE);
    assert_eq!(stored(&ours, ME), vec![]);
    let typing = from(ALICE, typing(TypingState::Started, 5, None));
    assert_eq!(stored(&typing, ME), vec![]);
    // Receipts others sent still update our messages.
    assert_eq!(stored(&from(ALICE, receipt), ME).len(), 1);
    // Incoming text maps as live.
    let content = from(ALICE, text_from_alice("hi", 1000));
    assert_eq!(stored(&content, ME), events(&content, ME));
}

#[test]
fn reads_here_are_synced_to_our_other_devices() {
    let read = |sender, at| MessageRef {
        sender,
        sent_at: Timestamp::from_millis(at),
    };
    let pni = ServiceId::Pni(Uuid::from_u128(3));
    let sync = read_sync(&[read(ALICE, 10), read(ME, 20), read(pni, 30)], ME);
    assert_eq!(
        sync.read,
        vec![sync_message::Read {
            sender_aci: Some(ALICE.to_string()),
            timestamp: Some(10),
            ..sync_message::Read::default()
        }]
    );
    assert_eq!(sync.content, None);
    // What the phone would make of it: the same messages, read elsewhere.
    assert_eq!(
        only(events(&from(ME, sync), ME)),
        BackendEvent::ReadElsewhere(vec![read(ALICE, 10)])
    );
}
