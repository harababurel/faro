//! The late-reply guard (D90). A clipboard reply that arrives after faro
//! stopped waiting (the user answered kitty's prompt after the timeout, or a
//! second kitty's prompt on another screen) reaches crossterm as keys:
//! `Alt+]` (or `Esc`, `]` when a read ends after the `ESC`), then
//! `5`, `5`, `2`, `2`, `;` and the rest one `Char` per byte. Left alone, it
//! types into faro and then wedges crossterm's input for good. The guard holds
//! back keys that could be the start of one; if they are, the app loop drains
//! the rest from the tty instead.

use std::time::Duration;

use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use tokio::time::Instant;

/// How long keys that might start a reply are held back. A reply's bytes
/// arrive together; a person typing `Alt+]` then `5` is much slower.
const HOLD: Duration = Duration::from_millis(50);

/// What follows `OSC` in a reply.
const TAIL: [char; 5] = ['5', '5', '2', '2', ';'];

/// What to do with a terminal event.
#[derive(Debug, PartialEq, Eq)]
pub enum Filtered {
    /// Hand these to the reducer (none while keys are held back).
    Pass(Vec<Event>),
    /// A reply is arriving: pause input and drain it; the held keys were
    /// part of it.
    Drain,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum Stage {
    #[default]
    Idle,
    /// A plain `Esc`: `]` would make it `OSC`.
    Esc,
    /// `OSC` and this many characters of [`TAIL`].
    Tail(usize),
}

/// Watches terminal events for the start of a clipboard reply.
#[derive(Debug, Default)]
pub struct LateGuard {
    /// Only after the first paste: before that no reply can come, and keys
    /// are never held back.
    armed: bool,
    stage: Stage,
    held: Vec<Event>,
    since: Option<Instant>,
}

impl LateGuard {
    pub fn arm(&mut self) {
        self.armed = true;
    }

    /// When the held keys go to the reducer after all.
    pub fn deadline(&self) -> Option<Instant> {
        self.since.map(|since| since + HOLD)
    }

    /// Give back the held keys (they weren't a reply).
    pub fn flush(&mut self) -> Vec<Event> {
        self.stage = Stage::Idle;
        self.since = None;
        std::mem::take(&mut self.held)
    }

    pub fn filter(&mut self, event: Event) -> Filtered {
        if !self.armed {
            return Filtered::Pass(vec![event]);
        }
        let next = match (self.stage, &event) {
            (Stage::Idle, Event::Key(key)) if is_alt(key, ']') => Some(Stage::Tail(0)),
            (Stage::Idle, Event::Key(key))
                if key.code == KeyCode::Esc && key.modifiers.is_empty() =>
            {
                Some(Stage::Esc)
            }
            (Stage::Esc, Event::Key(key)) if is_plain(key, ']') => Some(Stage::Tail(0)),
            (Stage::Tail(n), Event::Key(key)) if TAIL.get(n).is_some_and(|&c| is_plain(key, c)) => {
                Some(Stage::Tail(n + 1))
            }
            _ => None,
        };
        match next {
            Some(Stage::Tail(n)) if n == TAIL.len() => {
                self.flush();
                Filtered::Drain
            }
            Some(stage) => {
                self.stage = stage;
                self.since.get_or_insert_with(Instant::now);
                self.held.push(event);
                Filtered::Pass(Vec::new())
            }
            None if self.stage == Stage::Idle => Filtered::Pass(vec![event]),
            None => {
                // Not a reply after all: give the held keys back, then look at
                // this one afresh (it may start another).
                let mut out = self.flush();
                match self.filter(event) {
                    Filtered::Pass(mut more) => out.append(&mut more),
                    Filtered::Drain => return Filtered::Drain,
                }
                Filtered::Pass(out)
            }
        }
    }
}

fn is_alt(key: &KeyEvent, c: char) -> bool {
    key.code == KeyCode::Char(c) && key.modifiers == KeyModifiers::ALT
}

fn is_plain(key: &KeyEvent, c: char) -> bool {
    key.code == KeyCode::Char(c) && key.modifiers.is_empty()
}
