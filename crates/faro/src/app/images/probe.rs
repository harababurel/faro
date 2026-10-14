//! Asking the terminal about images in the middle of a session (D137): which
//! graphics protocol it speaks and how big its cells are, the same question
//! ratatui-image asks at startup (its own query string and reply parser),
//! read the way clipboard paste reads replies (D90): the app loop has
//! dropped its `EventStream`, [`tty::barrier`] waits for crossterm's reader
//! to let go, and the reply is read from the tty without blocking.

use std::io::Write as _;
use std::time::{Duration, Instant};

use ratatui_image::picker::cap_parser::{Parser, QueryStdioOptions, Response};

use super::super::clipboard::tty::{self, Source, Tty};
use crate::app::image_cache::{CellSize, ImageProtocol};

/// How long the terminal has to answer (it ends with the `ESC [ 0 n` every
/// terminal sends for the status report at the end). Real terminals answer
/// at once; this covers a slow SSH link, where an answer after faro stopped
/// listening would be read as typed keys.
const WAIT: Duration = Duration::from_secs(3);
/// After that, how long to keep reading for a second terminal's answer
/// (two kittys attached to one tmux session both answer), so it isn't taken
/// for keys either (as clipboard paste's `MORE_REPLIES`).
const MORE: Duration = Duration::from_millis(500);

/// What the terminal said.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Probe {
    /// `None`: no graphics protocol (half-blocks).
    pub protocol: Option<ImageProtocol>,
    pub cell: Option<CellSize>,
    /// Whether it answered at all.
    pub answered: bool,
    /// The last focus report that arrived meanwhile (`true`: gained), for
    /// the reducer (D42), as clipboard paste passes them on (D90).
    pub focus: Option<bool>,
}

/// Ask. The caller has dropped its `EventStream` and makes a new one after.
pub async fn ask() -> Probe {
    let tty = match Tty::open() {
        Ok(tty) => tty,
        Err(error) => {
            tracing::warn!(%error, "couldn't ask the terminal about images");
            return Probe::default();
        }
    };
    match tokio::task::spawn_blocking(tty::barrier).await {
        Ok(Ok(())) => {}
        Ok(Err(error)) => tracing::warn!(%error, "waiting for the terminal reader failed"),
        Err(error) => tracing::warn!(%error, "waiting for the terminal reader failed"),
    }
    // ratatui-image's own query, wrapped for tmux passthrough by itself.
    let in_tmux = std::env::var_os("TMUX").is_some();
    let query = Parser::query(in_tmux, QueryStdioOptions::default());
    if let Err(error) = write(&query) {
        tracing::warn!(%error, "couldn't ask the terminal about images");
        return Probe::default();
    }
    let probe = tokio::task::spawn_blocking(move || {
        let mut tty = tty;
        read(&mut tty)
    })
    .await
    .unwrap_or_default();
    tracing::info!(?probe, "asked the terminal about images");
    probe
}

/// Write the query (on the app loop's thread, between frames).
fn write(query: &str) -> std::io::Result<()> {
    let mut out = std::io::stdout().lock();
    out.write_all(query.as_bytes())?;
    out.flush()
}

/// Read the answer from `source` until its status report (and a little
/// after), or until it's too late.
fn read(source: &mut impl Source) -> Probe {
    let mut parser = Parser::new();
    let mut probe = Probe::default();
    let (mut kitty, mut sixel) = (false, false);
    let mut deadline = Instant::now() + WAIT;
    let mut done = false;
    loop {
        let bytes = match source.read(deadline) {
            Ok(Some(bytes)) => bytes,
            Ok(None) => break,
            Err(error) => {
                tracing::warn!(%error, "reading the terminal's answer failed");
                break;
            }
        };
        let text = String::from_utf8_lossy(&bytes);
        if let Some(focus) = last_focus(&text) {
            probe.focus = Some(focus);
        }
        for next in text.chars() {
            for response in parser.push(next) {
                match response {
                    Response::Kitty => kitty = true,
                    Response::Sixel => sixel = true,
                    Response::CellSize(Some((width, height))) => {
                        probe.cell.get_or_insert(CellSize { width, height });
                    }
                    Response::Status => {
                        probe.answered = true;
                        if !done {
                            done = true;
                            deadline = Instant::now() + MORE;
                        }
                    }
                    Response::CellSize(None)
                    | Response::RectangularOps
                    | Response::KittyCompression
                    | Response::CursorPositionReport(..)
                    | Response::Background(..) => {}
                }
            }
        }
    }
    probe.protocol = if kitty {
        Some(ImageProtocol::Kitty)
    } else if sixel {
        Some(ImageProtocol::Sixel)
    } else {
        None
    };
    probe
}

/// The last focus report (`ESC [ I` gained, `ESC [ O` lost) in `text`.
fn last_focus(text: &str) -> Option<bool> {
    let gained = text.rfind("\x1b[I");
    let lost = text.rfind("\x1b[O");
    match (gained, lost) {
        (Some(g), Some(l)) => Some(g > l),
        (Some(_), None) => Some(true),
        (None, Some(_)) => Some(false),
        (None, None) => None,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::io;

    use super::*;

    /// A terminal that answers with these chunks, then nothing.
    struct Script(VecDeque<&'static str>);

    impl Source for Script {
        fn read(&mut self, _deadline: Instant) -> io::Result<Option<Vec<u8>>> {
            Ok(self.0.pop_front().map(|chunk| chunk.as_bytes().to_vec()))
        }
    }

    #[test]
    fn kitty_and_its_cell_size_are_read_from_the_answer() {
        let mut kitty = Script(VecDeque::from([
            "\x1b_Gi=31;OK\x1b\\\x1b[?62;22",
            "c\x1b[6;38;18t\x1b[0n",
        ]));
        assert_eq!(
            read(&mut kitty),
            Probe {
                protocol: Some(ImageProtocol::Kitty),
                cell: Some(CellSize {
                    width: 18,
                    height: 38
                }),
                answered: true,
                focus: None,
            }
        );
        let mut plain = Script(VecDeque::from(["\x1b[?62;22c\x1b[0n"]));
        assert_eq!(
            read(&mut plain),
            Probe {
                protocol: None,
                cell: None,
                answered: true,
                focus: None,
            }
        );
        let mut refocused = Script(VecDeque::from(["\x1b[O\x1b[?62c\x1b[I\x1b[0n"]));
        assert_eq!(
            read(&mut refocused).focus,
            Some(true),
            "the last report wins"
        );
        let mut silent = Script(VecDeque::new());
        assert_eq!(read(&mut silent), Probe::default());
    }
}
