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
        Effect::LoadMessages(conversation) => Some(AppEvent::MessagesLoaded {
            conversation,
            messages: store.messages(conversation, None, PAGE).await?,
        }),
        Effect::LoadOlder {
            conversation,
            before,
        } => Some(AppEvent::OlderLoaded {
            conversation,
            messages: store.messages(conversation, Some(before), PAGE).await?,
        }),
        Effect::LoadMessagesAround {
            conversation,
            target,
        } => Some(AppEvent::MessagesAroundLoaded {
            conversation,
            target,
            messages: store
                .messages_around(conversation, target.sent_at, 100)
                .await?,
        }),
        Effect::SearchMessages {
            conversation,
            query,
        } => Some(AppEvent::SearchResults {
            conversation,
            hits: store.search_messages(conversation, &query, 50).await?,
            query,
        }),
        Effect::LoadMessageReceipts {
            conversation,
            message,
        } => Some(AppEvent::MessageReceiptsLoaded {
            conversation,
            message,
            receipts: store.message_receipts(message).await?,
        }),
        Effect::ReadElsewhere(messages) => {
            let cleared = store.read_elsewhere(messages).await?;
            (!cleared.is_empty()).then_some(AppEvent::UnreadCleared(cleared))
        }
        Effect::Persist(_)
        | Effect::SaveAttachment { .. }
        | Effect::SaveConversationState(..)
        | Effect::MarkRead(_)
        | Effect::RememberOpen(_)
        | Effect::SaveNickname(..) => {
            write_effect(store, effect).await?;
            None
        }
        Effect::Backend(_)
        | Effect::Copy(_)
        | Effect::OpenUrl(_)
        | Effect::OpenFile(_)
        | Effect::PlayAudio { .. }
        | Effect::StopAudio
        | Effect::SaveToDownloads { .. }
        | Effect::ListFiles(_)
        | Effect::StageFile(_)
        | Effect::CheckPaste { .. }
        | Effect::CheckTerminal
        | Effect::ReadClipboardImage { .. }
        | Effect::Bell
        | Effect::DesktopNotify { .. }
        | Effect::SaveSetting(_) => {
            tracing::error!("a non-store effect reached the store worker; dropped");
            None
        }
    })
}

async fn write_effect(store: &Store, effect: Effect) -> Result<(), StoreError> {
    match effect {
        Effect::Persist(event) => store.apply(*event).await,
        Effect::SaveAttachment {
            message,
            index,
            attachment,
        } => store.set_attachment(message, index, attachment).await,
        Effect::SaveConversationState(conversation, state) => {
            store.set_conversation_state(conversation, *state).await
        }
        Effect::MarkRead(conversation) => store.mark_read(conversation).await,
        Effect::RememberOpen(conversation) => {
            store
                .set_app_state(LAST_OPEN, &conversation.to_string())
                .await
        }
        Effect::SaveNickname(id, nickname) => store.set_nickname(id, nickname).await,
        Effect::LoadConversations
        | Effect::LoadMessages(_)
        | Effect::LoadOlder { .. }
        | Effect::LoadMessagesAround { .. }
        | Effect::SearchMessages { .. }
        | Effect::LoadMessageReceipts { .. }
        | Effect::ReadElsewhere(_)
        | Effect::Backend(_)
        | Effect::Copy(_)
        | Effect::OpenUrl(_)
        | Effect::OpenFile(_)
        | Effect::PlayAudio { .. }
        | Effect::StopAudio
        | Effect::SaveToDownloads { .. }
        | Effect::ListFiles(_)
        | Effect::StageFile(_)
        | Effect::CheckPaste { .. }
        | Effect::CheckTerminal
        | Effect::ReadClipboardImage { .. }
        | Effect::Bell
        | Effect::DesktopNotify { .. }
        | Effect::SaveSetting(_) => Ok(()),
    }
}
