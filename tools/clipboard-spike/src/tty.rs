//! Approach A: with the `EventStream` dropped, read the reply from the tty
//! ourselves.
//!
//! Readiness comes from mio (epoll on Linux, kqueue on macOS, where `poll(2)`
//! doesn't work on ttys); crossterm uses the same. mio is edge-triggered and the
//! tty is a blocking fd (it's shared with stdout, so it must not be switched to
//! non-blocking), so after each wakeup we read exactly what `FIONREAD` says is
//! waiting: that drains the fd without ever blocking.

use std::{
    io::{self, Write},
    os::fd::{AsFd, AsRawFd},
    time::{Duration, Instant},
};

use mio::{Events, Interest, Poll, Token, unix::SourceFd};

use crate::{
    BETWEEN_PACKETS, MAX_BYTES, first_packet,
    reply::{Reply, Scanner},
};

pub struct DirectRead {
    pub reply: Reply,
    /// Bytes outside packets (keys typed during the read; lost to the app).
    pub stray: Vec<u8>,
    /// crossterm already had events queued when we took over.
    pub queued_before: bool,
    pub wakeups: usize,
    pub bytes_read: usize,
}

impl DirectRead {
    pub fn stats(&self) -> String {
        format!(
            "wakeups={} tty_bytes={} stray_bytes={} stray={:?} queued_before={}",
            self.wakeups,
            self.bytes_read,
            self.stray.len(),
            String::from_utf8_lossy(&self.stray),
            self.queued_before
        )
    }
}

pub fn read_reply(request: &str, _list: bool) -> io::Result<DirectRead> {
    // Barrier: the dropped EventStream's thread has been woken and won't poll
    // again; taking crossterm's reader lock once makes sure it has let go.
    // Anything this reads (keys typed just now) stays queued for the next stream.
    let queued_before = crossterm::event::poll(Duration::from_millis(20))?;

    let stdin = io::stdin();
    let fd = stdin.as_fd();
    if !rustix::termios::isatty(fd) {
        return Err(io::Error::other("stdin is not a tty"));
    }
    let raw = fd.as_raw_fd();
    let mut poll = Poll::new()?;
    poll.registry()
        .register(&mut SourceFd(&raw), Token(0), Interest::READABLE)?;

    let mut out = io::stdout();
    out.write_all(request.as_bytes())?;
    out.flush()?;

    let mut read = DirectRead {
        reply: Reply::default(),
        stray: Vec::new(),
        queued_before,
        wakeups: 0,
        bytes_read: 0,
    };
    let mut scanner = Scanner::default();
    let mut events = Events::with_capacity(4);
    let mut buf = vec![0u8; 64 * 1024];
    let mut deadline = Instant::now() + first_packet();
    'outer: loop {
        let now = Instant::now();
        let Some(left) = deadline
            .checked_duration_since(now)
            .filter(|d| !d.is_zero())
        else {
            read.reply.timed_out = true;
            break;
        };
        if let Err(error) = poll.poll(&mut events, Some(left)) {
            if error.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(error);
        }
        if events.is_empty() {
            continue;
        }
        read.wakeups += 1;
        loop {
            let waiting = rustix::io::ioctl_fionread(fd)? as usize;
            if waiting == 0 {
                break;
            }
            let want = waiting.min(buf.len());
            let n = rustix::io::read(fd, &mut buf[..want])?;
            if n == 0 {
                break 'outer;
            }
            read.bytes_read += n;
            for packet in scanner.push(&buf[..n]) {
                read.reply.absorb(packet, MAX_BYTES);
            }
            deadline = Instant::now() + BETWEEN_PACKETS;
            if read.reply.finished() {
                break 'outer;
            }
        }
    }
    let _ = poll.registry().deregister(&mut SourceFd(&raw));
    read.stray = scanner.stray;
    Ok(read)
}

/// What the late-reply drain threw away.
pub struct Drained {
    pub queued_events: usize,
    pub tty_bytes: usize,
    pub done: bool,
}

/// A reply we no longer wait for has started arriving as key events (the
/// `EventStream` is already dropped). Discard what crossterm has queued, then
/// read the rest straight from the tty until `DONE`/an error, or 1 s of quiet.
/// Without this, a large late reply wedges crossterm (see the spike report).
pub fn drain_late() -> io::Result<Drained> {
    let mut drained = Drained {
        queued_events: 0,
        tty_bytes: 0,
        done: false,
    };
    let mut collector = crate::keys::KeyCollector::default();
    let mut scanner = Scanner::default();
    let mut reply = Reply::default();
    // Seed the collector: it's inside a packet already.
    collector.feed(crossterm::event::Event::Key(
        crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char(']'),
            crossterm::event::KeyModifiers::ALT,
        ),
    ));
    // Barrier as in `read_reply`, then empty crossterm's queue.
    let mut more = crossterm::event::poll(Duration::from_millis(20))?;
    while more {
        let event = crossterm::event::read()?;
        drained.queued_events += 1;
        if let Some(packet) = collector.feed(event).packet {
            for p in scanner.push(&packet) {
                reply.absorb(p, MAX_BYTES);
            }
        }
        if reply.finished() {
            drained.done = true;
            return Ok(drained);
        }
        more = crossterm::event::poll(Duration::ZERO)?;
    }

    let stdin = io::stdin();
    let fd = stdin.as_fd();
    let raw = fd.as_raw_fd();
    let mut poll = Poll::new()?;
    poll.registry()
        .register(&mut SourceFd(&raw), Token(0), Interest::READABLE)?;
    let mut events = Events::with_capacity(4);
    let mut buf = vec![0u8; 64 * 1024];
    let mut deadline = Instant::now() + Duration::from_secs(1);
    // The tty may hold data with no new edge coming (that's the wedge), so
    // look before waiting.
    let mut first = true;
    'outer: loop {
        if !first {
            let Some(left) = deadline
                .checked_duration_since(Instant::now())
                .filter(|d| !d.is_zero())
            else {
                break;
            };
            if let Err(error) = poll.poll(&mut events, Some(left)) {
                if error.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(error);
            }
            if events.is_empty() {
                continue;
            }
        }
        first = false;
        loop {
            let waiting = rustix::io::ioctl_fionread(fd)? as usize;
            if waiting == 0 {
                break;
            }
            let n = rustix::io::read(fd, &mut buf[..waiting.min(64 * 1024)])?;
            if n == 0 {
                break 'outer;
            }
            drained.tty_bytes += n;
            // Mid-packet bytes land in `stray`; only complete packets count.
            for p in scanner.push(&buf[..n]) {
                reply.absorb(p, usize::MAX);
            }
            deadline = Instant::now() + Duration::from_secs(1);
            if reply.finished() {
                drained.done = true;
                break 'outer;
            }
        }
    }
    let _ = poll.registry().deregister(&mut SourceFd(&raw));
    Ok(drained)
}
