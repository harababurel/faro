//! The demo's starting point (D112): the mock's backlog in the in-memory
//! store, loaded through the real reducer, the newest chat open.

use std::time::Duration;

use faro_app::app::event::AppEvent;
use faro_app::app::image_cache::NoImages;
use faro_app::app::state::State;
use faro_app::app::term_event::{Event, KeyCode, KeyEvent};
use faro_app::app::update::update;
use faro_app::theme::Theme;
use faro_app::ui::{self, View};
use faro_core::backend::{Backend as _, BackendEvent, BackendEvents};
use faro_core::domain::{ConnectionState, ConversationId};
use faro_core::mock::{DETERMINISTIC_EPOCH, MockBackend, MockConfig};
use faro_core::rt;
use faro_core::store::Store;
use ratatui::Terminal;
use ratatui::buffer::Buffer;
use tokio::sync::mpsc::error::TryRecvError;

use crate::backend::CanvasBackend;

/// The demo's seed: the same conversation for every visitor.
pub const SEED: u64 = 42;

/// How often to look for more of the backlog, and how many empty looks in a
/// row mean it has all arrived (the mock sends it in one go after connecting).
const POLL: Duration = Duration::from_millis(100);
const QUIET_POLLS: u32 = 3;

pub type Error = Box<dyn std::error::Error>;

/// A first frame: [`first_state`] drawn into a `width × height` buffer.
pub async fn first_frame(width: u16, height: u16) -> Result<Buffer, Error> {
    Ok(draw(&first_state().await?, width, height))
}

/// The state a visitor starts from: the mock's backlog stored, the chat list
/// loaded and the newest chat open. (Which chat the real demo opens is still
/// to be decided.)
pub async fn first_state() -> Result<State, Error> {
    let (mut state, store) = stored_state(&MockConfig {
        activity: None,
        disconnects: false,
        errors: false,
        ..MockConfig::deterministic(SEED)
    })
    .await?;
    // Open the newest chat, as a visitor pressing Enter would.
    if let Some(newest) = state.highlighted() {
        open(&mut state, &store, newest).await?;
    }
    store.close().await?;
    Ok(state)
}

/// The same with `mock`, and nothing open: the store stays open so the
/// caller can open a conversation of its own the way the app does (the site
/// poster, §6.4). Keep the mock deterministic (`MockConfig::deterministic`,
/// no activity) if the frame has to repeat.
pub async fn stored_state(mock: &MockConfig) -> Result<(State, Store), Error> {
    let (backend, events) = MockBackend::new(mock);
    let me = backend.whoami().await?.service_id;
    let contacts = backend.contacts().await?;
    let groups = backend.groups().await?;
    let backlog = backlog(events).await;
    drop(backend);

    let store = Store::memory();
    store.upsert_contacts(contacts.clone()).await?;
    store.upsert_groups(groups.clone()).await?;
    for event in backlog {
        store.apply(event).await?;
    }
    let mut state = State::new(me, contacts, groups);
    update(
        &mut state,
        AppEvent::Loaded {
            conversations: store.conversations().await?,
            last_open: None,
            recent_reactions: Vec::new(),
        },
    );
    update(
        &mut state,
        AppEvent::Backend(BackendEvent::Connection(ConnectionState::Connected)),
    );
    Ok((state, store))
}

/// Open `conversation` and load its history from `store`, as a visitor
/// pressing Enter on it in the chat list would.
pub async fn open(
    state: &mut State,
    store: &Store,
    conversation: ConversationId,
) -> Result<(), Error> {
    let enter = Event::Key(KeyEvent::from(KeyCode::Enter));
    update(state, AppEvent::Terminal(enter));
    update(
        state,
        AppEvent::MessagesLoaded {
            conversation,
            messages: store.messages(conversation, None, 200).await?,
        },
    );
    Ok(())
}

/// Everything the mock delivers on connecting: it says `Connected`, then
/// sends the backlog in one go.
async fn backlog(mut events: BackendEvents) -> Vec<BackendEvent> {
    let mut backlog = Vec::new();
    let mut connected = false;
    let mut quiet = 0;
    while quiet < QUIET_POLLS {
        match events.try_recv() {
            Ok(event) => {
                connected |= event == BackendEvent::Connection(ConnectionState::Connected);
                backlog.push(event);
                quiet = 0;
            }
            Err(TryRecvError::Empty) => {
                if connected {
                    quiet += 1;
                }
                rt::sleep(POLL).await;
            }
            Err(TryRecvError::Disconnected) => break,
        }
    }
    backlog
}

/// Draw `state` into a `width × height` buffer: the theme its config names
/// (resolved the way the app loop resolves it), the deterministic clock and
/// the still logo frame, so the picture is byte-identical across runs and
/// days. The first frame and the site poster both come through here.
pub fn draw(state: &State, width: u16, height: u16) -> Buffer {
    let theme = Theme::find(state.theme_name(), &state.user_themes)
        .cloned()
        .unwrap_or_default();
    let tz = jiff::tz::TimeZone::UTC;
    let now = i64::try_from(DETERMINISTIC_EPOCH.as_millis())
        .ok()
        .and_then(|ms| jiff::Timestamp::from_millisecond(ms).ok())
        .unwrap_or(jiff::Timestamp::UNIX_EPOCH);
    let view = View {
        theme: &theme,
        tz: &tz,
        now,
        logo_time: None,
        images: &NoImages,
    };
    let Ok(mut terminal) = Terminal::new(CanvasBackend::new(width, height));
    // `CanvasBackend` can't fail.
    let Ok(_) = terminal.draw(|frame| {
        ui::render(frame, state, &view);
    });
    terminal.backend().buffer().clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    #[expect(
        clippy::disallowed_methods,
        reason = "#[tokio::test] expands to block_on; tests may block"
    )]
    async fn the_first_frame_shows_the_demo_conversation() {
        let buffer = first_frame(120, 34).await.unwrap();
        let text = faro_app::ui::text::grid(&buffer);
        // The chat list, and the newest chat open with its messages.
        assert!(text.contains("╭ Conversations"), "{text}");
        assert!(text.contains("╭ Michael Scott"), "{text}");
        assert!(!text.contains("Loading…"), "{text}");
        assert!(!text.contains("connecting…"), "{text}");
    }
}
