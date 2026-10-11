#![expect(
    clippy::disallowed_methods,
    reason = "#[tokio::test] expands to block_on and tests check file modes synchronously; tests may block"
)]

use std::os::unix::fs::PermissionsExt as _;
use std::time::Duration;

use uuid::Uuid;

use super::*;
use crate::backend::{Backend as _, BackendEvent};
use crate::domain::{
    AttachmentId, AttachmentRef, ConnectionState, DeliveryStatus, Direction, GroupId, Mention,
    Quote, Reaction, ServiceId,
};
use crate::mock::{MockBackend, MockConfig};

/// The engine a test runs on: each test in [`engines!`] runs on both.
#[derive(Clone, Copy)]
enum Kind {
    Sqlite,
    Memory,
}

impl Kind {
    async fn open(self) -> Store {
        match self {
            Self::Sqlite => Store::open_in_memory().await.unwrap(),
            Self::Memory => Store::memory(),
        }
    }
}

/// For each `async fn name(kind: Kind)`, the tests `name::sqlite` and
/// `name::memory`: the engines must agree on everything.
macro_rules! engines {
    ($($name:ident),* $(,)?) => {$(
        mod $name {
            use super::Kind;

            #[tokio::test]
            async fn sqlite() {
                super::$name(Kind::Sqlite).await;
            }

            #[tokio::test]
            async fn memory() {
                super::$name(Kind::Memory).await;
            }
        }
    )*};
}

engines! {
    message_round_trips_with_quote_reactions_and_attachments,
    attachment_status_updates_and_duplicate_replay_heals_pre_m9_pointer,
    duplicates_are_ignored_and_only_incoming_counts_as_unread,
    reads_on_the_phone_clear_what_came_before,
    receipts_never_regress,
    per_recipient_receipts_upsert_and_never_downgrade,
    unsent_messages_fail_on_startup,
    reactions_are_set_replaced_and_removed,
    only_newer_edits_win_and_deletes_drop_content,
    mentions_are_kept_replaced_by_edits_and_dropped_on_delete,
    conversations_are_ordered_and_titled,
    pages_walk_back_without_gaps_or_overlap,
    conversation_and_app_state_round_trip,
    a_mute_without_end_round_trips,
    handles_fail_cleanly_after_close,
    a_new_database_takes_either_profile_and_keeps_it,
    an_unmarked_database_with_history_is_a_real_account,
    profiles_never_cross,
    nicknames_are_kept_apart_from_contact_updates,
    search_messages_matches_bodies_and_transcripts,
    messages_around_returns_oldest_first_window,
}

fn aci(n: u128) -> ServiceId {
    ServiceId::Aci(Uuid::from_u128(n))
}

const ME: u128 = 1;
const ALICE: u128 = 2;
const BOB: u128 = 3;
const CAROL: u128 = 4;

fn ts(ms: u64) -> Timestamp {
    Timestamp::from_millis(ms)
}

fn direct(n: u128) -> ConversationId {
    ConversationId::Direct(aci(n))
}

fn group() -> ConversationId {
    ConversationId::Group(GroupId::from_bytes([9; GroupId::LEN]))
}

fn incoming(conversation: ConversationId, sender: u128, sent_at: u64, body: &str) -> Message {
    Message {
        conversation,
        sender: aci(sender),
        sent_at: ts(sent_at),
        direction: Direction::Incoming,
        body: Some(MessageBody::new(body)),
        mentions: vec![],
        quote: None,
        attachments: vec![],
        reactions: vec![],
        edited_at: None,
        deleted: false,
    }
}

fn outgoing(conversation: ConversationId, sent_at: u64, status: DeliveryStatus) -> Message {
    Message {
        direction: Direction::Outgoing(status),
        ..incoming(conversation, ME, sent_at, "from me")
    }
}

async fn store_with(kind: Kind, messages: &[Message]) -> Store {
    let store = kind.open().await;
    for message in messages {
        store
            .apply(BackendEvent::Message(message.clone()))
            .await
            .unwrap();
    }
    store
}

async fn only_message(store: &Store, conversation: ConversationId) -> Message {
    let mut messages = store.messages(conversation, None, 10).await.unwrap();
    assert_eq!(messages.len(), 1);
    messages.remove(0)
}

async fn summary(store: &Store, id: ConversationId) -> ConversationSummary {
    store
        .conversations()
        .await
        .unwrap()
        .into_iter()
        .find(|c| c.id == id)
        .unwrap()
}

async fn message_round_trips_with_quote_reactions_and_attachments(kind: Kind) {
    let original = incoming(direct(ALICE), ALICE, 100, "first");
    let mut reply = incoming(direct(ALICE), ALICE, 200, "reply");
    reply.quote = Some(Quote {
        target: original.reference(),
        preview: Some(MessageBody::new("first")),
    });
    reply.reactions = vec![
        Reaction {
            sender: aci(ME),
            emoji: "👍".into(),
        },
        Reaction {
            sender: aci(ALICE),
            emoji: "😂".into(),
        },
    ];
    reply.attachments = vec![AttachmentRef {
        file_name: Some("cat.jpg".into()),
        size: Some(12_345),
        width: Some(800),
        height: Some(600),
        ..AttachmentRef::remote("att-1", "image/jpeg")
    }];
    reply.edited_at = Some(ts(250));

    let store = store_with(kind, &[original.clone(), reply.clone()]).await;
    let page = store.messages(direct(ALICE), None, 10).await.unwrap();
    assert_eq!(page, vec![original, reply]);
}

async fn attachment_status_updates_and_duplicate_replay_heals_pre_m9_pointer(kind: Kind) {
    use crate::domain::{AttachmentStatus, TranscriptSource};

    let mut message = incoming(direct(ALICE), ALICE, 100, "photo");
    message.attachments = vec![AttachmentRef {
        file_name: Some("photo.png".into()),
        size: Some(4_096),
        ..AttachmentRef::remote("cdn:12345", "image/png")
    }];
    let store = store_with(kind, std::slice::from_ref(&message)).await;

    // Duplicate replay with full pointer + dimensions + voice_note heals metadata.
    let mut replayed = message.clone();
    replayed.attachments[0].id = AttachmentId("p1:deadbeef".into());
    replayed.attachments[0].width = Some(320);
    replayed.attachments[0].height = Some(240);
    replayed.attachments[0].voice_note = true;
    store
        .apply(BackendEvent::Message(replayed.clone()))
        .await
        .unwrap();
    let stored = only_message(&store, direct(ALICE)).await;
    assert_eq!(stored.attachments, replayed.attachments);

    // Downloading updates status, dimensions, duration, waveform, and transcript;
    // a later duplicate replay keeps them.
    let mut downloaded = replayed.attachments[0].clone();
    downloaded.width = Some(640);
    downloaded.height = Some(480);
    downloaded.duration_secs = Some(14);
    downloaded.waveform = Some(vec![0, 2, 4, 7, 5, 3, 1, 0, 1, 3, 6, 7, 4, 2, 1, 0]);
    downloaded.transcript = Some("Voice note check".into());
    downloaded.transcript_source = Some(TranscriptSource::Local);
    downloaded.status = AttachmentStatus::Downloaded(PathBuf::from("/tmp/photo.png"));
    store
        .set_attachment(message.reference(), 0, downloaded.clone())
        .await
        .unwrap();
    store.apply(BackendEvent::Message(replayed)).await.unwrap();
    let stored = only_message(&store, direct(ALICE)).await;
    assert_eq!(stored.attachments, vec![downloaded]);
}

async fn duplicates_are_ignored_and_only_incoming_counts_as_unread(kind: Kind) {
    let message = incoming(direct(ALICE), ALICE, 100, "hi");
    let store = store_with(
        kind,
        &[
            message.clone(),
            message,
            outgoing(direct(ALICE), 200, DeliveryStatus::Sent),
        ],
    )
    .await;
    assert_eq!(
        store.stats().await.unwrap(),
        StoreStats {
            conversations: 1,
            messages: 2
        }
    );
    let alice = summary(&store, direct(ALICE)).await;
    assert_eq!(alice.unread, 1);
    assert_eq!(alice.last_activity, Some(ts(200)));

    store.mark_read(direct(ALICE)).await.unwrap();
    assert_eq!(summary(&store, direct(ALICE)).await.unread, 0);
}

async fn reads_on_the_phone_clear_what_came_before(kind: Kind) {
    let alice = |at| incoming(direct(ALICE), ALICE, at, "hi");
    let in_group = |sender, at| incoming(group(), sender, at, "hey");
    let store = store_with(
        kind,
        &[
            alice(100),
            alice(200),
            alice(300),
            outgoing(direct(ALICE), 350, DeliveryStatus::Sent),
            alice(400),
            in_group(BOB, 100),
            in_group(CAROL, 200),
        ],
    )
    .await;
    assert_eq!(summary(&store, direct(ALICE)).await.unread, 4);
    assert_eq!(summary(&store, group()).await.unread, 2);

    // Read up to 300 in Alice's chat (200 too, which changes nothing more),
    // all of the group, and something faro never had.
    let read = |sender, at| MessageRef {
        sender: aci(sender),
        sent_at: ts(at),
    };
    let cleared = store
        .read_elsewhere(vec![
            read(ALICE, 200),
            read(ALICE, 300),
            read(CAROL, 200),
            read(BOB, 999),
        ])
        .await
        .unwrap();
    assert_eq!(cleared, vec![(direct(ALICE), 3), (group(), 2)]);
    assert_eq!(summary(&store, direct(ALICE)).await.unread, 1);
    assert_eq!(summary(&store, group()).await.unread, 0);

    // Already read here: nothing more to clear, and counts never go up.
    store.mark_read(direct(ALICE)).await.unwrap();
    let cleared = store.read_elsewhere(vec![read(ALICE, 100)]).await.unwrap();
    assert_eq!(cleared, vec![]);
    assert_eq!(summary(&store, direct(ALICE)).await.unread, 0);

    // Applied as an event (catching up), it does the same.
    store
        .apply(BackendEvent::Message(alice(500)))
        .await
        .unwrap();
    store
        .apply(BackendEvent::ReadElsewhere(vec![read(ALICE, 500)]))
        .await
        .unwrap();
    assert_eq!(summary(&store, direct(ALICE)).await.unread, 0);
}

async fn receipts_never_regress(kind: Kind) {
    let sent = outgoing(direct(ALICE), 100, DeliveryStatus::Sending);
    let store = store_with(kind, std::slice::from_ref(&sent)).await;
    let update = |status| BackendEvent::DeliveryUpdate {
        message: sent.reference(),
        status,
        by: None,
        updated_at: None,
    };
    store.apply(update(DeliveryStatus::Read)).await.unwrap();
    store
        .apply(update(DeliveryStatus::Delivered))
        .await
        .unwrap();
    store.apply(update(DeliveryStatus::Sent)).await.unwrap();
    let stored = only_message(&store, direct(ALICE)).await;
    assert_eq!(stored.direction, Direction::Outgoing(DeliveryStatus::Read));

    // A receipt for a message we never saw is dropped, not an error.
    let unknown = BackendEvent::DeliveryUpdate {
        message: MessageRef {
            sender: aci(ME),
            sent_at: ts(999),
        },
        status: DeliveryStatus::Read,
        by: None,
        updated_at: None,
    };
    store.apply(unknown).await.unwrap();
}

async fn per_recipient_receipts_upsert_and_never_downgrade(kind: Kind) {
    let sent = outgoing(group(), 100, DeliveryStatus::Sent);
    let target = sent.reference();
    let store = store_with(kind, std::slice::from_ref(&sent)).await;

    let receipt = |who, status, at| BackendEvent::DeliveryUpdate {
        message: target,
        status,
        by: Some(aci(who)),
        updated_at: Some(ts(at)),
    };
    store
        .apply(receipt(ALICE, DeliveryStatus::Delivered, 150))
        .await
        .unwrap();
    store
        .apply(receipt(BOB, DeliveryStatus::Read, 200))
        .await
        .unwrap();
    // Late delivered receipt for Bob must not downgrade 'read' or overwrite timestamp.
    store
        .apply(receipt(BOB, DeliveryStatus::Delivered, 180))
        .await
        .unwrap();
    // Alice reads later -> upgrades to Read and updates timestamp.
    store
        .apply(receipt(ALICE, DeliveryStatus::Read, 250))
        .await
        .unwrap();

    assert_eq!(
        store.message_receipts(target).await.unwrap(),
        vec![
            RecipientReceipt {
                recipient: aci(BOB),
                status: DeliveryStatus::Read,
                updated_at: ts(200),
            },
            RecipientReceipt {
                recipient: aci(ALICE),
                status: DeliveryStatus::Read,
                updated_at: ts(250),
            },
        ]
    );
}

async fn unsent_messages_fail_on_startup(kind: Kind) {
    let store = store_with(
        kind,
        &[
            outgoing(direct(ALICE), 100, DeliveryStatus::Sending),
            outgoing(direct(ALICE), 200, DeliveryStatus::Delivered),
        ],
    )
    .await;
    assert_eq!(store.fail_unsent().await.unwrap(), 1);
    let statuses: Vec<_> = store
        .messages(direct(ALICE), None, 10)
        .await
        .unwrap()
        .into_iter()
        .map(|m| m.direction)
        .collect();
    assert_eq!(
        statuses,
        vec![
            Direction::Outgoing(DeliveryStatus::Failed),
            Direction::Outgoing(DeliveryStatus::Delivered)
        ]
    );
    assert_eq!(store.fail_unsent().await.unwrap(), 0);
}

async fn reactions_are_set_replaced_and_removed(kind: Kind) {
    let message = incoming(group(), ALICE, 100, "hi all");
    let store = store_with(kind, std::slice::from_ref(&message)).await;
    let react = |sender, emoji: Option<&str>| BackendEvent::Reaction {
        conversation: group(),
        target: message.reference(),
        sender: aci(sender),
        emoji: emoji.map(str::to_owned),
    };
    store.apply(react(BOB, Some("❤️"))).await.unwrap();
    store.apply(react(CAROL, Some("👍"))).await.unwrap();
    store.apply(react(BOB, Some("😮"))).await.unwrap();
    store.apply(react(CAROL, None)).await.unwrap();
    let stored = only_message(&store, group()).await;
    assert_eq!(
        stored.reactions,
        vec![Reaction {
            sender: aci(BOB),
            emoji: "😮".into()
        }]
    );
}

async fn only_newer_edits_win_and_deletes_drop_content(kind: Kind) {
    let message = incoming(direct(ALICE), ALICE, 100, "tpyo");
    let store = store_with(kind, std::slice::from_ref(&message)).await;
    let edit = |body: &str, at| BackendEvent::Edited {
        conversation: direct(ALICE),
        target: message.reference(),
        body: MessageBody::new(body),
        mentions: vec![],
        edited_at: ts(at),
    };
    store.apply(edit("typo fixed", 300)).await.unwrap();
    store.apply(edit("typo", 200)).await.unwrap();
    let stored = only_message(&store, direct(ALICE)).await;
    assert_eq!(stored.body, Some(MessageBody::new("typo fixed")));
    assert_eq!(stored.edited_at, Some(ts(300)));

    store
        .apply(BackendEvent::Reaction {
            conversation: direct(ALICE),
            target: message.reference(),
            sender: aci(ME),
            emoji: Some("👍".into()),
        })
        .await
        .unwrap();
    store
        .apply(BackendEvent::Deleted {
            conversation: direct(ALICE),
            target: message.reference(),
        })
        .await
        .unwrap();
    store.apply(edit("resurrected", 400)).await.unwrap();
    let stored = only_message(&store, direct(ALICE)).await;
    assert!(stored.deleted);
    assert_eq!(stored.body, None);
    assert_eq!(stored.reactions, vec![]);
}

async fn mentions_are_kept_replaced_by_edits_and_dropped_on_delete(kind: Kind) {
    let mention = |who, start| Mention {
        who: aci(who),
        start,
        len: 3,
    };
    let mut message = incoming(group(), ALICE, 100, "\u{FFFC} and \u{FFFC}");
    message.mentions = vec![mention(ME, 0), mention(BOB, 8)];
    let store = store_with(kind, std::slice::from_ref(&message)).await;
    let stored = only_message(&store, group()).await;
    assert_eq!(stored.mentions, message.mentions);

    let edit = |body: &str, mentions, at| BackendEvent::Edited {
        conversation: group(),
        target: message.reference(),
        body: MessageBody::new(body),
        mentions,
        edited_at: ts(at),
    };
    store
        .apply(edit("hi \u{FFFC}", vec![mention(BOB, 3)], 300))
        .await
        .unwrap();
    // Older than the applied edit: changes nothing, mentions included.
    store.apply(edit("old", vec![], 200)).await.unwrap();
    let stored = only_message(&store, group()).await;
    assert_eq!(stored.mentions, vec![mention(BOB, 3)]);

    store
        .apply(BackendEvent::Deleted {
            conversation: group(),
            target: message.reference(),
        })
        .await
        .unwrap();
    assert_eq!(only_message(&store, group()).await.mentions, vec![]);
}

async fn conversations_are_ordered_and_titled(kind: Kind) {
    let store = kind.open().await;
    let contact = |n, name: Option<&str>, profile: Option<&str>| Contact {
        service_id: aci(n),
        name: name.map(str::to_owned),
        profile_name: profile.map(str::to_owned),
        phone: Some(PhoneNumber::new(format!("+4470000000{n}"))),
    };
    store
        .upsert_contacts(vec![
            contact(ALICE, Some("Alice A."), Some("alice")),
            contact(BOB, None, Some("bob")),
            contact(CAROL, None, None),
        ])
        .await
        .unwrap();
    store
        .upsert_groups(vec![Group {
            id: GroupId::from_bytes([9; GroupId::LEN]),
            title: "Climbing".into(),
            members: vec![aci(ME), aci(ALICE), aci(BOB)],
        }])
        .await
        .unwrap();
    for message in [
        incoming(direct(ALICE), ALICE, 100, "a"),
        incoming(group(), BOB, 200, "b"),
    ] {
        store.apply(BackendEvent::Message(message)).await.unwrap();
    }
    let pinned = ConversationState {
        pinned: Some(0),
        ..ConversationState::default()
    };
    store
        .set_conversation_state(direct(BOB), pinned)
        .await
        .unwrap();

    let list = store.conversations().await.unwrap();
    let order: Vec<_> = list.iter().map(|c| c.id).collect();
    assert_eq!(
        order,
        vec![direct(BOB), group(), direct(ALICE), direct(CAROL)]
    );
    let titles: Vec<_> = list.iter().map(|c| c.title.as_deref()).collect();
    assert_eq!(
        titles,
        vec![Some("bob"), Some("Climbing"), Some("Alice A."), None]
    );
    assert_eq!(
        list.last()
            .and_then(|c| c.phone.as_ref())
            .map(PhoneNumber::expose),
        Some("+44700000004")
    );
    assert_eq!(list.first().map(|c| c.state.pinned), Some(Some(0)));
}

async fn pages_walk_back_without_gaps_or_overlap(kind: Kind) {
    let messages: Vec<_> = (1..=25)
        .map(|i| incoming(direct(ALICE), ALICE, i, "m"))
        .collect();
    let store = store_with(kind, &messages).await;
    let mut seen = Vec::new();
    let mut before = None;
    let mut sizes = Vec::new();
    loop {
        let page = store.messages(direct(ALICE), before, 10).await.unwrap();
        if page.is_empty() {
            break;
        }
        sizes.push(page.len());
        before = page.first().map(Message::reference);
        seen.splice(0..0, page.iter().map(|m| m.sent_at.as_millis()));
    }
    assert_eq!(sizes, vec![10, 10, 5]);
    assert_eq!(seen, (1..=25).collect::<Vec<_>>());
}

async fn conversation_and_app_state_round_trip(kind: Kind) {
    let store = kind.open().await;
    let state = ConversationState {
        hidden: true,
        archived: true,
        pinned: Some(3),
        muted_until: Some(ts(5_000)),
        draft: Some(MessageBody::new("half a thought")),
        allowed: true,
        notify: NotifyLevel::Mentions,
        remote_transcription: RemoteTranscription::Allowed,
    };
    store
        .set_conversation_state(group(), state.clone())
        .await
        .unwrap();
    assert_eq!(summary(&store, group()).await.state, state);

    assert_eq!(store.app_state("last").await.unwrap(), None);
    store.set_app_state("last", "one").await.unwrap();
    store.set_app_state("last", "two").await.unwrap();
    assert_eq!(
        store.app_state("last").await.unwrap().as_deref(),
        Some("two")
    );
    store.remove_app_state("last").await.unwrap();
    assert_eq!(store.app_state("last").await.unwrap(), None);
    store.remove_app_state("never set").await.unwrap();
}

async fn a_mute_without_end_round_trips(kind: Kind) {
    let store = kind.open().await;
    let state = ConversationState {
        muted_until: Some(ConversationState::MUTED_FOREVER),
        ..ConversationState::default()
    };
    store
        .set_conversation_state(group(), state.clone())
        .await
        .unwrap();
    let stored = summary(&store, group()).await.state;
    assert_eq!(stored, state);
    assert!(stored.muted_at(ts(u64::from(u32::MAX))));
    assert!(!ConversationState::default().muted_at(ts(0)));
    let until_5s = ConversationState {
        muted_until: Some(ts(5_000)),
        ..ConversationState::default()
    };
    assert!(until_5s.muted_at(ts(4_999)));
    assert!(!until_5s.muted_at(ts(5_000)));
}

#[tokio::test]
async fn file_store_is_private_and_reopens() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("faro.db");
    let store = Store::open(&path, Profile::Signal).await.unwrap();
    store.set_app_state("k", "v").await.unwrap();
    store.close().await.unwrap();
    let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, paths::FILE_MODE);

    // Migrations are idempotent and the data survived.
    let store = Store::open(&path, Profile::Signal).await.unwrap();
    assert_eq!(store.app_state("k").await.unwrap().as_deref(), Some("v"));
    store.close().await.unwrap();
}

async fn handles_fail_cleanly_after_close(kind: Kind) {
    let store = kind.open().await;
    let other = store.clone();
    store.close().await.unwrap();
    assert!(matches!(other.stats().await, Err(StoreError::Closed)));
}

fn mismatch(result: Result<()>) -> ProfileMismatch {
    match result {
        Err(StoreError::Profile(mismatch)) => mismatch,
        other => panic!("expected a profile mismatch, got {other:?}"),
    }
}

async fn a_new_database_takes_either_profile_and_keeps_it(kind: Kind) {
    for profile in [Profile::Demo, Profile::Signal] {
        let store = kind.open().await;
        store.claim_profile(profile).await.unwrap();
        assert_eq!(
            store.app_state(PROFILE_KEY).await.unwrap().as_deref(),
            Some(profile.as_str())
        );
        store.claim_profile(profile).await.unwrap();
    }
}

async fn an_unmarked_database_with_history_is_a_real_account(kind: Kind) {
    let history = [incoming(direct(ALICE), ALICE, 100, "hi")];
    let store = store_with(kind, &history).await;
    assert_eq!(
        mismatch(store.claim_profile(Profile::Demo).await),
        ProfileMismatch::UnmarkedHistory
    );
    assert_eq!(
        store.app_state(PROFILE_KEY).await.unwrap(),
        None,
        "unmarked"
    );
    store.claim_profile(Profile::Signal).await.unwrap();
    assert_eq!(
        store.app_state(PROFILE_KEY).await.unwrap().as_deref(),
        Some("signal")
    );

    // Any app state at all counts as history, too.
    let store = kind.open().await;
    store.set_app_state("ui.last_open", "x").await.unwrap();
    assert_eq!(
        mismatch(store.claim_profile(Profile::Demo).await),
        ProfileMismatch::UnmarkedHistory
    );
}

async fn profiles_never_cross(kind: Kind) {
    let demo = kind.open().await;
    demo.claim_profile(Profile::Demo).await.unwrap();
    assert_eq!(
        mismatch(demo.claim_profile(Profile::Signal).await),
        ProfileMismatch::DemoDatabase
    );
    let real = kind.open().await;
    real.claim_profile(Profile::Signal).await.unwrap();
    assert_eq!(
        mismatch(real.claim_profile(Profile::Demo).await),
        ProfileMismatch::SignalDatabase
    );
    let odd = kind.open().await;
    odd.set_app_state(PROFILE_KEY, "staging").await.unwrap();
    for profile in [Profile::Demo, Profile::Signal] {
        assert_eq!(
            mismatch(odd.claim_profile(profile).await),
            ProfileMismatch::Unknown("staging".into())
        );
    }
}

#[tokio::test]
async fn opening_marks_the_profile_and_it_survives_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("faro.db");
    let store = Store::open(&path, Profile::Demo).await.unwrap();
    assert_eq!(
        store.app_state(PROFILE_KEY).await.unwrap().as_deref(),
        Some("demo")
    );
    // Read while it's open (with its `-wal` file), then after closing.
    assert_eq!(Store::profile_of(&path).await.unwrap(), Some(Profile::Demo));
    store.close().await.unwrap();
    assert_eq!(Store::profile_of(&path).await.unwrap(), Some(Profile::Demo));
    assert_eq!(
        mismatch(Store::open(&path, Profile::Signal).await.map(|_| ())),
        ProfileMismatch::DemoDatabase
    );
    let store = Store::open(&path, Profile::Demo).await.unwrap();
    store.close().await.unwrap();
}

fn file_state(path: &std::path::Path) -> (Vec<u8>, u32) {
    let bytes = std::fs::read(path).unwrap();
    let mode = std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
    (bytes, mode)
}

/// Refused before anything touches the file: no permission change, no
/// journal files, no migrations (D87).
#[tokio::test]
async fn a_refused_database_is_left_untouched() {
    use sqlx::{ConnectOptions as _, Connection as _};

    let dir = tempfile::tempdir().unwrap();
    // A real account's database.
    let real = dir.path().join("faro.db");
    let store = Store::open(&real, Profile::Signal).await.unwrap();
    store
        .apply(BackendEvent::Message(incoming(
            direct(ALICE),
            ALICE,
            1,
            "hi",
        )))
        .await
        .unwrap();
    store.close().await.unwrap();
    std::fs::set_permissions(&real, std::fs::Permissions::from_mode(0o644)).unwrap();
    let before = file_state(&real);
    assert_eq!(
        mismatch(Store::open(&real, Profile::Demo).await.map(|_| ())),
        ProfileMismatch::SignalDatabase
    );
    assert_eq!(file_state(&real), before);
    for side in ["faro.db-wal", "faro.db-shm"] {
        assert!(!dir.path().join(side).exists(), "{side}");
    }

    // Something faro never migrated (an older or foreign schema).
    let old = dir.path().join("old.db");
    let mut conn = SqliteConnectOptions::new()
        .filename(&old)
        .create_if_missing(true)
        .connect()
        .await
        .unwrap();
    sqlx::query("CREATE TABLE notes (text TEXT)")
        .execute(&mut conn)
        .await
        .unwrap();
    conn.close().await.unwrap();
    let before = file_state(&old);
    assert_eq!(
        mismatch(Store::open(&old, Profile::Demo).await.map(|_| ())),
        ProfileMismatch::UnmarkedHistory
    );
    assert_eq!(file_state(&old), before, "not migrated");
    assert_eq!(Store::profile_of(&old).await.unwrap(), None);
}

/// M2 acceptance: a restart keeps history (with the mock backend).
#[tokio::test]
async fn restart_keeps_mock_history() {
    const BACKLOG: usize = 30;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("faro.db");
    let store = Store::open(&path, Profile::Demo).await.unwrap();

    let config = MockConfig {
        activity: None,
        backlog: BACKLOG,
        disconnects: false,
        errors: false,
        ..MockConfig::deterministic(7)
    };
    let (mock, mut events) = MockBackend::new(&config);
    let mut received = 0;
    while received < BACKLOG {
        let event = tokio::time::timeout(Duration::from_secs(10), events.recv())
            .await
            .expect("mock went quiet")
            .expect("mock stopped");
        if event == BackendEvent::Connection(ConnectionState::Connected) {
            store
                .upsert_contacts(mock.contacts().await.unwrap())
                .await
                .unwrap();
            store
                .upsert_groups(mock.groups().await.unwrap())
                .await
                .unwrap();
        } else if let BackendEvent::Message(_) = event {
            received += 1;
        }
        store.apply(event).await.unwrap();
    }
    let before = store.stats().await.unwrap();
    let conversations = store.conversations().await.unwrap();
    let busiest = conversations.first().unwrap().id;
    let history = store.messages(busiest, None, 100).await.unwrap();
    store.close().await.unwrap();
    assert_eq!(before.messages, BACKLOG as u64);
    assert_ne!(history, vec![]);

    let store = Store::open(&path, Profile::Demo).await.unwrap();
    assert_eq!(store.stats().await.unwrap(), before);
    assert_eq!(store.conversations().await.unwrap(), conversations);
    assert_eq!(store.messages(busiest, None, 100).await.unwrap(), history);
    store.close().await.unwrap();
}

async fn nicknames_are_kept_apart_from_contact_updates(kind: Kind) {
    let store = kind.open().await;
    assert_eq!(store.nicknames().await.unwrap(), HashMap::new());
    store
        .set_nickname(aci(ALICE), Some("Ali".to_owned()))
        .await
        .unwrap();
    // A later contact update (new profile name) keeps the nickname.
    store
        .upsert_contacts(vec![Contact {
            service_id: aci(ALICE),
            name: None,
            profile_name: Some("Alice".to_owned()),
            phone: None,
        }])
        .await
        .unwrap();
    assert_eq!(
        store.nicknames().await.unwrap(),
        HashMap::from([(aci(ALICE), "Ali".to_owned())])
    );
    store.set_nickname(aci(ALICE), None).await.unwrap();
    assert_eq!(store.nicknames().await.unwrap(), HashMap::new());
}

async fn search_messages_matches_bodies_and_transcripts(kind: Kind) {
    let m1 = incoming(direct(ALICE), ALICE, 100, "Meet at the crag at 9am");
    let m2 = incoming(direct(BOB), BOB, 200, "100% ready_now for the crag");
    let mut voice = incoming(direct(ALICE), ALICE, 300, "");
    voice.body = None;
    voice.attachments = vec![AttachmentRef {
        id: AttachmentId("att-voice".to_owned()),
        content_type: "audio/aac".to_owned(),
        file_name: None,
        size: Some(4096),
        width: None,
        height: None,
        status: crate::domain::AttachmentStatus::Remote,
        voice_note: true,
        duration_secs: Some(5),
        waveform: None,
        transcript: Some("Bring the quickdraws to the crag".to_owned()),
        transcript_source: Some(crate::domain::TranscriptSource::Local),
    }];
    let deleted = incoming(direct(ALICE), ALICE, 400, "crag secret");

    let store = store_with(
        kind,
        &[m1.clone(), m2.clone(), voice.clone(), deleted.clone()],
    )
    .await;
    store
        .set_attachment(voice.reference(), 0, voice.attachments[0].clone())
        .await
        .unwrap();
    store
        .apply(BackendEvent::Deleted {
            conversation: direct(ALICE),
            target: deleted.reference(),
        })
        .await
        .unwrap();

    // Global search for "crag" finds voice, m2, m1 newest first, skipping deleted.
    let hits = store.search_messages(None, "CRAG", 10).await.unwrap();
    assert_eq!(hits.len(), 3);
    assert_eq!(hits[0].message.sent_at, ts(300));
    assert_eq!(
        hits[0].matched_transcript.as_deref(),
        Some("Bring the quickdraws to the crag")
    );
    assert_eq!(hits[1].message.sent_at, ts(200));
    assert_eq!(hits[1].matched_transcript, None);
    assert_eq!(hits[2].message.sent_at, ts(100));

    // Scoped search + multi-token + LIKE wildcard escaping (`100%` and `ready_now`).
    let scoped = store
        .search_messages(Some(direct(BOB)), "100% ready_now", 10)
        .await
        .unwrap();
    assert_eq!(scoped.len(), 1);
    assert_eq!(scoped[0].conversation, direct(BOB));
    assert_eq!(scoped[0].message.sent_at, ts(200));

    assert_eq!(
        store.search_messages(None, "   ", 10).await.unwrap(),
        vec![]
    );
}

async fn messages_around_returns_oldest_first_window(kind: Kind) {
    let msgs: Vec<Message> = (1..=7)
        .map(|i| incoming(direct(ALICE), ALICE, i * 100, &format!("m{i}")))
        .collect();
    let store = store_with(kind, &msgs).await;

    let window = store
        .messages_around(direct(ALICE), ts(400), 2)
        .await
        .unwrap();
    let times: Vec<u64> = window.iter().map(|m| m.sent_at.as_millis()).collect();
    assert_eq!(times, vec![300, 400, 500, 600]);
}
