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
use std::time::{Duration, Instant};

use faro_core::clipboard::{self as protocol, Contents, Focus, ReadError, Reader};
use mio::unix::SourceFd;
use mio::{Events, Interest, Poll, Token, Waker};

use super::{BETWEEN_PACKETS, LIST_LIMIT, LIST_WAIT, MORE_REPLIES, PROMPT_WAIT};

/// How long crossterm's reader gets to let go of the tty.
const BARRIER: Duration = Duration::from_millis(20);
/// A late reply is over once nothing has come for this long.
const DRAIN_QUIET: Duration = Duration::from_secs(1);
/// The most taken from the tty per read.
const CHUNK: usize = 64 * 1024;

const TTY: Token = Token(0);
const WAKE: Token = Token(1);

/// Where reply bytes come from.
pub trait Source {
    /// Whatever has arrived, waiting until `deadline` for something; `None`
    /// once the deadline passes with nothing.
    fn read(&mut self, deadline: Instant) -> io::Result<Option<Vec<u8>>>;
}

/// faro's terminal input (stdin).
pub struct Tty {
    poll: Poll,
    events: Events,
    waker: Arc<Waker>,
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
        })
    }

    pub fn waker(&self) -> Arc<Waker> {
        Arc::clone(&self.waker)
    }
}

impl Source for Tty {
    fn read(&mut self, deadline: Instant) -> io::Result<Option<Vec<u8>>> {
        loop {
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
                return Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled"));
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
    Read(ReadError),
    Io(io::Error),
}

/// What a phase got, and the focus reports found on the way.
#[derive(Debug)]
pub struct Outcome<T> {
    pub result: Result<T, Failure>,
    pub focus: Vec<Focus>,
}

/// The types on the clipboard. Every kitty attached to faro's tmux session
/// answers (D91): the first list with an image wins; after a list without
/// one, others get [`MORE_REPLIES`] to arrive. Lists are merged.
pub fn list(source: &mut impl Source, id: &str) -> Outcome<Vec<String>> {
    let mut deadline = Instant::now() + LIST_WAIT;
    let mut reader = Reader::new(LIST_LIMIT, Some(id));
    let mut focus = Vec::new();
    let mut types: Vec<String> = Vec::new();
    let mut answered = false;
    let mut first_error = None;
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
                break;
            }
            deadline = deadline.min(Instant::now() + MORE_REPLIES);
            reader.feed(&rest);
            continue;
        }
        match source.read(deadline) {
            Ok(Some(bytes)) => reader.feed(&bytes),
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
        Ok(types)
    } else {
        Err(Failure::Read(first_error.unwrap_or(ReadError::NoReply)))
    };
    Outcome { result, focus }
}

/// The data of one type: [`PROMPT_WAIT`] for the user to answer kitty's
/// prompt, then at most [`BETWEEN_PACKETS`] between packets. The first
/// complete reply wins.
pub fn data(source: &mut impl Source, id: &str, limit: usize) -> Outcome<Contents> {
    let mut deadline = Instant::now() + PROMPT_WAIT;
    let mut reader = Reader::new(limit, Some(id));
    while !reader.finished() {
        match source.read(deadline) {
            Ok(Some(bytes)) => {
                reader.feed(&bytes);
                if reader.started() {
                    deadline = Instant::now() + BETWEEN_PACKETS;
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
    let focus = reader.take_focus();
    Outcome {
        result: reader.finish().map_err(Failure::Read),
        focus,
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
