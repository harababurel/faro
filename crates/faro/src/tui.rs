//! Terminal setup and teardown. The terminal is always restored: on drop, and
//! from a panic hook (otherwise a panic would leave the shell in raw mode with
//! mouse reporting on).

use std::io::{self, Stdout, stdout};
use std::sync::Once;

use crossterm::event::{
    DisableBracketedPaste, DisableFocusChange, DisableMouseCapture, EnableBracketedPaste,
    EnableFocusChange, EnableMouseCapture,
};
use crossterm::terminal::{
    BeginSynchronizedUpdate, EndSynchronizedUpdate, EnterAlternateScreen, LeaveAlternateScreen,
    disable_raw_mode, enable_raw_mode,
};
use crossterm::{cursor, execute};
use ratatui::backend::{Backend as _, ClearType, CrosstermBackend};
use ratatui::{Frame, Terminal};

pub struct Tui {
    terminal: Terminal<CrosstermBackend<Stdout>>,
}

impl Tui {
    /// Raw mode, alternate screen, mouse capture (DESIGN.md §7.5).
    pub fn enter() -> io::Result<Self> {
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
                EnableMouseCapture,
                EnableBracketedPaste,
                EnableFocusChange
            )?;
            let mut tui = Self {
                terminal: Terminal::new(CrosstermBackend::new(stdout()))?,
            };
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

/// Best effort: undo everything `enter` did. Safe to call more than once.
fn restore() {
    // Nothing useful can be done if these fail while exiting or panicking.
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
