//! Phase 0 spike for D89: who reads kitty's OSC 5522 clipboard reply?
//!
//! Two approaches, both driven from a crossterm `EventStream` set up like faro's
//! (raw mode, bracketed paste, focus reporting, mouse capture):
//!
//! * **A, pause:** drop the `EventStream`, read the tty directly (mio + raw
//!   `read(2)`) until the reply is complete, then make a new `EventStream`.
//! * **B, keys:** leave crossterm running and rebuild the reply from the key
//!   events it turns it into (`Alt+]`, one `Char` per byte, `Alt+\`).
//!
//! Keys: `1` A list types, `2` A read image, `3` B list types, `4` B read image,
//! `p` toggle the kitty password (`pw`/`name`), `h` help, `q`/`Ctrl+C` quit.
//! Anything else is echoed, so typing after a read shows that input still works.

mod keys;
mod reply;
mod tty;

use std::{
    fs::OpenOptions,
    io::{self, Write},
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};

use base64::{Engine, engine::general_purpose::STANDARD};
use crossterm::{
    event::{
        DisableBracketedPaste, DisableFocusChange, DisableMouseCapture, EnableBracketedPaste,
        EnableFocusChange, EnableMouseCapture, Event, EventStream, KeyCode, KeyEvent, KeyModifiers,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode},
};
use futures_util::StreamExt;
use tokio::time::{Instant as TokioInstant, sleep_until};

use crate::{
    keys::KeyCollector,
    reply::{Reply, Scanner},
};

/// Seconds the user gets to answer kitty's permission prompt (`--first-packet-secs`).
static FIRST_PACKET_SECS: AtomicU64 = AtomicU64::new(60);

pub fn first_packet() -> Duration {
    Duration::from_secs(FIRST_PACKET_SECS.load(Ordering::Relaxed))
}
/// Longest pause allowed between two packets once the reply has started.
pub const BETWEEN_PACKETS: Duration = Duration::from_secs(5);
/// Decoded size cap (same as faro's image auto-download limit).
pub const MAX_BYTES: usize = 25 * 1024 * 1024;
/// Image types in order of preference.
const IMAGE_TYPES: [&str; 4] = ["image/png", "image/jpeg", "image/gif", "image/webp"];

struct Options {
    log: Option<PathBuf>,
    out: PathBuf,
    password: bool,
    /// Drain late replies that start arriving as keys (on by default).
    safety_net: bool,
}

fn parse_args() -> Options {
    let mut options = Options {
        log: None,
        out: PathBuf::from("."),
        password: false,
        safety_net: true,
    };
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--log" => options.log = args.next().map(PathBuf::from),
            "--out" => options.out = args.next().map_or_else(|| ".".into(), PathBuf::from),
            "--password" => options.password = true,
            "--no-safety-net" => options.safety_net = false,
            "--first-packet-secs" => {
                if let Some(secs) = args.next().and_then(|s| s.parse().ok()) {
                    FIRST_PACKET_SECS.store(secs, Ordering::Relaxed);
                }
            }
            _ => {}
        }
    }
    options
}

/// Where lines go: the screen (raw mode, so `\r\n`) and an optional log file.
pub struct Log {
    file: Option<std::fs::File>,
}

impl Log {
    pub fn line(&mut self, text: &str) {
        let mut out = io::stdout();
        let _ = write!(out, "{text}\r\n");
        let _ = out.flush();
        if let Some(file) = &mut self.file {
            let _ = writeln!(file, "{text}");
            let _ = file.flush();
        }
    }
}

/// What a read is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Goal {
    List,
    Image,
}

/// A B-approach request waiting for its reply.
struct Pending {
    goal: Goal,
    /// `None` while listing types; the chosen type while reading data.
    mime: Option<String>,
    started: Instant,
    deadline: TokioInstant,
    scanner: Scanner,
    reply: Reply,
}

struct App {
    log: Log,
    options: Options,
    password: String,
    in_tmux: bool,
    reads: usize,
    /// Events the app saw (not swallowed by the B collector), by kind.
    keys_seen: usize,
    mouse_seen: usize,
    other_seen: usize,
}

impl App {
    fn request(&self, mimes: &str) -> String {
        let mut meta = String::from("type=read");
        if self.options.password {
            meta.push_str(&format!(
                ":pw={}:name={}",
                STANDARD.encode(&self.password),
                STANDARD.encode("faro-spike")
            ));
        }
        let sequence = format!("\x1b]5522;{meta};{}\x1b\\", STANDARD.encode(mimes));
        if self.in_tmux {
            wrap_tmux_passthrough(&sequence)
        } else {
            sequence
        }
    }

    fn counters(&self) -> String {
        format!(
            "keys_seen={} mouse_seen={} other_seen={}",
            self.keys_seen, self.mouse_seen, self.other_seen
        )
    }

    /// Report a finished read and save image data.
    fn finish(
        &mut self,
        approach: &str,
        goal: Goal,
        reply: &Reply,
        elapsed: Duration,
        extra: &str,
    ) {
        let types = reply.types();
        let mut file = String::new();
        if goal == Goal::Image
            && let Some((mime, bytes)) = reply.data.first()
            && !bytes.is_empty()
        {
            self.reads += 1;
            let ext = mime.rsplit('/').next().unwrap_or("bin");
            let path = self
                .options
                .out
                .join(format!("clipboard-{}.{ext}", self.reads));
            match std::fs::write(&path, bytes) {
                Ok(()) => file = path.display().to_string(),
                Err(error) => file = format!("(write failed: {error})"),
            }
        }
        self.log.line(&format!(
            "RESULT approach={approach} goal={goal:?} status={} types={types:?} mime={} bytes={} \
             packets={} corrupt={} ms={} {extra} file={file}",
            reply.status(),
            reply.data.first().map_or("-", |(m, _)| m.as_str()),
            reply.data.first().map_or(0, |(_, b)| b.len()),
            reply.packets,
            reply.corrupt,
            elapsed.as_millis(),
        ));
    }

    fn note_event(&mut self, event: &Event) {
        match event {
            Event::Key(key) => {
                self.keys_seen += 1;
                self.log.line(&format!("event: key {}", describe_key(key)));
            }
            Event::Mouse(_) => self.mouse_seen += 1,
            Event::Paste(text) => {
                self.other_seen += 1;
                self.log
                    .line(&format!("event: paste ({} bytes)", text.len()));
            }
            other => {
                self.other_seen += 1;
                self.log.line(&format!("event: {other:?}"));
            }
        }
    }
}

fn describe_key(key: &KeyEvent) -> String {
    if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT {
        format!("{:?}", key.code)
    } else {
        format!("{:?}+{:?}", key.modifiers, key.code)
    }
}

/// Same as faro's `tui::wrap_tmux_passthrough`.
fn wrap_tmux_passthrough(sequence: &str) -> String {
    format!("\x1bPtmux;{}\x1b\\", sequence.replace('\x1b', "\x1b\x1b"))
}

fn best_image_type(types: &[String]) -> Option<&'static str> {
    IMAGE_TYPES
        .into_iter()
        .find(|wanted| types.iter().any(|t| t == wanted))
}

fn random_password() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    format!("{:x}{:x}", nanos, std::process::id())
}

fn help(log: &mut Log) {
    log.line("keys: 1 A list | 2 A read image | 3 B list | 4 B read image | p password | h help | q quit");
}

fn main() -> io::Result<()> {
    let options = parse_args();
    let file = match &options.log {
        Some(path) => Some(OpenOptions::new().create(true).append(true).open(path)?),
        None => None,
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    enable_raw_mode()?;
    execute!(
        io::stdout(),
        EnableBracketedPaste,
        EnableFocusChange,
        EnableMouseCapture
    )?;
    let result = runtime.block_on(run(options, file));
    let _ = execute!(
        io::stdout(),
        DisableMouseCapture,
        DisableFocusChange,
        DisableBracketedPaste
    );
    let _ = disable_raw_mode();
    result
}

async fn run(options: Options, file: Option<std::fs::File>) -> io::Result<()> {
    let in_tmux = std::env::var_os("TMUX").is_some();
    let mut app = App {
        log: Log { file },
        password: random_password(),
        in_tmux,
        options,
        reads: 0,
        keys_seen: 0,
        mouse_seen: 0,
        other_seen: 0,
    };
    app.log.line(&format!(
        "clipboard-spike: TERM={} TMUX={} KITTY_WINDOW_ID={} pid={}",
        std::env::var("TERM").unwrap_or_default(),
        if in_tmux { "set" } else { "unset" },
        std::env::var("KITTY_WINDOW_ID").unwrap_or_default(),
        std::process::id(),
    ));
    help(&mut app.log);

    let mut stream = Some(EventStream::new());
    let mut collector = KeyCollector::default();
    let mut pending: Option<Pending> = None;
    loop {
        let deadline = pending.as_ref().map(|p| p.deadline);
        let Some(input) = stream.as_mut() else { break };
        let event = tokio::select! {
            event = input.next() => match event {
                Some(Ok(event)) => event,
                Some(Err(error)) => return Err(error),
                None => break,
            },
            () = sleep_until(deadline.unwrap_or_else(TokioInstant::now)), if deadline.is_some() => {
                if let Some(p) = pending.take() {
                    let mut reply = p.reply;
                    reply.timed_out = true;
                    let extra = format!("consumed_events={} {}", collector.take_consumed(), app.counters());
                    app.finish("B", p.goal, &reply, p.started.elapsed(), &extra);
                }
                continue;
            }
        };

        // B: while a request is out, the collector takes the reply's key events.
        let events = if pending.is_none() && app.options.safety_net {
            let fed = collector.feed(event);
            if collector.in_packet() || fed.packet.is_some() {
                // A reply nobody waits for (e.g. after a timeout): drain it.
                drop(stream.take());
                let drained = tokio::task::spawn_blocking(tty::drain_late).await;
                stream = Some(EventStream::new());
                collector.reset();
                let consumed = collector.take_consumed();
                match drained {
                    Ok(Ok(d)) => app.log.line(&format!(
                        "LATE reply drained: key_events={consumed} queued_events={} tty_bytes={} done={}",
                        d.queued_events, d.tty_bytes, d.done
                    )),
                    Ok(Err(error)) => app.log.line(&format!("LATE drain failed: {error}")),
                    Err(error) => app.log.line(&format!("LATE drain task failed: {error}")),
                }
            }
            fed.passthrough
        } else if pending.is_some() {
            let fed = collector.feed(event);
            if let (Some(bytes), Some(p)) = (fed.packet, pending.as_mut()) {
                for packet in p.scanner.push(&bytes) {
                    p.reply.absorb(packet, MAX_BYTES);
                }
                p.deadline = TokioInstant::now() + BETWEEN_PACKETS;
                if p.reply.finished()
                    && let Some(done) = pending.take()
                {
                    pending = on_b_finished(&mut app, &mut collector, done);
                }
            }
            fed.passthrough
        } else {
            vec![event]
        };

        for event in events {
            app.note_event(&event);
            let Event::Key(key) = event else { continue };
            match (key.code, key.modifiers) {
                (KeyCode::Char('q'), _) | (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                    app.log.line(&format!("quit: {}", app.counters()));
                    return Ok(());
                }
                (KeyCode::Char('h'), _) => help(&mut app.log),
                (KeyCode::Char('p'), _) => {
                    app.options.password = !app.options.password;
                    app.log.line(&format!("password: {}", app.options.password));
                }
                (KeyCode::Char(c @ ('1' | '2')), _) => {
                    let goal = if c == '1' { Goal::List } else { Goal::Image };
                    // A: give the tty back to crossterm only when the read is over.
                    drop(stream.take());
                    approach_a(&mut app, goal).await;
                    stream = Some(EventStream::new());
                }
                (KeyCode::Char(c @ ('3' | '4')), _) if pending.is_none() => {
                    let goal = if c == '3' { Goal::List } else { Goal::Image };
                    pending = Some(start_b(&mut app, goal, None));
                }
                _ => {}
            }
        }
    }
    Ok(())
}

fn start_b(app: &mut App, goal: Goal, mime: Option<String>) -> Pending {
    let request = app.request(mime.as_deref().unwrap_or("."));
    let mut out = io::stdout();
    let _ = out.write_all(request.as_bytes());
    let _ = out.flush();
    app.log.line(&format!(
        "B: asked for {}",
        mime.as_deref().unwrap_or("the type list")
    ));
    Pending {
        goal,
        mime,
        started: Instant::now(),
        deadline: TokioInstant::now() + first_packet(),
        scanner: Scanner::default(),
        reply: Reply::default(),
    }
}

/// A B reply is complete: report it, or go on from the type list to the image.
fn on_b_finished(app: &mut App, collector: &mut KeyCollector, done: Pending) -> Option<Pending> {
    let consumed = collector.take_consumed();
    let extra = format!(
        "consumed_events={consumed} stray_bytes={} {}",
        done.scanner.stray.len(),
        app.counters()
    );
    if done.goal == Goal::Image && done.mime.is_none() {
        let types = done.reply.types();
        app.finish("B", Goal::List, &done.reply, done.started.elapsed(), &extra);
        return match best_image_type(&types) {
            Some(mime) => Some(start_b(app, Goal::Image, Some(mime.to_owned()))),
            None => {
                app.log.line("B: no image on the clipboard");
                None
            }
        };
    }
    app.finish("B", done.goal, &done.reply, done.started.elapsed(), &extra);
    None
}

async fn approach_a(app: &mut App, goal: Goal) {
    let list = app.request(".");
    let started = Instant::now();
    let listed = tokio::task::spawn_blocking(move || tty::read_reply(&list, true)).await;
    let listed = match listed {
        Ok(Ok(read)) => read,
        Ok(Err(error)) => return app.log.line(&format!("A: tty read failed: {error}")),
        Err(error) => return app.log.line(&format!("A: task failed: {error}")),
    };
    let extra = format!("{} {}", listed.stats(), app.counters());
    app.finish("A", Goal::List, &listed.reply, started.elapsed(), &extra);
    if goal == Goal::List {
        return;
    }
    let Some(mime) = best_image_type(&listed.reply.types()) else {
        return app.log.line("A: no image on the clipboard");
    };
    let request = app.request(mime);
    app.log.line(&format!("A: asked for {mime}"));
    let started = Instant::now();
    let read = tokio::task::spawn_blocking(move || tty::read_reply(&request, false)).await;
    match read {
        Ok(Ok(read)) => {
            let extra = format!("{} {}", read.stats(), app.counters());
            app.finish("A", Goal::Image, &read.reply, started.elapsed(), &extra);
        }
        Ok(Err(error)) => app.log.line(&format!("A: tty read failed: {error}")),
        Err(error) => app.log.line(&format!("A: task failed: {error}")),
    }
}
