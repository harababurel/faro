//! What each store effect does (D113): the reducer's reads and writes
//! against the store, with the result as an `AppEvent`. The terminal app and
//! the web demo each run these one at a time and in order on their own
//! worker (so a `Persist` is done before a later `MarkRead` or
//! `LoadMessages`).

use faro_core::domain::ConversationId;
use faro_core::store::{Store, StoreError};

use super::event::{AppEvent, Effect};
use super::update::PAGE;

/// `app_state` key of the conversation to reopen on start.
const LAST_OPEN: &str = "ui.last_open_conversation";
/// `app_state` key of the reactions last sent from here, most recent first,
/// separated by spaces (D104).
const RECENT_REACTIONS: &str = "ui.recent_reactions";

/// Run one store effect; what the app loop should hear back, if anything. A
/// failure is logged and reported as `AppEvent::StoreFailed`. Effects that
/// aren't the store's are dropped with an error in the log.
pub async fn execute(store: &Store, effect: Effect) -> Option<AppEvent> {
    match run(store, effect).await {
        Ok(event) => event,
        Err(error) => {
            tracing::warn!(%error, "store operation failed");
            Some(AppEvent::StoreFailed(error.to_string()))
        }
    }
}

async fn run(store: &Store, effect: Effect) -> Result<Option<AppEvent>, StoreError> {
    Ok(match effect {
        Effect::LoadConversations => {
            let conversations = store.conversations().await?;
            let last_open = store
                .app_state(LAST_OPEN)
                .await?
                .and_then(|id| id.parse::<ConversationId>().ok());
            let recent_reactions = store
                .app_state(RECENT_REACTIONS)
                .await?
                .unwrap_or_default()
                .split_whitespace()
                .map(str::to_owned)
                .collect();
            Some(AppEvent::Loaded {
                conversations,
                last_open,
                recent_reactions,
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
        | Effect::SaveNickname(..)
        | Effect::SaveRecentReactions(_) => {
            write(store, effect).await?;
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
        | Effect::CheckCommand(_)
        | Effect::StageFile(_)
        | Effect::CheckPaste { .. }
        | Effect::CheckTerminal
        | Effect::ReadClipboardImage { .. }
        | Effect::Bell
        | Effect::DesktopNotify { .. }
        | Effect::ReleaseFocus
        | Effect::SaveSetting(_) => {
            tracing::error!("a non-store effect reached the store worker; dropped");
            None
        }
    })
}

async fn write(store: &Store, effect: Effect) -> Result<(), StoreError> {
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
        Effect::SaveRecentReactions(recent) => {
            store
                .set_app_state(RECENT_REACTIONS, &recent.join(" "))
                .await
        }
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
        | Effect::CheckCommand(_)
        | Effect::StageFile(_)
        | Effect::CheckPaste { .. }
        | Effect::CheckTerminal
        | Effect::ReadClipboardImage { .. }
        | Effect::Bell
        | Effect::DesktopNotify { .. }
        | Effect::ReleaseFocus
        | Effect::SaveSetting(_) => Ok(()),
    }
}
