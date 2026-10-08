#![expect(
    clippy::disallowed_methods,
    reason = "#[tokio::test] expands to block_on; tests may block"
)]

use tokio::sync::mpsc;
use uuid::Uuid;

use super::*;
use crate::domain::{ConnectionState, DeliveryStatus, Direction, Message, MessageBody, Timestamp};
use crate::mock::{MockBackend, MockConfig};

fn person(n: u128) -> ServiceId {
    ServiceId::Aci(Uuid::from_u128(n))
}

fn group(n: u8) -> GroupId {
    GroupId::from_bytes([n; GroupId::LEN])
}

const ALLOWED: u128 = 1;
const FRIEND: u128 = 2;
const STRANGER: u128 = 3;

fn list() -> Allowlist {
    Allowlist {
        people: [person(ALLOWED)].into(),
        groups: [group(7)].into(),
    }
}

fn shared() -> Shared {
    Shared {
        list: list(),
        members: Mutex::new(HashMap::new()),
    }
}

fn message(conversation: ConversationId, sender: ServiceId) -> BackendEvent {
    BackendEvent::Message(Message {
        conversation,
        sender,
        sent_at: Timestamp::from_millis(1),
        direction: Direction::Incoming,
        body: Some(MessageBody::new("hi")),
        quote: None,
        attachments: Vec::new(),
        reactions: Vec::new(),
        edited_at: None,
        deleted: false,
    })
}

fn contact(who: ServiceId) -> BackendEvent {
    BackendEvent::ContactUpdated(Contact {
        service_id: who,
        name: None,
        profile_name: Some("someone".to_owned()),
        phone: None,
    })
}

fn group_update(id: GroupId, members: Vec<ServiceId>) -> BackendEvent {
    BackendEvent::GroupUpdated(Group {
        id,
        title: "a group".to_owned(),
        members,
    })
}

#[test]
fn the_file_lists_people_and_groups() {
    let text = format!(
        "# comment\npeople = [\"{}\"]\ngroups = [\"{}\"]\n",
        person(ALLOWED),
        group(7)
    );
    assert_eq!(Allowlist::parse(&text), Ok(list()));
    assert_eq!(Allowlist::parse(""), Ok(Allowlist::default()));
}

#[test]
fn anything_unexpected_in_the_file_is_an_error() {
    for bad in [
        "peeple = []",
        "people = \"not a list\"",
        "people = [1]",
        "people = [\"alice.123\"]",
        "groups = [\"abcd\"]",
        "people = [",
    ] {
        assert!(Allowlist::parse(bad).is_err(), "{bad} was accepted");
    }
}

#[test]
fn only_events_about_allowed_conversations_get_through() {
    let shared = shared();
    let direct = |n| ConversationId::Direct(person(n));
    let allowed_group = ConversationId::Group(group(7));
    let other_group = ConversationId::Group(group(8));

    assert!(shared.admits(&BackendEvent::Connection(ConnectionState::Connected)));
    assert!(shared.admits(&BackendEvent::DeliveryUpdate {
        message: MessageRef {
            sender: person(ALLOWED),
            sent_at: Timestamp::from_millis(1),
        },
        status: DeliveryStatus::Delivered,
    }));
    assert!(shared.admits(&message(direct(ALLOWED), person(ALLOWED))));
    assert!(!shared.admits(&message(direct(STRANGER), person(STRANGER))));
    assert!(shared.admits(&message(allowed_group, person(FRIEND))));
    assert!(!shared.admits(&message(other_group, person(ALLOWED))));
    assert!(!shared.admits(&BackendEvent::Typing {
        conversation: direct(STRANGER),
        sender: person(STRANGER),
        state: TypingState::Started,
    }));
    assert!(!shared.admits(&BackendEvent::Deleted {
        conversation: other_group,
        target: MessageRef {
            sender: person(STRANGER),
            sent_at: Timestamp::from_millis(1),
        },
    }));

    // People: listed ones, and members of allowed groups once known.
    assert!(shared.admits(&contact(person(ALLOWED))));
    assert!(!shared.admits(&contact(person(FRIEND))));
    assert!(shared.admits(&group_update(group(7), vec![person(FRIEND)])));
    assert!(shared.admits(&contact(person(FRIEND))));
    assert!(!shared.admits(&contact(person(STRANGER))));
    // Other groups aren't let through, and their members stay unknown.
    assert!(!shared.admits(&group_update(group(8), vec![person(STRANGER)])));
    assert!(!shared.admits(&contact(person(STRANGER))));
}

#[tokio::test(start_paused = true)]
async fn the_filter_passes_only_what_is_allowed() {
    let (backend, _) = MockBackend::new(&MockConfig::deterministic(1));
    let (tx, events) = mpsc::channel(16);
    let (_guarded, mut out) = guard(backend, events, list());
    let direct = |n| ConversationId::Direct(person(n));
    for event in [
        message(direct(STRANGER), person(STRANGER)),
        message(direct(ALLOWED), person(ALLOWED)),
        contact(person(STRANGER)),
        BackendEvent::Connection(ConnectionState::Connected),
    ] {
        tx.send(event).await.unwrap();
    }
    drop(tx);
    let mut passed = Vec::new();
    while let Some(event) = out.recv().await {
        passed.push(event);
    }
    assert_eq!(
        passed,
        vec![
            message(direct(ALLOWED), person(ALLOWED)),
            BackendEvent::Connection(ConnectionState::Connected),
        ]
    );
}

#[tokio::test(start_paused = true)]
async fn actions_towards_others_are_refused() {
    let (backend, events) = MockBackend::new(&MockConfig::deterministic(1));
    let (guarded, _events) = guard(backend, events, list());
    let stranger = ConversationId::Direct(person(STRANGER));
    let other_group = ConversationId::Group(group(8));
    let target = MessageRef {
        sender: person(STRANGER),
        sent_at: Timestamp::from_millis(1),
    };
    let msg = OutgoingMessage {
        sent_at: Timestamp::from_millis(2),
        body: MessageBody::new("hello"),
        quote: None,
    };
    let refused = |r: Result<()>| matches!(r, Err(BackendError::NotAllowed));
    assert!(refused(guarded.send(stranger, msg.clone()).await));
    assert!(refused(guarded.send(other_group, msg.clone()).await));
    assert!(refused(
        guarded.react(stranger, target, Some("👍".into())).await
    ));
    assert!(refused(
        guarded.send_typing(other_group, TypingState::Started).await
    ));
    assert!(refused(guarded.delete_for_everyone(stranger, target).await));
    // Receipts to strangers are dropped (nothing to send: not an error).
    assert_eq!(guarded.mark_read(&[target]).await, Ok(()));
    // Allowed conversations reach the backend.
    let allowed = ConversationId::Direct(person(ALLOWED));
    assert!(!refused(guarded.send(allowed, msg).await));
}

#[tokio::test(start_paused = true)]
async fn the_directory_keeps_only_allowed_people_and_groups() {
    let config = MockConfig::deterministic(1);
    let (reference, _) = MockBackend::new(&config);
    let everyone = reference.contacts().await.unwrap();
    let groups = reference.groups().await.unwrap();
    let chosen = groups.first().cloned().unwrap();
    let outsider = everyone
        .iter()
        .map(|c| c.service_id)
        .find(|id| !chosen.members.contains(id))
        .unwrap();
    let friend = chosen.members.first().copied().unwrap();

    let (backend, events) = MockBackend::new(&config);
    let list = Allowlist {
        people: BTreeSet::new(),
        groups: [chosen.id].into(),
    };
    let (guarded, _events) = guard(backend, events, list);
    let seen = guarded.groups().await.unwrap();
    assert_eq!(seen, vec![chosen.clone()]);
    let contacts: Vec<ServiceId> = guarded
        .contacts()
        .await
        .unwrap()
        .into_iter()
        .map(|c| c.service_id)
        .collect();
    assert!(contacts.contains(&friend), "group members keep their names");
    assert!(!contacts.contains(&outsider));
    assert!(contacts.iter().all(|id| chosen.members.contains(id)));
}
