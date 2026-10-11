//! Pasting an image from the clipboard (D89, D90, D91): `Ctrl+V` asks kitty
//! for the clipboard over OSC 5522 and stages the image like a pasted file. It
//! is never sent without the user pressing Enter.
//!
//! How the reply is read (D90): the app loop drops its `EventStream` before
//! calling [`start`], this module waits for crossterm's reader to let go
//! ([`tty::barrier`]), writes each request from the app loop's thread (so it
//! never lands inside a frame), reads the reply from the tty on a blocking
//! thread, and sends [`AppEvent::ClipboardImage`] when done, which tells the
//! loop to make a new `EventStream`. Replies that arrive after faro stopped
//! waiting reach crossterm as keys; [`LateGuard`] spots them and [`drain`]
//! reads the rest the same way.

mod guard;
mod tty;

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use crossterm::event::EventStream;
use faro_core::clipboard::{self as protocol, Credentials, Focus, ReadError};
use faro_core::domain::ConversationId;
use tokio::sync::mpsc;

use self::guard::{Filtered, LateGuard};
use self::tty::{Failure, Tty};
use super::event::{AppEvent, ClipboardError};
use super::state::StagedAttachment;
use super::term_event::Event;

/// The largest image faro takes from the clipboard (owner, D91): the same as
/// the automatic image download limit.
pub const MAX_IMAGE: usize = 25 * 1024 * 1024;

/// How long kitty has to answer the type list (no prompt; covers slow SSH).
const LIST_WAIT: Duration = Duration::from_secs(5);
/// After a type list without an image, how long to wait for another one
/// (another kitty attached to the same tmux session, D91).
const MORE_REPLIES: Duration = Duration::from_millis(500);
/// How long the user has to answer kitty's prompt (owner, D91).
const PROMPT_WAIT: Duration = Duration::from_secs(120);
/// Longest pause between two packets once the data has started.
const BETWEEN_PACKETS: Duration = Duration::from_secs(5);
/// The largest type list taken (a few dozen bytes in practice).
const LIST_LIMIT: usize = 64 * 1024;

/// Per run: kitty's password (so "allow all" covers this run only, D91) and
/// request ids (so a late reply is told from the current one).
pub struct Session {
    credentials: Credentials,
    run: String,
    requests: u64,
}

impl Default for Session {
    fn default() -> Self {
        let run = uuid::Uuid::new_v4().simple().to_string();
        Self {
            credentials: Credentials::new(uuid::Uuid::new_v4().to_string(), "faro"),
            run: run.chars().take(8).collect(),
            requests: 0,
        }
    }
}

impl Session {
    fn next_id(&mut self) -> String {
        self.requests += 1;
        format!("faro-{}-{}", self.run, self.requests)
    }
}

/// Wakes the blocking reader so it gives up at once (faro is quitting).
#[derive(Clone)]
pub struct Cancel(Arc<mio::Waker>);

impl Cancel {
    pub fn cancel(&self) {
        let _ = self.0.wake();
    }
}

/// The app loop's side of pasting (D90, D91): pausing and resuming input,
/// the late-reply guard, cancelling at exit.
#[derive(Default)]
pub struct Paste {
    session: Session,
    guard: LateGuard,
    cancel: Option<Cancel>,
}

impl Paste {
    /// `Effect::ReadClipboardImage`: pause input and start the read.
    pub fn begin(
        &mut self,
        input: &mut Option<EventStream>,
        conversation: ConversationId,
        attachments_dir: PathBuf,
        results: &mpsc::UnboundedSender<AppEvent>,
    ) {
        *input = None;
        self.guard.arm();
        self.cancel = start(
            &mut self.session,
            conversation,
            attachments_dir,
            results.clone(),
        );
    }

    /// A terminal event, through the late-reply guard.
    pub fn filter(
        &mut self,
        input: &mut Option<EventStream>,
        event: Event,
        results: &mpsc::UnboundedSender<AppEvent>,
    ) -> Vec<AppEvent> {
        match self.guard.filter(event) {
            Filtered::Pass(events) => events.into_iter().map(AppEvent::Terminal).collect(),
            Filtered::Drain => {
                *input = None;
                self.cancel = drain(results.clone());
                Vec::new()
            }
        }
    }

    /// When keys held back by the guard go to the reducer after all.
    pub fn deadline(&self) -> Option<tokio::time::Instant> {
        self.guard.deadline()
    }

    pub fn flush(&mut self) -> Vec<AppEvent> {
        self.guard
            .flush()
            .into_iter()
            .map(AppEvent::Terminal)
            .collect()
    }

    /// A read or drain is over: make a new `EventStream`.
    pub fn resume(&mut self, input: &mut Option<EventStream>, event: &AppEvent) {
        if matches!(
            event,
            AppEvent::ClipboardImage { .. } | AppEvent::ClipboardDrained
        ) {
            self.cancel = None;
            input.get_or_insert_with(EventStream::new);
        }
    }

    /// faro is quitting: stop waiting on the tty.
    pub fn stop(&mut self) {
        if let Some(cancel) = self.cancel.take() {
            cancel.cancel();
        }
    }
}

/// Start reading the clipboard for `conversation`. The app loop has already
/// dropped its `EventStream`; [`AppEvent::ClipboardImage`] always comes back
/// (after any focus reports), so the loop knows when to make a new one.
pub fn start(
    session: &mut Session,
    conversation: ConversationId,
    attachments_dir: PathBuf,
    results: mpsc::UnboundedSender<AppEvent>,
) -> Option<Cancel> {
    let finish = move |results: &mpsc::UnboundedSender<AppEvent>, result| {
        let _ = results.send(AppEvent::ClipboardImage {
            conversation,
            result,
        });
    };
    let tty = match Tty::open() {
        Ok(tty) => tty,
        Err(error) => {
            finish(&results, Err(ClipboardError::Failed(error.to_string())));
            return None;
        }
    };
    let cancel = Cancel(tty.waker());
    let ids = (session.next_id(), session.next_id());
    let credentials = session.credentials.clone();
    tokio::spawn(async move {
        let mut focus = Vec::new();
        let result = read(tty, ids, credentials, attachments_dir, &mut focus).await;
        send_focus(&results, focus);
        finish(&results, result);
    });
    Some(cancel)
}

/// A late reply started arriving as keys: drain the rest from the tty (input
/// is already paused), then send [`AppEvent::ClipboardDrained`].
pub fn drain(results: mpsc::UnboundedSender<AppEvent>) -> Option<Cancel> {
    let tty = match Tty::open() {
        Ok(tty) => tty,
        Err(error) => {
            tracing::warn!(%error, "couldn't drain a late clipboard reply");
            let _ = results.send(AppEvent::ClipboardDrained);
            return None;
        }
    };
    let cancel = Cancel(tty.waker());
    tokio::spawn(async move {
        match tokio::task::spawn_blocking(move || tty::drain(tty)).await {
            Ok(Ok(drained)) => {
                tracing::info!(
                    bytes = drained.bytes,
                    done = drained.done,
                    "drained a late clipboard reply"
                );
                send_focus(&results, drained.focus);
            }
            Ok(Err(error)) => tracing::warn!(%error, "draining a late clipboard reply failed"),
            Err(error) => tracing::warn!(%error, "draining a late clipboard reply failed"),
        }
        let _ = results.send(AppEvent::ClipboardDrained);
    });
    Some(cancel)
}

/// Focus reports found in the reply's bytes go back to the reducer as the
/// events they are (D42, D90).
fn send_focus(results: &mpsc::UnboundedSender<AppEvent>, focus: Vec<Focus>) {
    for focus in focus {
        let event = match focus {
            Focus::Gained => Event::FocusGained,
            Focus::Lost => Event::FocusLost,
        };
        let _ = results.send(AppEvent::Terminal(event));
    }
}

/// Whether kitty can be reached: directly, or as one of the clients attached
/// to faro's tmux session (D91 asks every one of them), and not over mosh.
async fn reachable() -> Result<(), ClipboardError> {
    let terminal = super::system::inspect_terminal().await;
    match (terminal.kitty, terminal.mosh) {
        (true, _) => Ok(()),
        (false, true) => Err(ClipboardError::Mosh),
        (false, false) => Err(ClipboardError::NotKitty),
    }
}

/// Run `work` on a blocking thread.
async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
) -> Result<T, ClipboardError> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|error| ClipboardError::Failed(error.to_string()))
}

async fn read(
    tty: Tty,
    (list_id, data_id): (String, String),
    credentials: Credentials,
    dir: PathBuf,
    focus: &mut Vec<Focus>,
) -> Result<StagedAttachment, ClipboardError> {
    reachable().await?;
    let in_tmux = std::env::var_os("TMUX").is_some();
    blocking(tty::barrier)
        .await?
        .map_err(|error| ClipboardError::Failed(error.to_string()))?;

    write_request(&protocol::list_request(Some(&list_id)), in_tmux)?;
    let (tty, listed) = blocking(move || {
        let mut tty = tty;
        let listed = tty::list(&mut tty, &list_id);
        (tty, listed)
    })
    .await?;
    focus.extend(listed.focus);
    let types = listed.result.map_err(|failure| match failure {
        Failure::Read(ReadError::NoReply) => ClipboardError::NoAnswer,
        Failure::Read(error) => ClipboardError::Read(error),
        Failure::Io(error) => ClipboardError::Failed(error.to_string()),
    })?;
    let mime = protocol::best_image_type(&types).ok_or(ClipboardError::NoImage)?;

    write_request(
        &protocol::read_request(&[mime], Some(&data_id), Some(&credentials)),
        in_tmux,
    )?;
    let (_tty, read) = blocking(move || {
        let mut tty = tty;
        let read = tty::data(&mut tty, &data_id, MAX_IMAGE);
        (tty, read)
    })
    .await?;
    focus.extend(read.focus);
    let contents = read.result.map_err(|failure| match failure {
        Failure::Read(ReadError::NoReply) => ClipboardError::PromptTimedOut,
        Failure::Read(error) => ClipboardError::Read(error),
        Failure::Io(error) => ClipboardError::Failed(error.to_string()),
    })?;
    let entry = contents.take(mime).ok_or(ClipboardError::NoImage)?;
    blocking(move || save(&dir, &entry.mime, &entry.data))
        .await?
        .map_err(ClipboardError::Failed)
}

/// Write a request to the terminal, wrapped for tmux passthrough inside tmux
/// (a bare OSC 5522 isn't passed on). Runs on the app loop's thread, between
/// frames, so it can't land inside one.
fn write_request(request: &str, in_tmux: bool) -> Result<(), ClipboardError> {
    let sequence = if in_tmux {
        crate::tui::wrap_tmux_passthrough(request)
    } else {
        request.to_owned()
    };
    let mut out = std::io::stdout().lock();
    out.write_all(sequence.as_bytes())
        .and_then(|()| out.flush())
        .map_err(|error| ClipboardError::Failed(error.to_string()))
}

/// Whether `path` is a pasted clipboard image waiting in `attachments/`
/// (faro's own file, D91): removed once sending has copied it, and at the
/// next start (D93).
pub fn is_pasted(path: &Path, attachments_dir: &Path) -> bool {
    path.parent() == Some(attachments_dir)
        && path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with(PASTED_PREFIX))
}

/// At startup nothing is staged, so pasted images still in `attachments/`
/// were removed with `Esc` or left staged at quit: delete them (D93).
pub fn sweep(attachments_dir: PathBuf) {
    tokio::spawn(async move {
        match tokio::task::spawn_blocking(move || sweep_blocking(&attachments_dir)).await {
            Ok(0) => {}
            Ok(removed) => tracing::info!(removed, "removed pasted images left from earlier runs"),
            Err(error) => tracing::warn!(%error, "sweeping pasted images failed"),
        }
    });
}

#[expect(
    clippy::disallowed_methods,
    reason = "runs inside spawn_blocking off the UI thread"
)]
fn sweep_blocking(dir: &Path) -> usize {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| is_pasted(path, dir))
        .filter(|path| std::fs::symlink_metadata(path).is_ok_and(|meta| meta.is_file()))
        .filter(|path| std::fs::remove_file(path).is_ok())
        .count()
}

/// The start of every pasted image's file name.
const PASTED_PREFIX: &str = "clipboard-";

/// Save the image as `<attachments>/clipboard-<local time>.<ext>` (`0600`,
/// written atomically) and describe it for the staged bar.
#[expect(
    clippy::disallowed_methods,
    reason = "runs inside spawn_blocking off the UI thread"
)]
fn save(dir: &Path, mime: &str, data: &[u8]) -> Result<StagedAttachment, String> {
    let ext = protocol::image_extension(mime).unwrap_or("img");
    let stamp = jiff::Zoned::now().strftime("%Y%m%d-%H%M%S").to_string();
    let mut name = format!("{PASTED_PREFIX}{stamp}.{ext}");
    let mut n = 2;
    while std::fs::symlink_metadata(dir.join(&name)).is_ok() {
        name = format!("{PASTED_PREFIX}{stamp}-{n}.{ext}");
        n += 1;
    }
    let path = dir.join(&name);
    faro_core::paths::write_private_atomic(&path, data)
        .map_err(|error| format!("saving {}: {error}", path.display()))?;
    let (width, height) = image::ImageReader::new(std::io::Cursor::new(data))
        .with_guessed_format()
        .ok()
        .and_then(|reader| reader.into_dimensions().ok())
        .map_or((None, None), |(w, h)| (Some(w), Some(h)));
    Ok(StagedAttachment {
        path,
        name,
        content_type: mime.to_owned(),
        size: u64::try_from(data.len()).unwrap_or(u64::MAX),
        width,
        height,
    })
}

#[cfg(test)]
mod tests;
