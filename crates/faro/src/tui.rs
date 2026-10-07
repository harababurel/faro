//! Terminal setup and teardown. The terminal is always restored: on drop, and
//! from a panic hook (otherwise a panic would leave the shell in raw mode with
//! mouse reporting on). That includes the window title (D49).

use std::io::{self, Stdout, Write as _, stdout};
use std::sync::Once;
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

pub struct Tui {
    terminal: Terminal<CrosstermBackend<Stdout>>,
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
        install_panic_hook();
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
                terminal: Terminal::new(CrosstermBackend::new(stdout()))?,
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
        let drawn = self.terminal.draw(render).map(drop);
        execute!(stdout(), EndSynchronizedUpdate)?;
        drawn
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

fn install_panic_hook() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            restore();
            previous(info);
        }));
    });
}
