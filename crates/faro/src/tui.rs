//! Terminal setup and teardown. The terminal is always restored: on drop, and
//! from a panic hook (otherwise a panic would leave the shell in raw mode with
//! mouse reporting on).

use std::io::{self, Stdout, stdout};
use std::sync::Once;

use crossterm::event::{DisableMouseCapture, EnableMouseCapture};
use crossterm::terminal::{
    BeginSynchronizedUpdate, EndSynchronizedUpdate, EnterAlternateScreen, LeaveAlternateScreen,
    disable_raw_mode, enable_raw_mode,
};
use crossterm::{cursor, execute};
use ratatui::backend::CrosstermBackend;
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
            execute!(stdout(), EnterAlternateScreen, EnableMouseCapture)?;
            let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;
            terminal.clear()?;
            Ok(Self { terminal })
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
