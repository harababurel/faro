//! Runs the reducer's effects against the store, one at a time and in order (so a
//! `Persist` is done before a later `MarkRead` or `LoadMessages`), and sends
//! results back to the app loop as `AppEvent`s.

use faro_core::domain::ConversationId;
use faro_core::store::{Store, StoreError};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use super::event::{AppEvent, Effect};
use super::update::PAGE;

/// `app_state` key of the conversation to reopen on start.
const LAST_OPEN: &str = "ui.last_open_conversation";

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
            Some(AppEvent::Loaded {
                conversations,
                last_open,
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
        Effect::SaveAttachment {
            message,
            index,
            attachment,
        } => {
            store.set_attachment(message, index, attachment).await?;
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
        Effect::ReadElsewhere(messages) => {
            let cleared = store.read_elsewhere(messages).await?;
            (!cleared.is_empty()).then_some(AppEvent::UnreadCleared(cleared))
        }
        Effect::RememberOpen(conversation) => {
            store
                .set_app_state(LAST_OPEN, &conversation.to_string())
                .await?;
            None
        }
        Effect::SaveNickname(id, nickname) => {
            store.set_nickname(id, nickname).await?;
            None
        }
        Effect::Backend(_)
        | Effect::Copy(_)
        | Effect::OpenUrl(_)
        | Effect::OpenFile(_)
        | Effect::SaveToDownloads { .. }
        | Effect::CheckTerminal
        | Effect::Bell
        | Effect::SaveSetting(_) => {
            // `app::run` routes these elsewhere (command task, config writer…).
            tracing::error!("a non-store effect reached the store worker; dropped");
            None
        }
    })
}
