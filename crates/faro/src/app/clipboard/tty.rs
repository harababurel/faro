//! Reading OSC 5522 replies straight from the tty while crossterm is paused
//! (D90). Readiness comes from mio (epoll, or kqueue on macOS, where `poll(2)`
//! doesn't work on ttys), as in crossterm. mio is edge-triggered and the tty
//! is a blocking fd (shared with stdout, so never switched to non-blocking),
//! so each read takes exactly what `FIONREAD` says is waiting: the fd is
//! drained without ever blocking. The phases are written against [`Source`]
//! so tests can script the terminal.

use std::io;
use std::os::fd::AsRawFd;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::{Duration, Instant};

use faro_core::clipboard::{self as protocol, Contents, Focus, ReadError, Reader, SupportReader};
use mio::unix::SourceFd;
use mio::{Events, Interest, Poll, Token, Waker};

use super::keys::KeyDecoder;
use super::{BETWEEN_PACKETS, LIST_LIMIT, LIST_WAIT, MORE_REPLIES, PROMPT_WAIT};
use crate::app::term_event::Event;

/// How long crossterm's reader gets to let go of the tty.
const BARRIER: Duration = Duration::from_millis(20);
/// Idle gap after a lone `ESC` before treating it as a standalone `Esc` key.
const ESC_WAIT: Duration = Duration::from_millis(30);
/// Minimum interval between live `ClipboardProgress` updates.
const PROGRESS_INTERVAL: Duration = Duration::from_millis(80);
/// A late reply is over once nothing has come for this long.
const DRAIN_QUIET: Duration = Duration::from_secs(1);
/// The most taken from the tty per read.
const CHUNK: usize = 64 * 1024;

const TTY: Token = Token(0);
const WAKE: Token = Token(1);

const WAKE_NONE: u8 = 0;
const WAKE_USER_CANCEL: u8 = 1;
const WAKE_SHUTDOWN: u8 = 2;

/// Where reply bytes come from.
pub trait Source {
    /// Whatever has arrived, waiting until `deadline` for something; `None`
    /// once the deadline passes with nothing.
    fn read(&mut self, deadline: Instant) -> io::Result<Option<Vec<u8>>>;
}

/// Live events observed while a phase reads from [`Source`].
pub trait Observer {
    /// A user key or bracketed paste arrived outside `OSC 5522` packets.
    fn on_input(&mut self, _event: Event) {}
    /// Decoded `OSC 5522` image payload bytes received so far.
    fn on_progress(&mut self, _bytes: u64) {}
    /// User cancelled (`Esc` / `Ctrl+C`) after packets had already started,
    /// so the UI can clear at once while [`data_with`] drains to `DONE`.
    fn on_midstream_cancel(&mut self) {}
}

#[cfg(test)]
impl Observer for () {}

/// Handle that wakes a blocking [`Tty`] read for user cancellation or shutdown.
#[derive(Clone)]
pub struct TtyWake {
    waker: Arc<Waker>,
    reason: Arc<AtomicU8>,
}

impl TtyWake {
    pub fn cancel_by_user(&self) {
        let _ = self.reason.compare_exchange(
            WAKE_NONE,
            WAKE_USER_CANCEL,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
        let _ = self.waker.wake();
    }

    pub fn shutdown(&self) {
        self.reason.store(WAKE_SHUTDOWN, Ordering::Release);
        let _ = self.waker.wake();
    }
}

/// faro's terminal input (stdin).
pub struct Tty {
    poll: Poll,
    events: Events,
    waker: Arc<Waker>,
    wake_reason: Arc<AtomicU8>,
}

impl Tty {
    pub fn open() -> io::Result<Self> {
        let fd = rustix::stdio::stdin();
        if !rustix::termios::isatty(fd) {
            return Err(io::Error::other("standard input isn't a terminal"));
        }
        let poll = Poll::new()?;
        poll.registry()
            .register(&mut SourceFd(&fd.as_raw_fd()), TTY, Interest::READABLE)?;
        let waker = Arc::new(Waker::new(poll.registry(), WAKE)?);
        Ok(Self {
            poll,
            events: Events::with_capacity(8),
            waker,
            wake_reason: Arc::new(AtomicU8::new(WAKE_NONE)),
        })
    }

    pub fn wake_handle(&self) -> TtyWake {
        TtyWake {
            waker: Arc::clone(&self.waker),
            reason: Arc::clone(&self.wake_reason),
        }
    }
}

impl Source for Tty {
    fn read(&mut self, deadline: Instant) -> io::Result<Option<Vec<u8>>> {
        loop {
            match self.wake_reason.swap(WAKE_NONE, Ordering::AcqRel) {
                WAKE_SHUTDOWN => {
                    return Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled"));
                }
                WAKE_USER_CANCEL => return Ok(Some(vec![0x03])),
                _ => {}
            }
            // Look before waiting: bytes may be there with no new edge coming.
            let bytes = read_waiting()?;
            if !bytes.is_empty() {
                return Ok(Some(bytes));
            }
            let Some(left) = deadline
                .checked_duration_since(Instant::now())
                .filter(|left| !left.is_zero())
            else {
                return Ok(None);
            };
            match self.poll.poll(&mut self.events, Some(left)) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error),
            }
            if self.events.iter().any(|event| event.token() == WAKE) {
                match self.wake_reason.swap(WAKE_NONE, Ordering::AcqRel) {
                    WAKE_USER_CANCEL => return Ok(Some(vec![0x03])),
                    _ => return Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled")),
                }
            }
        }
    }
}

/// Everything waiting on stdin, without blocking.
fn read_waiting() -> io::Result<Vec<u8>> {
    let fd = rustix::stdio::stdin();
    let mut bytes = Vec::new();
    loop {
        let waiting = usize::try_from(rustix::io::ioctl_fionread(fd)?).unwrap_or(CHUNK);
        if waiting == 0 || bytes.len() >= CHUNK {
            return Ok(bytes);
        }
        let start = bytes.len();
        bytes.resize(start + waiting.min(CHUNK), 0);
        let read = match bytes.get_mut(start..) {
            Some(room) => rustix::io::read(fd, room)?,
            None => 0,
        };
        bytes.truncate(start + read);
        if read == 0 {
            return Ok(bytes);
        }
    }
}

/// The dropped `EventStream`'s thread has been woken and won't poll again;
/// taking crossterm's reader lock once makes sure it has let go. Keys this
/// reads stay queued for the next stream.
pub fn barrier() -> io::Result<()> {
    crossterm::event::poll(BARRIER).map(drop)
}

/// Why a phase got nothing.
#[derive(Debug)]
pub enum Failure {
    /// Cancelled by the user (`Esc` or `Ctrl+C`). `drained` is `true` when
    /// packets had already started and [`data_with`] drained the rest to `DONE`.
    Cancelled {
        drained: bool,
    },
    Read(ReadError),
    Io(io::Error),
}

/// What a phase got, and the focus reports found on the way.
#[derive(Debug)]
pub struct Outcome<T> {
    pub result: Result<T, Failure>,
    pub focus: Vec<Focus>,
}

/// Forward decoded user input events; returns `true` when cancelled.
fn forward_input(raw: &[u8], decoder: &mut KeyDecoder, observer: &mut impl Observer) -> bool {
    if raw.is_empty() {
        return false;
    }
    let batch = decoder.feed(raw);
    for event in batch.events {
        observer.on_input(event);
    }
    batch.cancelled
}

/// The answers to [`protocol::SUPPORT_QUERY`] (D150).
#[cfg(test)]
pub fn support(source: &mut impl Source, several: bool) -> Outcome<Support> {
    support_with(source, several, super::SUPPORT_WAIT, &mut ())
}

/// [`support`] with a caller-chosen timeout and live [`Observer`].
pub fn support_with(
    source: &mut impl Source,
    several: bool,
    wait_limit: Duration,
    observer: &mut impl Observer,
) -> Outcome<Support> {
    let mut deadline = Instant::now() + wait_limit;
    let mut reader = SupportReader::default();
    let mut decoder = KeyDecoder::default();
    loop {
        if reader.answered() > 0 {
            if reader.supported() || !several {
                break;
            }
            deadline = deadline.min(Instant::now() + MORE_REPLIES);
        }
        let pending_esc = reader.has_pending_esc();
        let wait = if pending_esc {
            deadline.min(Instant::now() + ESC_WAIT)
        } else {
            deadline
        };
        match source.read(wait) {
            Ok(Some(bytes)) => {
                reader.feed(&bytes);
                if forward_input(&reader.take_input(), &mut decoder, observer) {
                    return Outcome {
                        result: Err(Failure::Cancelled { drained: false }),
                        focus: reader.take_focus(),
                    };
                }
            }
            Ok(None) if pending_esc => {
                reader.flush_esc();
                if forward_input(&reader.take_input(), &mut decoder, observer) {
                    return Outcome {
                        result: Err(Failure::Cancelled { drained: false }),
                        focus: reader.take_focus(),
                    };
                }
            }
            Ok(None) => break,
            Err(error) => {
                return Outcome {
                    result: Err(Failure::Io(error)),
                    focus: reader.take_focus(),
                };
            }
        }
    }
    Outcome {
        result: Ok(Support {
            answered: reader.answered() > 0,
            supported: reader.supported(),
        }),
        focus: reader.take_focus(),
    }
}

/// What the terminal said about OSC 5522.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Support {
    pub answered: bool,
    pub supported: bool,
}

/// Listed clipboard types plus any unread bytes after `DONE`.
#[derive(Debug)]
pub struct Listed {
    pub types: Vec<String>,
    pub rest: Vec<u8>,
}

/// The types on the clipboard (D91).
#[cfg(test)]
pub fn list(source: &mut impl Source, id: &str) -> Outcome<Vec<String>> {
    let outcome = list_with(source, id, &mut ());
    Outcome {
        result: outcome.result.map(|listed| listed.types),
        focus: outcome.focus,
    }
}

/// [`list`] with a live [`Observer`] and trailing bytes preserved for [`data_with`].
pub fn list_with(
    source: &mut impl Source,
    id: &str,
    observer: &mut impl Observer,
) -> Outcome<Listed> {
    let mut deadline = Instant::now() + LIST_WAIT;
    let mut reader = Reader::new(LIST_LIMIT, Some(id));
    let mut decoder = KeyDecoder::default();
    let mut focus = Vec::new();
    let mut types: Vec<String> = Vec::new();
    let mut answered = false;
    let mut first_error = None;
    let mut trailing = Vec::new();

    loop {
        if reader.finished() {
            let rest = reader.take_rest();
            focus.extend(reader.take_focus());
            let done = std::mem::replace(&mut reader, Reader::new(LIST_LIMIT, Some(id)));
            match done.finish() {
                Ok(contents) => {
                    answered = true;
                    for found in contents.types() {
                        if !types.contains(&found) {
                            types.push(found);
                        }
                    }
                }
                Err(error) => {
                    first_error.get_or_insert(error);
                }
            }
            if protocol::best_image_type(&types).is_some() {
                trailing = rest;
                break;
            }
            deadline = deadline.min(Instant::now() + MORE_REPLIES);
            reader.feed(&rest);
            if forward_input(&reader.take_input(), &mut decoder, observer) {
                focus.extend(reader.take_focus());
                return Outcome {
                    result: Err(Failure::Cancelled { drained: false }),
                    focus,
                };
            }
            continue;
        }
        let pending_esc = reader.has_pending_esc();
        let wait = if pending_esc {
            deadline.min(Instant::now() + ESC_WAIT)
        } else {
            deadline
        };
        match source.read(wait) {
            Ok(Some(bytes)) => {
                reader.feed(&bytes);
                if forward_input(&reader.take_input(), &mut decoder, observer) {
                    focus.extend(reader.take_focus());
                    return Outcome {
                        result: Err(Failure::Cancelled { drained: false }),
                        focus,
                    };
                }
            }
            Ok(None) if pending_esc => {
                reader.flush_esc();
                if forward_input(&reader.take_input(), &mut decoder, observer) {
                    focus.extend(reader.take_focus());
                    return Outcome {
                        result: Err(Failure::Cancelled { drained: false }),
                        focus,
                    };
                }
            }
            Ok(None) => break,
            Err(error) => {
                focus.extend(reader.take_focus());
                return Outcome {
                    result: Err(Failure::Io(error)),
                    focus,
                };
            }
        }
    }
    focus.extend(reader.take_focus());
    let result = if answered {
        Ok(Listed {
            types,
            rest: trailing,
        })
    } else {
        Err(Failure::Read(first_error.unwrap_or(ReadError::NoReply)))
    };
    Outcome { result, focus }
}

/// The data of one type: [`PROMPT_WAIT`] for the user to answer kitty's
/// prompt, then at most [`BETWEEN_PACKETS`] between packets.
#[cfg(test)]
pub fn data(source: &mut impl Source, id: &str, limit: usize) -> Outcome<Contents> {
    data_with(source, id, limit, &[], &mut ())
}

/// [`data`] with initial trailing bytes from [`list_with`] and a live [`Observer`].
pub fn data_with(
    source: &mut impl Source,
    id: &str,
    limit: usize,
    initial: &[u8],
    observer: &mut impl Observer,
) -> Outcome<Contents> {
    let mut deadline = Instant::now() + PROMPT_WAIT;
    let mut reader = Reader::new(limit, Some(id));
    let mut decoder = KeyDecoder::default();
    let mut progress = ProgressTracker::default();

    if !initial.is_empty() {
        reader.feed(initial);
        if forward_input(&reader.take_input(), &mut decoder, observer) {
            return cancel_data(source, reader, &mut decoder, observer);
        }
        progress.report(reader.stored(), reader.finished(), observer);
        if reader.started() {
            deadline = Instant::now() + BETWEEN_PACKETS;
        }
    }

    while !reader.finished() {
        let pending_esc = reader.has_pending_esc();
        let wait = if pending_esc {
            deadline.min(Instant::now() + ESC_WAIT)
        } else {
            deadline
        };
        match source.read(wait) {
            Ok(Some(bytes)) => {
                reader.feed(&bytes);
                if forward_input(&reader.take_input(), &mut decoder, observer) {
                    return cancel_data(source, reader, &mut decoder, observer);
                }
                progress.report(reader.stored(), reader.finished(), observer);
                if reader.started() {
                    deadline = Instant::now() + BETWEEN_PACKETS;
                }
            }
            Ok(None) if pending_esc => {
                reader.flush_esc();
                if forward_input(&reader.take_input(), &mut decoder, observer) {
                    return cancel_data(source, reader, &mut decoder, observer);
                }
            }
            Ok(None) => break,
            Err(error) => {
                return Outcome {
                    result: Err(Failure::Io(error)),
                    focus: reader.take_focus(),
                };
            }
        }
    }

    progress.report(reader.stored(), true, observer);
    let mut focus = reader.take_focus();
    let rest = reader.take_rest();
    if !rest.is_empty() {
        let mut tail = Reader::new(0, None);
        tail.feed(&rest);
        focus.extend(tail.take_focus());
        if forward_input(&tail.take_input(), &mut decoder, observer) {
            return Outcome {
                result: Err(Failure::Cancelled { drained: false }),
                focus,
            };
        }
    }
    Outcome {
        result: reader.finish().map_err(Failure::Read),
        focus,
    }
}

#[derive(Default)]
struct ProgressTracker {
    reported: usize,
    last_at: Option<Instant>,
}

impl ProgressTracker {
    fn report(&mut self, stored: usize, force: bool, observer: &mut impl Observer) {
        if stored <= self.reported {
            return;
        }
        let now = Instant::now();
        let due = force
            || self
                .last_at
                .is_none_or(|prev| now.saturating_duration_since(prev) >= PROGRESS_INTERVAL);
        if due {
            self.reported = stored;
            self.last_at = Some(now);
            observer.on_progress(u64::try_from(stored).unwrap_or(u64::MAX));
        }
    }
}

/// Handle `Esc` / `Ctrl+C` during [`data_with`]: stop immediately if no reply
/// is mid-stream, or notify the UI at once and drain remaining packets to `DONE`.
fn cancel_data(
    source: &mut impl Source,
    mut reader: Reader,
    decoder: &mut KeyDecoder,
    observer: &mut impl Observer,
) -> Outcome<Contents> {
    if reader.finished() || (!reader.started() && !reader.has_partial_packet()) {
        return Outcome {
            result: Err(Failure::Cancelled { drained: false }),
            focus: reader.take_focus(),
        };
    }
    observer.on_midstream_cancel();
    reader.discard_data();
    let mut deadline = Instant::now() + DRAIN_QUIET;
    while !reader.finished() {
        match source.read(deadline) {
            Ok(Some(bytes)) => {
                reader.feed(&bytes);
                let _ = forward_input(&reader.take_input(), decoder, observer);
                deadline = Instant::now() + DRAIN_QUIET;
            }
            Ok(None) | Err(_) => break,
        }
    }
    Outcome {
        result: Err(Failure::Cancelled { drained: true }),
        focus: reader.take_focus(),
    }
}

/// What draining a late reply threw away.
#[derive(Debug, Default)]
pub struct Drained {
    pub bytes: usize,
    pub done: bool,
    pub focus: Vec<Focus>,
}

/// A late reply has started arriving as keys and the `EventStream` is gone:
/// throw away what crossterm has queued (keeping focus reports), then read
/// the rest from the tty until its end, or [`DRAIN_QUIET`] of nothing.
/// Without this, a large late reply wedges crossterm (D90).
pub fn drain(mut tty: Tty) -> io::Result<Drained> {
    let mut focus = Vec::new();
    let mut more = crossterm::event::poll(BARRIER)?;
    while more {
        match crossterm::event::read()? {
            crossterm::event::Event::FocusGained => focus.push(Focus::Gained),
            crossterm::event::Event::FocusLost => focus.push(Focus::Lost),
            crossterm::event::Event::Key(_)
            | crossterm::event::Event::Mouse(_)
            | crossterm::event::Event::Paste(_)
            | crossterm::event::Event::Resize(..) => {}
        }
        more = crossterm::event::poll(Duration::ZERO)?;
    }
    let mut drained = drain_source(&mut tty)?;
    focus.append(&mut drained.focus);
    drained.focus = focus;
    Ok(drained)
}

/// The tty part of [`drain`]. The reply may start mid-packet; that part is
/// dropped like any stray bytes. Nothing is kept (a limit of 0).
pub fn drain_source(source: &mut impl Source) -> io::Result<Drained> {
    let mut reader = Reader::new(0, None);
    let mut drained = Drained::default();
    let mut deadline = Instant::now() + DRAIN_QUIET;
    while !reader.finished() {
        let Some(bytes) = source.read(deadline)? else {
            break;
        };
        drained.bytes += bytes.len();
        reader.feed(&bytes);
        deadline = Instant::now() + DRAIN_QUIET;
    }
    drained.done = reader.finished();
    drained.focus = reader.take_focus();
    Ok(drained)
}
