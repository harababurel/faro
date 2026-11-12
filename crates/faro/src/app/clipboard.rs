//! Pasting an image from the clipboard (D89, D90, D91, D150): `Ctrl+V` asks
//! the terminal for the clipboard over OSC 5522 (kitty, or any terminal that
//! says it knows the protocol, such as newer Ghostty) or, when it can't and
//! faro runs on the user's computer, reads that computer's clipboard
//! ([`system`]). The image is staged like a pasted file and never sent
//! without the user pressing Enter.
//!
//! How the reply is read (D90): the app loop pauses its `EventStream` for the
//! `OSC 5522` tty exchange, [`tty`] forwards typed keys, live byte progress,
//! and instant `Esc` / `Ctrl+C` cancellation, and as soon as the tty lets go
//! (before local system clipboard reads or TIFF→PNG conversion) sends
//! [`AppEvent::ClipboardStage`] so `EventStream` resumes early.

mod guard;
pub(super) mod keys;
mod system;
pub(super) mod tty;

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crossterm::event::EventStream;
use faro_core::clipboard::{self as protocol, Credentials, Focus, ReadError};
use faro_core::domain::ConversationId;
use tokio::sync::{Notify, mpsc};

use self::guard::{Filtered, LateGuard};
use self::tty::{Failure, Observer, Tty, TtyWake};
use super::event::{AppEvent, ClipboardError, PasteStage};
use super::state::{StagedAttachment, Terminal};
use super::term_event::Event;

/// The largest image faro takes from the clipboard (owner, D91): the same as
/// the automatic image download limit.
pub const MAX_IMAGE: usize = 25 * 1024 * 1024;
/// TIFF is uncompressed, so it may arrive this large; the PNG it becomes
/// must still fit [`MAX_IMAGE`] (D150).
const MAX_TIFF: usize = 100 * 1024 * 1024;

/// The most taken of `mime` before converting it.
fn limit_for(mime: &str) -> usize {
    if mime == protocol::TIFF {
        MAX_TIFF
    } else {
        MAX_IMAGE
    }
}

/// How long a remote terminal has to answer whether it knows OSC 5522 (covers
/// slow SSH links and tmux without `allow-passthrough`).
const SUPPORT_WAIT: Duration = Duration::from_secs(3);
/// Local terminals answer status reports in < 1 ms; when local tmux drops the
/// query (`allow-passthrough` off), fall back quickly to the OS clipboard.
const LOCAL_SUPPORT_WAIT: Duration = Duration::from_millis(200);

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

/// Wakes an in-flight read or cancels off-tty clipboard / staging work.
pub struct Cancel {
    tty_wake: Option<TtyWake>,
    cancelled: Arc<AtomicBool>,
    notify: Arc<Notify>,
}

impl Cancel {
    fn from_tty(wake: TtyWake) -> Self {
        Self {
            tty_wake: Some(wake),
            cancelled: Arc::new(AtomicBool::new(false)),
            notify: Arc::new(Notify::new()),
        }
    }

    pub fn cancel_by_user(&self) {
        self.cancelled.store(true, Ordering::Release);
        self.notify.notify_waiters();
        if let Some(wake) = &self.tty_wake {
            wake.cancel_by_user();
        }
    }

    pub fn release_tty(&mut self) {
        self.tty_wake = None;
    }

    pub fn shutdown(&mut self) {
        self.cancelled.store(true, Ordering::Release);
        self.notify.notify_waiters();
        if let Some(wake) = self.tty_wake.take() {
            wake.shutdown();
        }
    }
}

/// Clipboard channel message from `Router` to the app loop.
#[derive(Debug, Clone, Copy)]
pub enum Action {
    Read(Request),
    Cancel,
}

/// `Effect::ReadClipboardImage`, for the app loop.
#[derive(Debug, Clone, Copy)]
pub struct Request {
    pub conversation: ConversationId,
    /// The computer's own clipboard may be read (D150).
    pub system: bool,
}

/// The app loop's side of pasting (D90, D91): pausing and resuming input,
/// the late-reply guard, cancelling on `Esc` or at exit.
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
        request: Request,
        attachments_dir: PathBuf,
        results: &mpsc::UnboundedSender<AppEvent>,
    ) {
        if self.cancel.is_some() {
            return;
        }
        *input = None;
        self.guard.arm();
        self.cancel = start(&mut self.session, request, attachments_dir, results.clone());
    }

    /// `Effect::CancelClipboardPaste`: abort an active clipboard read.
    pub fn cancel(&mut self) {
        if let Some(cancel) = &self.cancel {
            cancel.cancel_by_user();
        }
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

    /// Resume `EventStream` once the tty is released (`ClipboardStage`) or the
    /// read / drain finishes (`ClipboardImage` / `ClipboardDrained`).
    pub fn resume(&mut self, input: &mut Option<EventStream>, event: &AppEvent) {
        if matches!(event, AppEvent::ClipboardStage(_)) {
            if let Some(cancel) = &mut self.cancel {
                cancel.release_tty();
            }
            input.get_or_insert_with(EventStream::new);
        } else if matches!(
            event,
            AppEvent::ClipboardImage { .. } | AppEvent::ClipboardDrained
        ) {
            self.cancel = None;
            input.get_or_insert_with(EventStream::new);
        }
    }

    /// faro is quitting: stop waiting on the tty.
    pub fn stop(&mut self) {
        if let Some(mut cancel) = self.cancel.take() {
            cancel.shutdown();
        }
    }
}

struct LiveObserver {
    results: mpsc::UnboundedSender<AppEvent>,
}

impl Observer for LiveObserver {
    fn on_input(&mut self, event: Event) {
        let _ = self.results.send(AppEvent::Terminal(event));
    }

    fn on_progress(&mut self, bytes: u64) {
        let _ = self.results.send(AppEvent::ClipboardProgress { bytes });
    }

    fn on_midstream_cancel(&mut self) {
        let _ = self.results.send(AppEvent::ClipboardCancelled);
    }
}

/// Start reading the clipboard for the request's conversation.
pub fn start(
    session: &mut Session,
    request: Request,
    attachments_dir: PathBuf,
    results: mpsc::UnboundedSender<AppEvent>,
) -> Option<Cancel> {
    let Request {
        conversation,
        system,
    } = request;
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
    let cancel = Cancel::from_tty(tty.wake_handle());
    let cancelled = Arc::clone(&cancel.cancelled);
    let cancel_notify = Arc::clone(&cancel.notify);
    let ids = (session.next_id(), session.next_id());
    let credentials = session.credentials.clone();
    tokio::spawn(async move {
        let mut focus = Vec::new();
        let read = Read {
            ids,
            credentials,
            dir: attachments_dir,
            system,
            results: results.clone(),
            cancelled,
            cancel_notify,
        };
        let outcome = read.run(tty, &mut focus).await;
        send_focus(&results, focus);
        match outcome {
            RunOutcome::Finished(result) => finish(&results, result),
            RunOutcome::MidstreamDrained => {
                let _ = results.send(AppEvent::ClipboardDrained);
            }
        }
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
    let cancel = Cancel::from_tty(tty.wake_handle());
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

/// Unlink a staged clipboard file off the UI thread (`Effect::DiscardStagedImage`).
pub fn discard_file(path: PathBuf) {
    tokio::spawn(async move {
        let _ = tokio::task::spawn_blocking(move || remove_file_blocking(&path)).await;
    });
}

#[expect(
    clippy::disallowed_methods,
    reason = "runs inside spawn_blocking off the UI thread"
)]
fn remove_file_blocking(path: &Path) {
    let _ = std::fs::remove_file(path);
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

/// Run `work` on a blocking thread.
async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
) -> Result<T, ClipboardError> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|error| ClipboardError::Failed(error.to_string()))
}

enum RunOutcome {
    Finished(Result<StagedAttachment, ClipboardError>),
    MidstreamDrained,
}

enum StepError {
    Clipboard(ClipboardError),
    MidstreamDrained,
}

impl From<ClipboardError> for StepError {
    fn from(error: ClipboardError) -> Self {
        Self::Clipboard(error)
    }
}

/// One `Ctrl+V`.
struct Read {
    ids: (String, String),
    credentials: Credentials,
    dir: PathBuf,
    /// The computer's own clipboard may be read (D150).
    system: bool,
    results: mpsc::UnboundedSender<AppEvent>,
    cancelled: Arc<AtomicBool>,
    cancel_notify: Arc<Notify>,
}

/// Where the image comes from.
enum Route {
    /// The terminal shares its clipboard (OSC 5522).
    Terminal(Tty),
    /// faro reads the computer's clipboard itself.
    System,
}

impl Read {
    async fn run(self, tty: Tty, focus: &mut Vec<Focus>) -> RunOutcome {
        match self.run_inner(tty, focus).await {
            Ok(staged) => RunOutcome::Finished(Ok(staged)),
            Err(StepError::Clipboard(err)) => RunOutcome::Finished(Err(err)),
            Err(StepError::MidstreamDrained) => RunOutcome::MidstreamDrained,
        }
    }

    /// D150: mosh stops everything; kitty is asked as before; any other
    /// terminal is asked whether it knows OSC 5522; failing that, the
    /// computer's own clipboard when faro runs on it.
    async fn run_inner(
        self,
        tty: Tty,
        focus: &mut Vec<Focus>,
    ) -> Result<StagedAttachment, StepError> {
        if self.cancelled.load(Ordering::Acquire) {
            return Err(ClipboardError::Cancelled.into());
        }
        let terminal = super::system::inspect_terminal().await;
        if self.cancelled.load(Ordering::Acquire) {
            return Err(ClipboardError::Cancelled.into());
        }
        if terminal.mosh && !terminal.kitty {
            return Err(ClipboardError::Mosh.into());
        }
        let in_tmux = std::env::var_os("TMUX").is_some();
        blocking(tty::barrier)
            .await?
            .map_err(|error| ClipboardError::Failed(error.to_string()))?;
        let route = choose(tty, &terminal, in_tmux, self.system, &self.results, focus).await?;
        let (mime, data) = match route {
            Route::Terminal(tty) => self.via_terminal(tty, &terminal, in_tmux, focus).await?,
            Route::System => {
                // `tty` was dropped inside `choose`: resume `EventStream` before
                // running `wl-paste` / `xclip` / `osascript`.
                send_focus(&self.results, std::mem::take(focus));
                let _ = self
                    .results
                    .send(AppEvent::ClipboardStage(PasteStage::SystemRead));
                let image = self.read_system().await?;
                (image.mime.to_owned(), image.data)
            }
        };
        // `tty` is now gone: resume `EventStream` before TIFF→PNG conversion and disk I/O.
        send_focus(&self.results, std::mem::take(focus));
        let _ = self
            .results
            .send(AppEvent::ClipboardStage(PasteStage::Processing));
        if self.cancelled.load(Ordering::Acquire) {
            return Err(ClipboardError::Cancelled.into());
        }
        let dir = self.dir;
        let cancelled = self.cancelled;
        Ok(blocking(move || stage_cancellable(&dir, &mime, data, &cancelled)).await??)
    }

    async fn read_system(&self) -> Result<system::Image, ClipboardError> {
        if self.cancelled.load(Ordering::Acquire) {
            return Err(ClipboardError::Cancelled);
        }
        tokio::select! {
            biased;
            () = self.cancel_notify.notified() => Err(ClipboardError::Cancelled),
            res = system::read() => res,
        }
    }

    /// OSC 5522: the type list (no prompt), then the best image type.
    async fn via_terminal(
        &self,
        tty: Tty,
        terminal: &Terminal,
        in_tmux: bool,
        focus: &mut Vec<Focus>,
    ) -> Result<(String, Vec<u8>), StepError> {
        let (list_id, data_id) = self.ids.clone();
        write_request(&protocol::list_request(Some(&list_id)), in_tmux)?;
        let list_tx = self.results.clone();
        let (tty, listed) = blocking(move || {
            let mut tty = tty;
            let mut observer = LiveObserver { results: list_tx };
            let listed = tty::list_with(&mut tty, &list_id, &mut observer);
            (tty, listed)
        })
        .await?;
        focus.extend(listed.focus);
        let listed = listed.result.map_err(|failure| match failure {
            Failure::Cancelled { drained: true } => StepError::MidstreamDrained,
            Failure::Cancelled { drained: false } => ClipboardError::Cancelled.into(),
            Failure::Read(ReadError::NoReply) => ClipboardError::NoAnswer.into(),
            Failure::Read(error) => ClipboardError::Read(error).into(),
            Failure::Io(error) => ClipboardError::Failed(error.to_string()).into(),
        })?;
        let mime = protocol::best_image_type(&listed.types).ok_or(ClipboardError::NoImage)?;

        let _ = self.results.send(AppEvent::ClipboardPrompt(terminal.name));
        write_request(
            &protocol::read_request(&[mime], Some(&data_id), Some(&self.credentials)),
            in_tmux,
        )?;
        let data_tx = self.results.clone();
        let initial = listed.rest;
        let (_tty, read) = blocking(move || {
            let mut tty = tty;
            let mut observer = LiveObserver { results: data_tx };
            let read = tty::data_with(&mut tty, &data_id, limit_for(mime), &initial, &mut observer);
            (tty, read)
        })
        .await?;
        focus.extend(read.focus);
        let contents = read.result.map_err(|failure| match failure {
            Failure::Cancelled { drained: true } => StepError::MidstreamDrained,
            Failure::Cancelled { drained: false } => ClipboardError::Cancelled.into(),
            Failure::Read(ReadError::NoReply) => ClipboardError::PromptTimedOut.into(),
            Failure::Read(error) => ClipboardError::Read(error).into(),
            Failure::Io(error) => ClipboardError::Failed(error.to_string()).into(),
        })?;
        let entry = contents.take(mime).ok_or(ClipboardError::NoImage)?;
        Ok((entry.mime, entry.data))
    }
}

/// Which way to read (D150). kitty is asked without the question (older
/// kittys read the clipboard but don't know the mode).
async fn choose(
    tty: Tty,
    terminal: &Terminal,
    in_tmux: bool,
    system: bool,
    results: &mpsc::UnboundedSender<AppEvent>,
    focus: &mut Vec<Focus>,
) -> Result<Route, StepError> {
    if terminal.kitty {
        return Ok(Route::Terminal(tty));
    }
    write_request(protocol::SUPPORT_QUERY, in_tmux)?;
    let several = terminal.tmux_clients.len() > 1;
    let wait = if terminal.remote {
        SUPPORT_WAIT
    } else {
        LOCAL_SUPPORT_WAIT
    };
    let tx = results.clone();
    let (tty, asked) = blocking(move || {
        let mut tty = tty;
        let mut observer = LiveObserver { results: tx };
        let asked = tty::support_with(&mut tty, several, wait, &mut observer);
        (tty, asked)
    })
    .await?;
    focus.extend(asked.focus);
    let support = asked.result.map_err(|failure| match failure {
        Failure::Cancelled { drained: true } => StepError::MidstreamDrained,
        Failure::Cancelled { drained: false } => ClipboardError::Cancelled.into(),
        Failure::Read(_) | Failure::Io(_) => ClipboardError::Failed(format!("{failure:?}")).into(),
    })?;
    tracing::info!(?support, name = ?terminal.name, remote = terminal.remote, "asked the terminal about OSC 5522");
    if support.supported {
        Ok(Route::Terminal(tty))
    } else if terminal.remote {
        Err(if terminal.mosh {
            ClipboardError::Mosh.into()
        } else {
            ClipboardError::OverSsh.into()
        })
    } else if system {
        Ok(Route::System)
    } else {
        Err(ClipboardError::SystemOff.into())
    }
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
    let temp = faro_core::paths::sweep_temp_plaintext(dir);
    let Ok(entries) = std::fs::read_dir(dir) else {
        return temp;
    };
    temp + entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| is_pasted(path, dir))
        .filter(|path| std::fs::symlink_metadata(path).is_ok_and(|meta| meta.is_file()))
        .filter(|path| std::fs::remove_file(path).is_ok())
        .count()
}

/// The start of every pasted image's file name.
const PASTED_PREFIX: &str = "clipboard-";

/// Stage and clean up immediately if `cancelled` became set while converting/saving.
fn stage_cancellable(
    dir: &Path,
    mime: &str,
    data: Vec<u8>,
    cancelled: &AtomicBool,
) -> Result<StagedAttachment, ClipboardError> {
    if cancelled.load(Ordering::Acquire) {
        return Err(ClipboardError::Cancelled);
    }
    let staged = stage(dir, mime, data)?;
    if cancelled.load(Ordering::Acquire) {
        remove_file_blocking(&staged.path);
        return Err(ClipboardError::Cancelled);
    }
    Ok(staged)
}

/// Make the image sendable (TIFF becomes PNG, D150), then save it.
fn stage(dir: &Path, mime: &str, data: Vec<u8>) -> Result<StagedAttachment, ClipboardError> {
    let (mime, data) = sendable(mime, data)?;
    save(dir, mime, &data).map_err(ClipboardError::Failed)
}

/// TIFF is converted to PNG, which must fit [`MAX_IMAGE`]; other images are
/// taken as they are.
fn sendable(mime: &str, data: Vec<u8>) -> Result<(&str, Vec<u8>), ClipboardError> {
    if mime != protocol::TIFF {
        return Ok((mime, data));
    }
    let image = image::load_from_memory_with_format(&data, image::ImageFormat::Tiff)
        .map_err(|error| ClipboardError::Failed(format!("converting the TIFF image: {error}")))?;
    drop(data);
    let mut png = Vec::new();
    image
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .map_err(|error| ClipboardError::Failed(format!("converting the TIFF image: {error}")))?;
    if png.len() > MAX_IMAGE {
        return Err(ClipboardError::Read(ReadError::TooLarge {
            limit: MAX_IMAGE,
        }));
    }
    Ok(("image/png", png))
}

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
