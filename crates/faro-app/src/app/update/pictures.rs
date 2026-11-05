//! Conversations' pictures (D160): read the pictures folder once at startup,
//! then fetch, a couple at a time, the pictures of listed conversations that
//! have none on disk yet. Each is fetched once and kept (`faro
//! refresh-pictures` empties the folder). Only with `avatars = "pictures"`
//! or `"auto"`, where the terminal can draw them and the backend can fetch
//! them: otherwise faro asks Signal for nothing.

use std::path::PathBuf;

use faro_core::backend::{BackendError, Capability};
use faro_core::config::Avatars;
use faro_core::domain::ConversationId;

use super::super::event::{Command, Effect};
use super::super::state::{SidebarView, State};

/// Pictures fetched at the same time.
pub const MAX_IN_FLIGHT: usize = 2;

/// The folder's contents: pictures and "has none" marks.
pub fn on_listed(state: &mut State, found: Vec<(ConversationId, Option<PathBuf>)>) -> Vec<Effect> {
    let pictures = &mut state.pictures;
    pictures.listed = true;
    for (id, file) in found {
        match file {
            Some(path) => {
                pictures.none.remove(&id);
                pictures.files.insert(id, path);
            }
            None => {
                if !pictures.files.contains_key(&id) {
                    pictures.none.insert(id);
                }
            }
        }
    }
    vec![]
}

pub fn on_fetched(
    state: &mut State,
    conversation: ConversationId,
    result: Result<Option<PathBuf>, BackendError>,
) -> Vec<Effect> {
    let pictures = &mut state.pictures;
    pictures.fetching.remove(&conversation);
    match result {
        Ok(Some(path)) => {
            pictures.none.remove(&conversation);
            pictures.files.insert(conversation, path);
        }
        Ok(None) => {
            pictures.none.insert(conversation);
        }
        Err(error) => {
            tracing::debug!(%error, %conversation, "no picture this time");
            pictures.failed.insert(conversation);
        }
    }
    vec![]
}

/// Fetches to start now: the first listed conversations (in list order)
/// whose picture isn't known yet, while fewer than [`MAX_IN_FLIGHT`] are
/// being fetched. Cheap when there's nothing to do: called after every event.
pub fn wanted(state: &mut State) -> Vec<Effect> {
    let pictures = &state.pictures;
    let wanted = match state.config.conversations.avatars {
        Avatars::Pictures | Avatars::Auto => true,
        Avatars::Off | Avatars::Initials => false,
    };
    // Nothing a terminal can't draw (mosh, half-blocks, one-row entries):
    // fetched once it can, e.g. after reattaching from kitty (D162).
    if !wanted
        || !state.draws_pictures()
        || !state.capabilities.supports(Capability::Pictures)
        || !pictures.listed
        || state.loading
        || pictures.fetching.len() >= MAX_IN_FLIGHT
    {
        return vec![];
    }
    let room = MAX_IN_FLIGHT - pictures.fetching.len();
    // What the list shows (chats, then the archive), never every contact:
    // there's an empty conversation for each.
    let next: Vec<ConversationId> = state
        .listed_in(SidebarView::Chats)
        .chain(state.listed_in(SidebarView::Archive))
        .map(|summary| summary.id)
        .filter(|id| {
            !pictures.files.contains_key(id)
                && !pictures.none.contains(id)
                && !pictures.fetching.contains(id)
                && !pictures.failed.contains(id)
        })
        .take(room)
        .collect();
    next.into_iter()
        .map(|id| {
            state.pictures.fetching.insert(id);
            Effect::Backend(Command::FetchPicture(id))
        })
        .collect()
}
