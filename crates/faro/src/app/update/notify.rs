//! Telling the user about new messages (M6, D49): the terminal bell and
//! desktop notifications. (The window title follows `State::unread_total`
//! and is set by the app loop.)

use faro_core::config::{DesktopNotifications, NotificationPrivacy};
use faro_core::domain::{ConversationId, Message};
use tokio::time::Instant;

use super::sending::wall_clock;
use crate::app::event::Effect;
use crate::app::state::{BELL_INTERVAL, State};
use crate::ui::text::sanitize;

/// Maximum characters shown in a `Preview` notification body.
const MAX_PREVIEW_CHARS: usize = 200;

/// A new incoming message the user hasn't seen: ring the bell (and send a
/// desktop notification if configured) if it rings in its conversation
/// (`State::rings_for`: every message, or mentions of us), the bell is on,
/// and it hasn't rung in the last `BELL_INTERVAL` (a flood rings once, not
/// continuously).
pub fn on_unseen_message(state: &mut State, message: &Message) -> Vec<Effect> {
    // Before the list loads, mutes and archives aren't known yet.
    if !state.config.notifications.bell || state.loading {
        return vec![];
    }
    let notifies = state
        .summary(message.conversation)
        .is_some_and(|summary| state.rings_for(summary, message, wall_clock()));
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
    let mut effects = vec![Effect::Bell];
    if state.config.notifications.desktop != DesktopNotifications::Off {
        let (title, body) = desktop_notification(state, message);
        effects.push(Effect::DesktopNotify { title, body });
    }
    effects
}

/// Format the `(title, body)` for a desktop notification according to
/// `[notifications] privacy` (DESIGN.md §8).
pub fn desktop_notification(state: &State, message: &Message) -> (String, String) {
    match state.config.notifications.privacy {
        NotificationPrivacy::None => ("faro".to_owned(), "New message".to_owned()),
        NotificationPrivacy::Sender => {
            let sender = sanitize(&state.person_name(message.sender)).into_owned();
            let body = match message.conversation {
                ConversationId::Direct(_) => format!("New message from {sender}"),
                ConversationId::Group(_) => {
                    let group =
                        sanitize(&state.conversation_title(message.conversation)).into_owned();
                    format!("New message from {sender} in {group}")
                }
            };
            ("faro".to_owned(), body)
        }
        NotificationPrivacy::Preview => {
            let title = sanitize(&state.conversation_title(message.conversation)).into_owned();
            let raw_text = state
                .readable_body(message)
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "New message".to_owned());
            let full = match message.conversation {
                ConversationId::Direct(_) => raw_text,
                ConversationId::Group(_) => {
                    format!("{}: {raw_text}", state.person_name(message.sender))
                }
            };
            let sanitized = sanitize(&full);
            let single_line: String = sanitized
                .chars()
                .map(|c| if c.is_control() { ' ' } else { c })
                .collect();
            (title, truncate_chars(&single_line, MAX_PREVIEW_CHARS))
        }
    }
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    let mut chars = text.chars();
    let prefix: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{prefix}…")
    } else {
        prefix
    }
}
