//! Terminal setup and teardown. The terminal is always restored: on drop, and
//! from the panic hook in `crash.rs` (otherwise a panic would leave the shell
//! in raw mode with mouse reporting on). That includes the window title (D49).

use std::io::{self, Stdout, Write as _, stdout};
use std::sync::atomic::{AtomicBool, Ordering};

use crossterm::event::{
    DisableBracketedPaste, DisableFocusChange, DisableMouseCapture, EnableBracketedPaste,
    EnableFocusChange, EnableMouseCapture,
};
use crossterm::terminal::{
    BeginSynchronizedUpdate, EndSynchronizedUpdate, EnterAlternateScreen, LeaveAlternateScreen,
    SetTitle, disable_raw_mode, enable_raw_mode,
};
use crossterm::{cursor, execute};
use ratatui::backend::{Backend as _, ClearType, CrosstermBackend};
use ratatui::{Frame, Terminal};

use crate::frames::FrameLog;

mod backend;
pub mod input;

pub struct Tui {
    terminal: Terminal<backend::Backend<CrosstermBackend<Stdout>>>,
    /// Drawn frames go here when `FARO_DEBUG_FRAMES` is set (D55).
    frames: Option<FrameLog>,
    /// Mouse capture is on (`[mouse] enabled`).
    mouse: bool,
    /// The window title we set; `None` while it's the terminal's own.
    title: Option<String>,
}

/// XTWINOPS: save the window and icon titles on the terminal's title stack,
/// and restore them. Terminals without a stack ignore both.
const PUSH_TITLE: &[u8] = b"\x1b[22;0t";
const POP_TITLE: &[u8] = b"\x1b[23;0t";

/// We changed the title and haven't given it back yet (also read by the
/// panic hook).
static TITLED: AtomicBool = AtomicBool::new(false);

impl Tui {
    /// Raw mode, alternate screen, mouse capture if `mouse` (DESIGN.md §7.5).
    pub fn enter(mouse: bool) -> io::Result<Self> {
        crate::crash::install(restore);
        let setup = || -> io::Result<Self> {
            enable_raw_mode()?;
            // Bracketed paste: a multi-line paste arrives as one event, not as
            // keystrokes (where each newline would be Enter).
            // Focus reporting: messages count as read only while the terminal
            // has focus (D42). Under tmux this needs `focus-events on`.
            execute!(
                stdout(),
                EnterAlternateScreen,
                EnableBracketedPaste,
                EnableFocusChange
            )?;
            let mut tui = Self {
                terminal: Terminal::new(backend::Backend::new(CrosstermBackend::new(stdout())))?,
                frames: None,
                mouse: false,
                title: None,
            };
            tui.set_mouse(mouse)?;
            tui.clear()?;
            Ok(tui)
        };
        setup().inspect_err(|_| restore())
    }

    /// Draw a frame as one synchronized update (DEC mode 2026), so terminals that
    /// support it never show a half-drawn frame. Others ignore the sequence.
    pub fn draw(&mut self, render: impl FnOnce(&mut Frame<'_>)) -> io::Result<()> {
        execute!(stdout(), BeginSynchronizedUpdate)?;
        let drawn = self.terminal.draw(render).map(|frame| {
            if let Some(log) = &mut self.frames {
                log.record(frame.buffer);
            }
        });
        execute!(stdout(), EndSynchronizedUpdate)?;
        drawn
    }

    /// Log every frame drawn from now on (`FARO_DEBUG_FRAMES`, D55).
    pub fn log_frames(&mut self, log: FrameLog) {
        self.frames = Some(log);
    }
}

impl Tui {
    pub fn mouse(&self) -> bool {
        self.mouse
    }

    /// Turn mouse capture on or off. Off, the terminal handles the mouse
    /// itself (selecting text, its own scrollback).
    pub fn set_mouse(&mut self, on: bool) -> io::Result<()> {
        if on {
            execute!(stdout(), EnableMouseCapture)?;
        } else {
            execute!(stdout(), DisableMouseCapture)?;
        }
        self.mouse = on;
        Ok(())
    }

    /// Set the window title (OSC 0; tmux shows it as the pane title), or give
    /// the terminal's own back (`None`). Writes only when it changes. The
    /// terminal's title is saved on its title stack before the first change.
    pub fn set_title(&mut self, title: Option<&str>) -> io::Result<()> {
        if self.title.as_deref() == title {
            return Ok(());
        }
        match title {
            Some(text) => {
                let mut out = stdout();
                if !TITLED.swap(true, Ordering::Relaxed) {
                    out.write_all(PUSH_TITLE)?;
                }
                execute!(out, SetTitle(text))?;
            }
            None => restore_title()?,
        }
        self.title = title.map(str::to_owned);
        Ok(())
    }

    /// Clear the screen; the next draw paints every cell again (fixes a
    /// terminal whose picture went out of step, e.g. after an emoji mode
    /// change under mosh).
    ///
    /// Not `Terminal::clear`: that asks the terminal for the cursor position
    /// and waits for the answer, which the event reader swallows, so it would
    /// block the UI for two seconds and then fail.
    pub fn clear(&mut self) -> io::Result<()> {
        self.terminal.backend_mut().clear_region(ClearType::All)?;
        // Forget the last frame (the other buffer is already blank after a
        // draw), so the next draw sends every cell.
        self.terminal.swap_buffers();
        Ok(())
    }

    /// Send a desktop notification through the terminal using OSC 99 (kitty)
    /// or OSC 777 (`WezTerm`, Foot, VTE, rxvt-unicode), wrapped in tmux DCS
    /// passthrough when `$TMUX` is set.
    pub fn notify(title: &str, body: &str, kitty: bool) -> io::Result<()> {
        let in_tmux = std::env::var_os("TMUX").is_some();
        let payload = format_osc_notification(title, body, kitty, in_tmux);
        let mut out = stdout();
        out.write_all(payload.as_bytes())?;
        out.flush()
    }
}

/// Format an OSC 99 (kitty) or OSC 777 desktop notification sequence, wrapped
/// in tmux DCS passthrough (`\x1bPtmux;...\x1b\\`) when `in_tmux` is true.
pub fn format_osc_notification(title: &str, body: &str, kitty: bool, in_tmux: bool) -> String {
    let clean_title = sanitize_osc_field(title);
    let clean_body = sanitize_osc_field(body);
    if kitty {
        let first = format!("\x1b]99;i=faro:d=0:p=title;{clean_title}\x1b\\");
        let second = format!("\x1b]99;i=faro:d=1:p=body;{clean_body}\x1b\\");
        if in_tmux {
            format!(
                "{}{}",
                wrap_tmux_passthrough(&first),
                wrap_tmux_passthrough(&second)
            )
        } else {
            format!("{first}{second}")
        }
    } else {
        let seq = format!("\x1b]777;notify;{clean_title};{clean_body}\x1b\\");
        if in_tmux {
            wrap_tmux_passthrough(&seq)
        } else {
            seq
        }
    }
}

/// Strip control characters (`\x1b`, `\x07`, newlines…) and OSC delimiters
/// (`;`, `\`) before embedding a string inside an OSC notification sequence.
pub fn sanitize_osc_field(text: &str) -> String {
    text.chars()
        .filter(|&c| !c.is_control() && c != ';' && c != '\\')
        .collect()
}

/// Wrap an escape sequence in tmux DCS passthrough (`\x1bPtmux;...\x1b\\`),
/// doubling every inner `ESC` byte (`\x1b` → `\x1b\x1b`).
pub fn wrap_tmux_passthrough(sequence: &str) -> String {
    format!("\x1bPtmux;{}\x1b\\", sequence.replace('\x1b', "\x1b\x1b"))
}

impl Drop for Tui {
    fn drop(&mut self) {
        restore();
    }
}

/// Give the window title back: restore the saved one, after blanking ours
/// for terminals without a title stack (a blank title beats a stale count).
fn restore_title() -> io::Result<()> {
    if TITLED.swap(false, Ordering::Relaxed) {
        let mut out = stdout();
        execute!(out, SetTitle(""))?;
        out.write_all(POP_TITLE)?;
        out.flush()?;
    }
    Ok(())
}

/// Best effort: undo everything `enter` did. Safe to call more than once.
fn restore() {
    // Nothing useful can be done if these fail while exiting or panicking.
    let _ = restore_title();
    let _ = disable_raw_mode();
    let _ = execute!(
        stdout(),
        DisableBracketedPaste,
        DisableFocusChange,
        DisableMouseCapture,
        LeaveAlternateScreen,
        cursor::Show
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_osc_99_for_kitty_and_osc_777_elsewhere() {
        assert_eq!(
            format_osc_notification("faro", "New message from Alice", true, false),
            "\x1b]99;i=faro:d=0:p=title;faro\x1b\\\x1b]99;i=faro:d=1:p=body;New message from Alice\x1b\\"
        );
        assert_eq!(
            format_osc_notification("faro", "New message from Alice", false, false),
            "\x1b]777;notify;faro;New message from Alice\x1b\\"
        );
    }

    #[test]
    fn wraps_osc_notifications_in_tmux_dcs_passthrough() {
        assert_eq!(
            format_osc_notification("faro", "Hi", false, true),
            "\x1bPtmux;\x1b\x1b]777;notify;faro;Hi\x1b\x1b\\\x1b\\"
        );
        assert_eq!(
            format_osc_notification("faro", "Hi", true, true),
            "\x1bPtmux;\x1b\x1b]99;i=faro:d=0:p=title;faro\x1b\x1b\\\x1b\\\x1bPtmux;\x1b\x1b]99;i=faro:d=1:p=body;Hi\x1b\x1b\\\x1b\\"
        );
    }

    #[test]
    fn sanitizes_control_chars_and_osc_delimiters_in_fields() {
        assert_eq!(
            sanitize_osc_field("Alice\x1b]52;c;evil\x07;\\test\n"),
            "Alice]52ceviltest"
        );
    }
}
