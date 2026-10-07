//! Test fixtures: a small cast and a conversation with every kind of message.

use faro_core::backend::BackendEvent;
use faro_core::domain::{
    AttachmentId, AttachmentRef, Contact, ConversationId, DeliveryStatus, Direction, Group,
    GroupId, Message, MessageBody, PhoneNumber, Quote, Reaction, ServiceId, Timestamp,
};
use faro_core::store::{ConversationState, ConversationSummary};
use uuid::Uuid;

use crate::app::event::AppEvent;
use crate::app::state::State;
use crate::app::update::update;

pub const ME: ServiceId = ServiceId::Aci(Uuid::from_u128(1));
pub const ALICE: ServiceId = ServiceId::Aci(Uuid::from_u128(2));
pub const BOB: ServiceId = ServiceId::Aci(Uuid::from_u128(3));
pub const CAROL: ServiceId = ServiceId::Aci(Uuid::from_u128(4));
pub const CLIMBING: GroupId = GroupId::from_bytes([7; GroupId::LEN]);

/// 2026-10-07 18:00:00 UTC: "now" in rendering tests.
pub const NOW_MS: u64 = 1_791_396_000_000;
const MINUTE: u64 = 60_000;
const DAY: u64 = 24 * 60 * MINUTE;

pub fn alice() -> ConversationId {
    ConversationId::Direct(ALICE)
}

pub fn bob() -> ConversationId {
    ConversationId::Direct(BOB)
}

pub fn climbing() -> ConversationId {
    ConversationId::Group(CLIMBING)
}

pub fn contacts() -> Vec<Contact> {
    let contact = |id, name: &str, phone: &str| Contact {
        service_id: id,
        name: Some(name.to_owned()),
        profile_name: None,
        phone: Some(PhoneNumber::new(phone)),
    };
    vec![
        contact(ALICE, "Alice 🌸", "+44 7700 900001"),
        contact(BOB, "Bob", "+44 7700 900002"),
        contact(CAROL, "Carol Bartholomew-Smythe", "+44 7700 900003"),
    ]
}

pub fn groups() -> Vec<Group> {
    vec![Group {
        id: CLIMBING,
        title: "Climbing 🧗".to_owned(),
        members: vec![ME, ALICE, BOB, CAROL],
    }]
}

pub fn summary(
    id: ConversationId,
    title: &str,
    minutes_ago: u64,
    unread: u32,
) -> ConversationSummary {
    ConversationSummary {
        id,
        title: Some(title.to_owned()),
        phone: None,
        last_activity: Some(Timestamp::from_millis(NOW_MS - minutes_ago * MINUTE)),
        unread,
        state: ConversationState::default(),
    }
}

/// Climbing (2 unread), Alice, Bob (pinned, read).
pub fn summaries() -> Vec<ConversationSummary> {
    let mut bob = summary(bob(), "Bob", 600, 0);
    bob.state.pinned = Some(0);
    vec![
        bob,
        summary(climbing(), "Climbing 🧗", 5, 2),
        summary(alice(), "Alice 🌸", 30, 1),
    ]
}

pub fn text(conversation: ConversationId, sender: ServiceId, at: u64, body: &str) -> Message {
    Message {
        conversation,
        sender,
        sent_at: Timestamp::from_millis(at),
        direction: if sender == ME {
            Direction::Outgoing(DeliveryStatus::Read)
        } else {
            Direction::Incoming
        },
        body: Some(MessageBody::new(body)),
        quote: None,
        attachments: vec![],
        reactions: vec![],
        edited_at: None,
        deleted: false,
    }
}

/// The Climbing group's history: yesterday and today, replies, reactions, an
/// edit, a delete, an attachment, emoji, a long message, sent and read messages.
pub fn climbing_history() -> Vec<Message> {
    let g = climbing();
    let t = |minutes_ago: u64| NOW_MS - minutes_ago * MINUTE;
    let plan = text(
        g,
        ALICE,
        t(DAY / MINUTE + 90),
        "Anyone up for the wall on Thursday?",
    );
    let mut yes = text(
        g,
        ME,
        t(DAY / MINUTE + 80),
        "Yes! 7pm at the usual place 👍",
    );
    yes.reactions = vec![
        Reaction {
            sender: ALICE,
            emoji: "👍".into(),
        },
        Reaction {
            sender: BOB,
            emoji: "👍".into(),
        },
        Reaction {
            sender: CAROL,
            emoji: "❤️".into(),
        },
    ];
    let mut reply = text(g, BOB, t(40), "Count me in, I'll bring the spare harness");
    reply.quote = Some(Quote {
        target: plan.reference(),
        preview: Some(MessageBody::new("Anyone up for the wall on Thursday?")),
    });
    let mut edited = text(g, BOB, t(38), "and chalk");
    edited.edited_at = Some(Timestamp::from_millis(t(37)));
    let mut deleted = text(g, CAROL, t(20), "");
    deleted.body = None;
    deleted.deleted = true;
    let mut photo = text(g, CAROL, t(10), "Route map from last time");
    photo.attachments = vec![AttachmentRef {
        id: AttachmentId("a1".into()),
        content_type: "image/jpeg".into(),
        file_name: Some("route-map.jpg".into()),
        size: Some(2_400_000),
    }];
    let long = text(
        g,
        CAROL,
        t(9),
        "Also: the new overhang section opened last week and apparently the grades are \
         soft, so bring your project shoes 🧗‍♀️🧗‍♂️ and maybe some tape for your fingers.",
    );
    let mut sent = text(g, ME, t(6), "On my way");
    sent.direction = Direction::Outgoing(DeliveryStatus::Sent);
    let late = text(g, ALICE, t(5), "Running 10 min late 🙈");
    vec![plan, yes, reply, edited, deleted, photo, long, sent, late]
}

/// A state as after startup: conversations loaded, Climbing open with its
/// history (2 unread when opened).
pub fn loaded_state() -> State {
    let mut state = State::new(ME, contacts(), groups());
    update(
        &mut state,
        AppEvent::Loaded {
            conversations: summaries(),
            last_open: Some(climbing()),
            allowlist: false,
            layout: Some(crate::app::state::MessageLayout::Irc),
        },
    );
    update(
        &mut state,
        AppEvent::MessagesLoaded {
            conversation: climbing(),
            messages: climbing_history(),
        },
    );
    update(
        &mut state,
        AppEvent::Backend(BackendEvent::Connection(
            faro_core::domain::ConnectionState::Connected,
        )),
    );
    state
}
