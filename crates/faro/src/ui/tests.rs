//! Snapshot tests (DESIGN.md §15): the fixture state rendered at several sizes.
//! Snapshots show text only; a few colour checks follow.
//! Update with `INSTA_UPDATE=always cargo test` (or `cargo insta review`), then
//! read the diff.

use faro_core::backend::BackendEvent;
use faro_core::domain::TypingState;
use jiff::tz::TimeZone;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;

use super::{View, render};
use crate::app::event::AppEvent;
use crate::app::state::State;
use crate::app::update::update;
use crate::fixtures::{self, ALICE, ME, NOW_MS, climbing, loaded_state};
use crate::theme::Theme;

fn draw(state: &State, width: u16, height: u16) -> Buffer {
    let theme = Theme::default();
    let tz = TimeZone::UTC;
    let now = jiff::Timestamp::from_millisecond(i64::try_from(NOW_MS).unwrap()).unwrap();
    let view = View {
        theme: &theme,
        tz: &tz,
        now,
    };
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|frame| render(frame, state, &view)).unwrap();
    terminal.backend().buffer().clone()
}

fn snapshot(buffer: &Buffer) -> String {
    let width = usize::from(buffer.area.width);
    let mut out = String::new();
    for row in buffer.content.chunks(width) {
        let mut skip = 0;
        for cell in row {
            // A wide character's second cell is blank; don't print it twice.
            if skip > 0 {
                skip -= 1;
                continue;
            }
            let symbol = cell.symbol();
            skip = unicode_width::UnicodeWidthStr::width(symbol).saturating_sub(1);
            out.push_str(symbol);
        }
        out.push('\n');
    }
    out
}

fn typing_state() -> State {
    let mut state = loaded_state();
    update(
        &mut state,
        AppEvent::Backend(BackendEvent::Typing {
            conversation: climbing(),
            sender: ALICE,
            state: TypingState::Started,
        }),
    );
    for c in "See you there".chars() {
        update(
            &mut state,
            AppEvent::Terminal(crossterm::event::Event::Key(
                crossterm::event::KeyEvent::from(crossterm::event::KeyCode::Char(c)),
            )),
        );
    }
    state
}

#[test]
fn wide_with_info_sidebar() {
    let mut state = typing_state();
    state.panels.right = true;
    insta::assert_snapshot!(snapshot(&draw(&state, 120, 32)));
}

#[test]
fn normal() {
    insta::assert_snapshot!(snapshot(&draw(&typing_state(), 100, 30)));
}

#[test]
fn narrow_hides_sidebars() {
    insta::assert_snapshot!(snapshot(&draw(&typing_state(), 60, 24)));
}

#[test]
fn loading() {
    let state = State::new(ME, fixtures::contacts(), fixtures::groups());
    insta::assert_snapshot!(snapshot(&draw(&state, 80, 12)));
}

#[test]
fn colours_carry_meaning() {
    let theme = Theme::default();
    let buffer = draw(&typing_state(), 100, 30);
    let find = |needle: &str| {
        let text = snapshot(&buffer);
        let (row, line) = text
            .lines()
            .enumerate()
            .find(|(_, l)| l.contains(needle))
            .unwrap_or_else(|| panic!("{needle:?} not on screen"));
        let col = line
            .find(needle)
            .map(|byte| unicode_width::UnicodeWidthStr::width(&line[..byte]));
        (
            u16::try_from(col.unwrap()).unwrap(),
            u16::try_from(row).unwrap(),
        )
    };
    // Connection status in the success colour.
    let (x, y) = find("● connected");
    assert_eq!(buffer[(x, y)].fg, theme.success);
    // The unread marker rule in the unread colour.
    let (x, y) = find("new messages");
    assert_eq!(buffer[(x, y)].fg, theme.unread);
    // The input box has the panel background.
    let (x, y) = find("See you there");
    assert_eq!(buffer[(x, y)].bg, theme.background_panel);
}

#[test]
fn tiny_terminals_never_panic() {
    let mut state = typing_state();
    for right in [false, true] {
        state.panels.right = right;
        for width in 0..=40 {
            for height in 0..=12 {
                draw(&state, width, height);
            }
        }
    }
    let empty = State::new(ME, fixtures::contacts(), fixtures::groups());
    for width in 0..=20 {
        for height in 0..=6 {
            draw(&empty, width, height);
        }
    }
}
