//! Runs the reducer's backend commands (sending, reactions, read receipts,
//! typing) one at a time, in order, and reports results back to the app loop
//! as `AppEvent`s (D39).
//!
//! Backend futures need not be `Send` (D30), so the task is local to the
//! current-thread runtime (`spawn_local`, inside the `LocalSet` set up in `main`).

use faro_core::backend::{Backend, OutgoingMessage, ReactionChange};
use faro_core::domain::{ConversationId, MessageRef, TypingState};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use super::event::AppEvent;

/// Work for the backend.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// Answered with `AppEvent::SendResult`. The message is already stored
    /// locally as `Sending`.
    Send {
        conversation: ConversationId,
        message: OutgoingMessage,
    },
    /// Answered with `AppEvent::ReactResult`. Already applied locally;
    /// `previous` is what to restore if it fails.
    React {
        conversation: ConversationId,
        target: MessageRef,
        change: ReactionChange,
        previous: Option<String>,
    },
    /// Read receipts for messages that have been seen (D42). No answer;
    /// failures are only logged.
    MarkRead(Vec<MessageRef>),
    /// Our typing indicator (D42). No answer; failures are only logged.
    Typing {
        conversation: ConversationId,
        state: TypingState,
    },
}

/// Start the task; it owns the backend and drops it when the sender is dropped
/// (or the `LocalSet` ends). Commands are queued without limit, like effects:
/// the reducer must never wait.
pub fn spawn<B: Backend + 'static>(
    backend: B,
    results: mpsc::UnboundedSender<AppEvent>,
) -> (mpsc::UnboundedSender<Command>, JoinHandle<()>) {
    let (commands, queue) = mpsc::unbounded_channel();
    let task = tokio::task::spawn_local(run(backend, queue, results));
    (commands, task)
}

async fn run<B: Backend>(
    backend: B,
    mut queue: mpsc::UnboundedReceiver<Command>,
    results: mpsc::UnboundedSender<AppEvent>,
) {
    while let Some(command) = queue.recv().await {
        let Some(event) = execute(&backend, command).await else {
            continue;
        };
        if results.send(event).is_err() {
            tracing::debug!("app loop stopped before a backend result arrived");
        }
    }
}

async fn execute<B: Backend>(backend: &B, command: Command) -> Option<AppEvent> {
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
