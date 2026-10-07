//! Telling the user about new messages (M6, D49): the terminal bell. (The
//! window title follows `State::unread_total` and is set by the app loop.)

use faro_core::domain::ConversationId;
use tokio::time::Instant;

use super::sending::wall_clock;
use crate::app::event::Effect;
use crate::app::state::{BELL_INTERVAL, State};

/// A new incoming message in `id` that the user hasn't seen: ring the bell
/// if the conversation notifies, the bell is on, and it hasn't rung in the
/// last `BELL_INTERVAL` (a flood rings once, not continuously).
pub fn on_unseen_message(state: &mut State, id: ConversationId) -> Vec<Effect> {
    // Before the list loads, mutes and archives aren't known yet.
    if !state.config.notifications.bell || state.loading {
        return vec![];
    }
    let notifies = state
        .summary(id)
        .is_some_and(|summary| state.notifies(summary, wall_clock()));
    if !notifies {
        return vec![];
    }
    let now = Instant::now();
    if state
        .last_bell
        .is_some_and(|rang| now < rang + BELL_INTERVAL)
    {
        return vec![];
    }
    state.last_bell = Some(now);
    vec![Effect::Bell]
}
