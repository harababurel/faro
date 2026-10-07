//! Runs the reducer's effects against the store, one at a time and in order (so a
//! `Persist` is done before a later `MarkRead` or `LoadMessages`), and sends
//! results back to the app loop as `AppEvent`s.

use faro_core::domain::ConversationId;
use faro_core::store::{Store, StoreError};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use super::event::{AppEvent, Effect};
use super::state::MessageLayout;
use super::update::PAGE;

/// `app_state` key of the conversation to reopen on start.
const LAST_OPEN: &str = "ui.last_open_conversation";
/// `app_state` key for allowlist mode (`on`/`off`); moves to the config file
/// when that exists (M5).
const ALLOWLIST_MODE: &str = "ui.allowlist_mode";
/// `app_state` key for the message layout (`bubbles`/`irc`); moves to the
/// config file in M5 (D45).
const MESSAGE_LAYOUT: &str = "ui.message_layout";

/// Start the worker. Effects are queued without limit: the reducer must never
/// wait, and the queue only grows as fast as the backend produces events.
/// Dropping the sender lets the worker finish the queue and stop; await the
/// handle before closing the store.
pub fn spawn(
    store: Store,
    results: mpsc::UnboundedSender<AppEvent>,
) -> (mpsc::UnboundedSender<Effect>, JoinHandle<()>) {
    let (effects, queue) = mpsc::unbounded_channel();
    let worker = tokio::spawn(run(store, queue, results));
    (effects, worker)
}

async fn run(
    store: Store,
    mut queue: mpsc::UnboundedReceiver<Effect>,
    results: mpsc::UnboundedSender<AppEvent>,
) {
    while let Some(effect) = queue.recv().await {
        let result = match execute(&store, effect).await {
            Ok(Some(event)) => Some(event),
            Ok(None) => None,
            Err(error) => {
                tracing::warn!(%error, "store operation failed");
                Some(AppEvent::StoreFailed(error.to_string()))
            }
        };
        if let Some(event) = result
            && results.send(event).is_err()
        {
            // The app loop is gone; keep draining so queued writes still happen.
            tracing::debug!("app loop stopped before a store result arrived");
        }
    }
}

async fn execute(store: &Store, effect: Effect) -> Result<Option<AppEvent>, StoreError> {
    Ok(match effect {
        Effect::LoadConversations => {
            let conversations = store.conversations().await?;
            let last_open = store
                .app_state(LAST_OPEN)
                .await?
                .and_then(|id| id.parse::<ConversationId>().ok());
            let allowlist = store.app_state(ALLOWLIST_MODE).await?.as_deref() == Some("on");
            let layout = store
                .app_state(MESSAGE_LAYOUT)
                .await?
                .as_deref()
                .and_then(MessageLayout::parse);
            Some(AppEvent::Loaded {
                conversations,
                last_open,
                allowlist,
                layout,
            })
        }
        Effect::LoadMessages(conversation) => {
            let messages = store.messages(conversation, None, PAGE).await?;
            Some(AppEvent::MessagesLoaded {
                conversation,
                messages,
            })
        }
        Effect::LoadOlder {
            conversation,
            before,
        } => {
            let messages = store.messages(conversation, Some(before), PAGE).await?;
            Some(AppEvent::OlderLoaded {
                conversation,
                messages,
            })
        }
        Effect::Persist(event) => {
            store.apply(*event).await?;
            None
        }
        Effect::SaveConversationState(conversation, state) => {
            store.set_conversation_state(conversation, *state).await?;
            None
        }
        Effect::MarkRead(conversation) => {
            store.mark_read(conversation).await?;
            None
        }
        Effect::RememberOpen(conversation) => {
            store
                .set_app_state(LAST_OPEN, &conversation.to_string())
                .await?;
            None
        }
        Effect::SetAllowlistMode(on) => {
            store
                .set_app_state(ALLOWLIST_MODE, if on { "on" } else { "off" })
                .await?;
            None
        }
        Effect::SetMessageLayout(layout) => {
            store.set_app_state(MESSAGE_LAYOUT, layout.name()).await?;
            None
        }
        Effect::Backend(_) | Effect::Copy(_) | Effect::OpenUrl(_) | Effect::CheckTerminal => {
            // `app::run` routes these to the command task.
            tracing::error!("a non-store effect reached the store worker; dropped");
            None
        }
    })
}
