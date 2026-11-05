//! Conversations' pictures (D160): listed once, fetched a couple at a time,
//! only with `avatars = "pictures"`, kept, and retried after failures only
//! when the connection comes back.

use std::path::PathBuf;

use faro_core::backend::{BackendError, BackendEvent, Capabilities, Capability};
use faro_core::config::Avatars;
use faro_core::domain::{ConnectionState, ConversationId};

use super::*;
use crate::app::event::Command;
use crate::fixtures::{alice, bob, climbing, loaded_state};

fn fetches(effects: &[Effect]) -> Vec<ConversationId> {
    effects
        .iter()
        .filter_map(|effect| {
            if let Effect::Backend(Command::FetchPicture(id)) = effect {
                Some(*id)
            } else {
                None
            }
        })
        .collect()
}

/// kitty reachable, previews on, `avatars = "pictures"`.
fn with_pictures() -> State {
    let mut state = loaded_state();
    state.capabilities = Capabilities::NONE.with(Capability::Pictures);
    state.config.conversations.avatars = Avatars::Pictures;
    state.config.conversations.previews = true;
    state.config.media.inline_images = faro_core::config::InlineImages::Auto;
    state.config.ui.colors = faro_core::config::ColorChoice::Truecolor;
    state.env.graphics = crate::app::state::GraphicsSupport::Direct;
    state.env.image_protocol = Some(faro_core::config::InlineImages::Kitty);
    state
}

/// `auto` (D162): fetched only while the terminal can draw them, e.g. not
/// over mosh; once kitty is reachable again they are.
#[test]
fn auto_fetches_only_where_pictures_can_be_drawn() {
    let mut state = with_pictures();
    state.config.conversations.avatars = Avatars::Auto;
    state.env.graphics = crate::app::state::GraphicsSupport::HalfblocksOnly;
    let effects = update(&mut state, AppEvent::PicturesListed(vec![]));
    assert_eq!(fetches(&effects), vec![], "mosh in the way");
    state.config.conversations.previews = false;
    state.env.graphics = crate::app::state::GraphicsSupport::Direct;
    assert_eq!(
        fetches(&update(&mut state, AppEvent::Tick)),
        vec![],
        "one-row entries"
    );
    state.config.conversations.previews = true;
    assert_eq!(
        fetches(&update(&mut state, AppEvent::Tick)).len(),
        2,
        "kitty again"
    );
}

#[test]
fn the_folder_is_read_once_conversations_are_loaded() {
    let mut state = State::new(crate::fixtures::ME, vec![], vec![]);
    let effects = update(
        &mut state,
        AppEvent::Loaded {
            conversations: vec![],
            last_open: None,
            recent_reactions: vec![],
        },
    );
    assert!(effects.contains(&Effect::ListPictures));
}

#[test]
fn nothing_is_fetched_unless_pictures_are_chosen() {
    let mut state = loaded_state();
    state.capabilities = Capabilities::NONE.with(Capability::Pictures);
    let effects = update(&mut state, AppEvent::PicturesListed(vec![]));
    assert_eq!(fetches(&effects), vec![], "avatars off (D155)");

    state.config.conversations.avatars = Avatars::Initials;
    assert_eq!(fetches(&update(&mut state, AppEvent::Tick)), vec![]);

    let mut unsupported = with_pictures();
    unsupported.capabilities = Capabilities::NONE;
    let effects = update(&mut unsupported, AppEvent::PicturesListed(vec![]));
    assert_eq!(fetches(&effects), vec![], "the backend can't");

    let mut unlisted = with_pictures();
    assert_eq!(
        fetches(&update(&mut unlisted, AppEvent::Tick)),
        vec![],
        "not before the folder is read"
    );
}

#[test]
fn missing_pictures_are_fetched_two_at_a_time_in_list_order() {
    let mut state = with_pictures();
    let order: Vec<ConversationId> = state.conversations.iter().map(|c| c.id).collect();
    let on_disk = PathBuf::from("/data/pictures/x.pic");
    let effects = update(
        &mut state,
        AppEvent::PicturesListed(vec![(order[0], Some(on_disk.clone()))]),
    );
    let first = fetches(&effects);
    assert_eq!(first, order[1..3].to_vec(), "the first on disk is kept");
    assert_eq!(state.pictures.files.get(&order[0]), Some(&on_disk));
    assert_eq!(
        fetches(&update(&mut state, AppEvent::Tick)),
        vec![],
        "two at once"
    );

    let saved = PathBuf::from("/data/pictures/y.pic");
    let effects = update(
        &mut state,
        AppEvent::PictureFetched {
            conversation: first[0],
            result: Ok(Some(saved.clone())),
        },
    );
    assert_eq!(state.pictures.files.get(&first[0]), Some(&saved));
    assert_eq!(
        fetches(&effects),
        order.get(3).copied().into_iter().collect::<Vec<_>>()
    );
    update(
        &mut state,
        AppEvent::PictureFetched {
            conversation: first[1],
            result: Ok(None),
        },
    );
    assert!(state.pictures.none.contains(&first[1]));
}

#[test]
fn only_listed_conversations_are_fetched() {
    use faro_core::store::ConversationSummary;

    let mut state = with_pictures();
    // A contact nobody has written to: an empty conversation, not listed.
    let quiet =
        ConversationId::Direct(faro_core::domain::ServiceId::Aci(uuid::Uuid::from_u128(99)));
    state.conversations.push(ConversationSummary {
        last_activity: None,
        unread: 0,
        state: faro_core::store::ConversationState::default(),
        ..state.conversations[0].clone()
    });
    if let Some(last) = state.conversations.last_mut() {
        last.id = quiet;
    }
    let listed: Vec<ConversationId> = state.visible_conversations().map(|c| c.id).collect();
    assert!(!listed.contains(&quiet));
    let mut asked = fetches(&update(&mut state, AppEvent::PicturesListed(vec![])));
    for _ in 0..4 {
        let done: Vec<ConversationId> = state.pictures.fetching.iter().copied().collect();
        for conversation in done {
            asked.extend(fetches(&update(
                &mut state,
                AppEvent::PictureFetched {
                    conversation,
                    result: Ok(None),
                },
            )));
        }
    }
    assert!(!asked.contains(&quiet), "{asked:?}");
    assert_eq!(asked.len(), listed.len(), "every listed one, once");
}

#[test]
fn a_failed_picture_waits_for_the_connection_to_come_back() {
    let mut state = with_pictures();
    let known: Vec<(ConversationId, Option<PathBuf>)> = [climbing(), bob()]
        .into_iter()
        .map(|id| (id, None))
        .collect();
    let effects = update(&mut state, AppEvent::PicturesListed(known));
    assert_eq!(fetches(&effects), vec![alice()], "the others have none");
    let effects = update(
        &mut state,
        AppEvent::PictureFetched {
            conversation: alice(),
            result: Err(BackendError::Network("offline".into())),
        },
    );
    assert_eq!(fetches(&effects), vec![], "not again at once");
    let effects = update(
        &mut state,
        AppEvent::Backend(BackendEvent::Connection(ConnectionState::Connected)),
    );
    assert_eq!(fetches(&effects), vec![alice()], "back online: tried again");
}
