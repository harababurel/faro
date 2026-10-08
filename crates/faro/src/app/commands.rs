//! Runs the reducer's backend commands (sending, reactions, read receipts,
//! typing) one at a time, in order, and reports results back to the app loop
//! as `AppEvent`s (D39). Attachment downloads (`Command::Fetch`, D75) are
//! spawned as concurrent local tasks so they never block sends or receipts.
//!
//! Backend futures need not be `Send` (D30), so the task is local to the
//! current-thread runtime (`spawn_local`, inside the `LocalSet` set up in `main`).

use std::path::{Path, PathBuf};
use std::rc::Rc;

use faro_core::backend::{Backend, BackendError, OutgoingMessage, ReactionChange};
use faro_core::domain::{
    AttachmentRef, AttachmentStatus, ConversationId, MessageRef, ServiceId, Timestamp, TypingState,
};
use faro_core::paths;
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
    /// Download an attachment and write it to `attachments/` (`0600`, D75).
    /// Answered with `AppEvent::Fetched`.
    Fetch {
        conversation: ConversationId,
        message: MessageRef,
        index: usize,
        attachment: AttachmentRef,
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

/// Abort spawned download tasks when the main command task is aborted.
#[derive(Default)]
struct InFlight(Vec<JoinHandle<()>>);

impl InFlight {
    fn spawn(&mut self, task: JoinHandle<()>) {
        self.0.retain(|h| !h.is_finished());
        self.0.push(task);
    }
}

impl Drop for InFlight {
    fn drop(&mut self) {
        for handle in &self.0 {
            handle.abort();
        }
    }
}

/// Start the task; it owns the backend and drops it when the sender is dropped
/// (or the `LocalSet` ends). Commands are queued without limit, like effects:
/// the reducer must never wait.
pub fn spawn<B: Backend + 'static>(
    backend: B,
    me: ServiceId,
    attachments_dir: PathBuf,
    results: mpsc::UnboundedSender<AppEvent>,
) -> (mpsc::UnboundedSender<Command>, JoinHandle<()>) {
    let (commands, queue) = mpsc::unbounded_channel();
    let task = tokio::task::spawn_local(run(Rc::new(backend), me, attachments_dir, queue, results));
    (commands, task)
}

async fn run<B: Backend + 'static>(
    backend: Rc<B>,
    me: ServiceId,
    attachments_dir: PathBuf,
    mut queue: mpsc::UnboundedReceiver<Command>,
    results: mpsc::UnboundedSender<AppEvent>,
) {
    let mut in_flight = InFlight::default();
    while let Some(command) = queue.recv().await {
        if let Command::Fetch {
            conversation,
            message,
            index,
            attachment,
        } = command
        {
            let backend = Rc::clone(&backend);
            let dir = attachments_dir.clone();
            let tx = results.clone();
            in_flight.spawn(tokio::task::spawn_local(async move {
                let result = fetch(backend.as_ref(), &dir, message, index, attachment).await;
                if let Err(error) = &result {
                    tracing::warn!(%error, %conversation, "attachment download failed");
                }
                let _ = tx.send(AppEvent::Fetched {
                    conversation,
                    message,
                    index,
                    result,
                });
            }));
            continue;
        }
        let Some(event) = execute(backend.as_ref(), me, &attachments_dir, command).await else {
            continue;
        };
        if results.send(event).is_err() {
            tracing::debug!("app loop stopped before a backend result arrived");
        }
    }
}

async fn fetch<B: Backend>(
    backend: &B,
    attachments_dir: &Path,
    message: MessageRef,
    index: usize,
    mut attachment: AttachmentRef,
) -> Result<AttachmentRef, BackendError> {
    let bytes = backend.fetch_attachment(&attachment).await?;
    let size = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    let name = paths::attachment_file_name(
        message,
        index,
        attachment.file_name.as_deref(),
        &attachment.content_type,
    );
    let path = attachments_dir.join(name);
    let needs_dims =
        attachment.is_image() && (attachment.width.is_none() || attachment.height.is_none());
    let needs_audio = attachment.is_voice_note()
        && (attachment.duration_secs.is_none() || attachment.waveform.is_none());
    let (path, dims, wav_info, fallback_waveform) = tokio::task::spawn_blocking(move || {
        let dims = if needs_dims {
            image::ImageReader::new(std::io::Cursor::new(&bytes))
                .with_guessed_format()
                .ok()
                .and_then(|r| r.into_dimensions().ok())
        } else {
            None
        };
        let (wav_info, fallback_waveform) = if needs_audio {
            let wav = faro_core::audio::inspect_wav(&bytes);
            let synth = wav
                .is_none()
                .then(|| faro_core::audio::synthetic_waveform(&bytes));
            (wav, synth)
        } else {
            (None, None)
        };
        paths::write_private_atomic(&path, &bytes)
            .map(|()| (path, dims, wav_info, fallback_waveform))
            .map_err(|e| BackendError::Storage(format!("writing attachment: {e}")))
    })
    .await
    .map_err(|e| BackendError::Storage(format!("attachment writer task: {e}")))??;
    if let Some((w, h)) = dims.filter(|(w, h)| *w > 0 && *h > 0) {
        attachment.width = Some(w);
        attachment.height = Some(h);
    }
    if needs_audio {
        let info = match wav_info {
            Some(info) => Some(info),
            None => inspect_audio_with_ffmpeg(&path).await,
        };
        if let Some(info) = info {
            if attachment.duration_secs.is_none() {
                attachment.duration_secs = info.duration_secs;
            }
            if attachment.waveform.is_none() {
                attachment.waveform = Some(info.waveform);
            }
        } else if attachment.waveform.is_none() {
            attachment.waveform = fallback_waveform;
        }
    }
    attachment.size = Some(size);
    attachment.status = AttachmentStatus::Downloaded(path);
    Ok(attachment)
}

async fn inspect_audio_with_ffmpeg(path: &Path) -> Option<faro_core::audio::AudioInfo> {
    use std::process::Stdio;
    let fut = tokio::process::Command::new("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(path)
        .args(["-ac", "1", "-ar", "8000", "-f", "s16le", "-"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .output();
    let output = tokio::time::timeout(std::time::Duration::from_secs(5), fut)
        .await
        .ok()?
        .ok()?;
    if !output.status.success() {
        return None;
    }
    faro_core::audio::pcm_s16le_info(&output.stdout, 8_000)
}

async fn execute<B: Backend>(
    backend: &B,
    me: ServiceId,
    attachments_dir: &Path,
    command: Command,
) -> Option<AppEvent> {
    let event = match command {
        Command::Send {
            conversation,
            mut message,
        } => {
            let sent_at = message.sent_at;
            let copy_result =
                copy_outgoing_attachments(me, attachments_dir, sent_at, &mut message).await;
            let result = match copy_result {
                Ok(()) => backend.send(conversation, message).await,
                Err(error) => Err(error),
            };
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
        Command::Fetch { .. } => return None,
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

async fn copy_outgoing_attachments(
    me: ServiceId,
    attachments_dir: &Path,
    sent_at: Timestamp,
    message: &mut OutgoingMessage,
) -> Result<(), BackendError> {
    if message.attachments.is_empty() {
        return Ok(());
    }
    let msg_ref = MessageRef {
        sender: me,
        sent_at,
    };
    for (index, att) in message.attachments.iter_mut().enumerate() {
        let name = paths::attachment_file_name(
            msg_ref,
            index,
            att.file_name.as_deref(),
            &att.content_type,
        );
        let dest = attachments_dir.join(name);
        if att.path == dest {
            continue;
        }
        let src = att.path.clone();
        let target = dest.clone();
        tokio::task::spawn_blocking(move || copy_attachment_file(&src, &target))
            .await
            .map_err(|e| BackendError::Storage(format!("attachment copy task: {e}")))??;
        att.path = dest;
    }
    Ok(())
}

#[expect(
    clippy::disallowed_methods,
    reason = "copying an outgoing attachment into faro's attachments/ runs inside spawn_blocking"
)]
fn copy_attachment_file(src: &Path, dest: &Path) -> Result<(), BackendError> {
    let bytes = std::fs::read(src)
        .map_err(|e| BackendError::Storage(format!("reading {}: {e}", src.display())))?;
    paths::write_private_atomic(dest, &bytes)
        .map_err(|e| BackendError::Storage(format!("writing {}: {e}", dest.display())))
}
