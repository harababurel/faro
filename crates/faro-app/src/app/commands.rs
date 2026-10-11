//! What each messaging command does (D113): the reducer's backend commands
//! run against any `Backend`, with the result as an `AppEvent`. The terminal
//! app and the web demo each run these one at a time and in order on their
//! own command task; downloads and transcription are theirs (files, processes
//! and memory differ).

use faro_core::backend::Backend;

use super::event::{AppEvent, Command};

/// Run one command; what the app loop should hear back, if anything.
/// Outgoing attachments must already be where the backend can read them.
/// `Fetch` and `Transcribe` aren't run here: they return `None`.
pub async fn execute<B: Backend>(backend: &B, command: Command) -> Option<AppEvent> {
    let event = match command {
        Command::Send {
            conversation,
            message,
        } => {
            let sent_at = message.sent_at;
            let result = backend.send(conversation, message).await;
            if let Err(error) = &result {
                tracing::warn!(%error, %conversation, "sending failed");
            }
            AppEvent::SendResult { sent_at, result }
        }
        Command::React {
            conversation,
            target,
            change,
            previous,
        } => {
            let result = backend.react(conversation, target, change).await;
            if let Err(error) = &result {
                tracing::warn!(%error, %conversation, "reacting failed");
            }
            AppEvent::ReactResult {
                conversation,
                target,
                previous,
                result,
            }
        }
        Command::Edit {
            conversation,
            target,
            edit,
        } => {
            let result = backend.edit(conversation, target, edit).await;
            if let Err(error) = &result {
                tracing::warn!(%error, %conversation, "editing message failed");
            }
            AppEvent::EditResult(result)
        }
        Command::DeleteForEveryone {
            conversation,
            target,
        } => {
            let result = backend.delete_for_everyone(conversation, target).await;
            if let Err(error) = &result {
                tracing::warn!(%error, %conversation, "deleting message failed");
            }
            AppEvent::DeleteResult(result)
        }
        Command::Fetch { .. } | Command::Transcribe { .. } => return None,
        Command::MarkRead(messages) => {
            if let Err(error) = backend.mark_read(&messages).await {
                tracing::warn!(%error, count = messages.len(), "sending read receipts failed");
            }
            return None;
        }
        Command::Typing {
            conversation,
            state,
        } => {
            if let Err(error) = backend.send_typing(conversation, state).await {
                tracing::debug!(%error, %conversation, "sending a typing indicator failed");
            }
            return None;
        }
    };
    Some(event)
}
