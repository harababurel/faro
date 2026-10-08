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
use crate::app::state::{MessageLayout, State};
use crate::app::update::update;
use crate::fixtures::{self, ALICE, ME, NOW_MS, climbing, loaded_state};
use crate::theme::Theme;

fn draw(state: &State, width: u16, height: u16) -> Buffer {
    draw_with(state, width, height, &Theme::default())
}

fn draw_with(state: &State, width: u16, height: u16, theme: &Theme) -> Buffer {
    let tz = TimeZone::UTC;
    let now = jiff::Timestamp::from_millisecond(i64::try_from(NOW_MS).unwrap()).unwrap();
    let view = View {
        theme,
        tz: &tz,
        now,
    };
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| {
            render(frame, state, &view);
        })
        .unwrap();
    terminal.backend().buffer().clone()
}

fn snapshot(buffer: &Buffer) -> String {
    crate::frames::grid(buffer)
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

/// Every screen worth checking colours on: the main view with the info
/// panel and a selected message, the palette, and a popup, in both layouts.
fn screens() -> Vec<State> {
    use crossterm::event::KeyCode::{Enter, Esc, Up};
    let mut screens = Vec::new();
    for layout in [MessageLayout::Irc, MessageLayout::Bubbles] {
        let mut state = typing_state();
        state.layout = layout;
        state.panels.right = true;
        screens.push(state);
        let mut selected = loaded_state();
        selected.layout = layout;
        keys(&mut selected, &[Up]);
        screens.push(selected);
        let mut menu = loaded_state();
        menu.layout = layout;
        keys(&mut menu, &[Up, Enter]);
        screens.push(menu);
        let mut palette = loaded_state();
        palette.layout = layout;
        keys(&mut palette, &[Esc]);
        update(
            &mut palette,
            AppEvent::Terminal(crossterm::event::Event::Key(
                crossterm::event::KeyEvent::new(
                    crossterm::event::KeyCode::Char('p'),
                    crossterm::event::KeyModifiers::CONTROL,
                ),
            )),
        );
        assert!(palette.palette.is_some());
        screens.push(palette);
    }
    screens
}

#[test]
fn the_colour_fallbacks_draw_only_colours_the_terminal_has() {
    use crate::theme::{ColorDepth, UserThemes};
    use ratatui::style::Color;
    let screens = screens();
    for theme in Theme::all(&UserThemes::default()) {
        let name = &theme.name;
        let indexed = theme.clone().for_depth(ColorDepth::Indexed256);
        for state in &screens {
            let buffer = draw_with(state, 100, 30, &indexed);
            for cell in &buffer.content {
                for colour in [cell.fg, cell.bg] {
                    assert!(!matches!(colour, Color::Rgb(..)), "{name}: {colour:?}");
                }
            }
        }
    }
    // Every theme is the system theme in 16 colours.
    let system = Theme::default().for_depth(ColorDepth::Ansi16);
    for state in &screens {
        let buffer = draw_with(state, 100, 30, &system);
        for cell in &buffer.content {
            for colour in [cell.fg, cell.bg] {
                assert!(
                    !matches!(colour, Color::Rgb(..) | Color::Indexed(_)),
                    "16 colours: {colour:?}"
                );
            }
        }
    }
}

#[test]
fn the_system_theme_selects_in_reverse_video() {
    use ratatui::style::Modifier;
    let system = Theme::system();
    let state = typing_state();
    let buffer = draw_with(&state, 100, 30, &system);
    let text = snapshot(&buffer);
    // The open conversation's row in the sidebar.
    let title = state.conversation_title(state.open.as_ref().unwrap().id);
    let row = text
        .lines()
        .position(|l| {
            l.chars()
                .take(24)
                .collect::<String>()
                .contains(title.as_str())
        })
        .unwrap();
    let reversed = (0..20)
        .filter(|x| {
            buffer[(*x, u16::try_from(row).unwrap())]
                .modifier
                .contains(Modifier::REVERSED)
        })
        .count();
    assert!(
        reversed >= 10,
        "{reversed} reversed cells in the selected row"
    );
    // Muted text is dim, not a colour.
    assert!(
        buffer
            .content
            .iter()
            .any(|c| c.modifier.contains(Modifier::DIM))
    );
}

#[test]
fn tiny_terminals_never_panic() {
    let mut state = typing_state();
    for (right, layout) in [
        (false, MessageLayout::Irc),
        (true, MessageLayout::Irc),
        (false, MessageLayout::Bubbles),
        (true, MessageLayout::Bubbles),
    ] {
        state.panels.right = right;
        state.layout = layout;
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

// --- Scrolling (M4 task 1) ---

use crate::app::state::{Anchor, Focus};

fn window_text(state: &State, height: usize) -> (Vec<String>, super::messages::Window) {
    let theme = Theme::default();
    let tz = TimeZone::UTC;
    let now = jiff::Timestamp::from_millisecond(i64::try_from(NOW_MS).unwrap()).unwrap();
    let view = View {
        theme: &theme,
        tz: &tz,
        now,
    };
    let open = state.open.as_ref().unwrap();
    let window = super::messages::window(state, open, &view, 60, height);
    let text = window.lines.iter().map(ToString::to_string).collect();
    (text, window)
}

fn scrolled_state(offset: isize) -> State {
    let mut state = loaded_state();
    let open = state.open.as_mut().unwrap();
    open.scroll = Some(Anchor {
        message: fixtures::climbing_history().last().unwrap().reference(),
        offset,
    });
    state
}

#[test]
fn a_scrolled_view_stays_put_when_messages_arrive() {
    let mut state = scrolled_state(4);
    let (before, window) = window_text(&state, 6);
    assert!(window.scroll.is_some());
    let news = fixtures::text(climbing(), ALICE, NOW_MS, "Parking is full");
    update(&mut state, AppEvent::Backend(BackendEvent::Message(news)));
    let (after, _) = window_text(&state, 6);
    assert_eq!(before, after);
}

#[test]
fn scrolling_is_clamped_to_the_loaded_history() {
    // Far past the top: the first row is the first loaded line.
    let (lines, window) = window_text(&scrolled_state(isize::MAX), 6);
    assert!(window.at_top);
    assert_eq!(lines.len(), 6);
    assert!(lines.first().unwrap().contains("Yesterday"));
    // Past the bottom: back to following the newest message.
    let (_, window) = window_text(&scrolled_state(-50), 6);
    assert_eq!(window.scroll, None);
    // The anchor is normalised to the message on the bottom row.
    let (_, window) = window_text(&scrolled_state(1), 6);
    let anchor = window.scroll.unwrap();
    assert_ne!(
        Some(anchor.message),
        fixtures::climbing_history()
            .last()
            .map(faro_core::domain::Message::reference)
    );
    assert!(anchor.offset >= 0);
}

#[test]
fn scrolled_up_with_messages_focus() {
    let mut state = scrolled_state(6);
    state.focus = Focus::Messages;
    state.open.as_mut().unwrap().unseen = 2;
    insta::assert_snapshot!(snapshot(&draw(&state, 100, 18)));
}

fn keys(state: &mut State, keys: &[crossterm::event::KeyCode]) {
    for key in keys {
        update(
            state,
            AppEvent::Terminal(crossterm::event::Event::Key(
                crossterm::event::KeyEvent::from(*key),
            )),
        );
    }
}

use crossterm::event::KeyCode::{Char, Enter, Up};

#[test]
fn message_menu_over_a_selected_message() {
    let mut state = loaded_state();
    keys(&mut state, &[Up, Char('k'), Enter]);
    insta::assert_snapshot!(snapshot(&draw(&state, 100, 20)));
}

#[test]
fn reaction_picker() {
    let mut state = loaded_state();
    keys(&mut state, &[Up, Char('+'), Char('l')]);
    insta::assert_snapshot!(snapshot(&draw(&state, 100, 20)));
}

#[test]
fn reply_bar_above_the_input() {
    let mut state = loaded_state();
    keys(&mut state, &[Up, Char('r'), Char('o'), Char('k')]);
    insta::assert_snapshot!(snapshot(&draw(&state, 100, 20)));
}

#[test]
fn the_selected_message_is_tinted_only_while_the_pane_has_focus() {
    let mut state = loaded_state();
    keys(&mut state, &[Up]);
    let theme = Theme::default();
    let buffer = draw(&state, 100, 20);
    let tinted = |buffer: &Buffer| {
        buffer
            .content
            .iter()
            .filter(|c| c.bg == theme.background_element)
            .count()
    };
    let bars = |buffer: &Buffer| buffer.content.iter().filter(|c| c.symbol() == "▌").count();
    // The newest message is one line: one tinted row, one bar.
    assert!(tinted(&buffer) >= 60);
    assert_eq!(bars(&buffer), 1);
    keys(&mut state, &[crossterm::event::KeyCode::Esc]);
    let buffer = draw(&state, 100, 20);
    assert_eq!(bars(&buffer), 0);
}

#[test]
fn selecting_an_old_message_scrolls_to_it() {
    let mut state = loaded_state();
    keys(&mut state, &[Up, Char('g')]);
    let (lines, window) = window_text(&state, 6);
    assert!(window.at_top);
    assert!(lines.iter().any(|l| l.contains("Anyone up for the wall")));
    let first = fixtures::climbing_history().first().unwrap().reference();
    assert!(window.rows.contains(&Some(first)));
    // Back to the newest: following again.
    keys(&mut state, &[Char('G')]);
    let (_, window) = window_text(&state, 6);
    assert_eq!(window.scroll, None);
}

// --- Curation (M4 task 5, D41) ---

fn sidebar_keys(state: &mut State, list: &[crossterm::event::KeyCode]) {
    state.focus = Focus::Conversations;
    keys(state, list);
}

#[test]
fn conversation_menu_beside_the_sidebar() {
    let mut state = loaded_state();
    sidebar_keys(&mut state, &[Char(' ')]);
    insta::assert_snapshot!(snapshot(&draw(&state, 100, 20)));
}

#[test]
fn archive_row_and_a_muted_conversation() {
    let mut state = loaded_state();
    // Archive Climbing (Alice opens), then mute Alice until unmuted.
    sidebar_keys(&mut state, &[Char('a'), Char('m'), Char('5')]);
    state.panels.right = true;
    state.notice = None;
    insta::assert_snapshot!(snapshot(&draw(&state, 120, 16)));
}

#[test]
fn archive_view() {
    let mut state = loaded_state();
    sidebar_keys(&mut state, &[Char('a'), Char('h'), Char('A')]);
    state.notice = None;
    insta::assert_snapshot!(snapshot(&draw(&state, 100, 14)));
}

#[test]
fn allowlist_mode_shows_in_the_title() {
    let mut state = loaded_state();
    sidebar_keys(&mut state, &[Char('l'), Char('L')]);
    let text = snapshot(&draw(&state, 100, 14));
    assert!(text.contains("Conversations · allowlist"), "{text}");
    assert!(text.contains("▸ Archive (1)"), "{text}");
}

fn with_palette(query: &str) -> State {
    use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
    let mut state = loaded_state();
    let press = |state: &mut State, code, modifiers| {
        update(
            state,
            AppEvent::Terminal(Event::Key(KeyEvent::new(code, modifiers))),
        );
    };
    press(&mut state, KeyCode::Char('p'), KeyModifiers::CONTROL);
    for c in query.chars() {
        press(&mut state, KeyCode::Char(c), KeyModifiers::NONE);
    }
    state
}

#[test]
fn palette_with_nothing_typed() {
    insta::assert_snapshot!(snapshot(&draw(&with_palette(""), 100, 26)));
}

#[test]
fn palette_searching() {
    insta::assert_snapshot!(snapshot(&draw(&with_palette(">mute"), 100, 20)));
}

#[test]
fn palette_keybindings() {
    insta::assert_snapshot!(snapshot(&draw(&with_palette("?"), 100, 26)));
}

#[test]
fn palette_settings_level() {
    use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
    let mut state = with_palette(":notif");
    update(
        &mut state,
        AppEvent::Terminal(Event::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        ))),
    );
    insta::assert_snapshot!(snapshot(&draw(&state, 100, 20)));
}

#[test]
fn palette_themes() {
    use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
    let mut state = with_palette("themes");
    for code in [KeyCode::Enter, KeyCode::Up] {
        update(
            &mut state,
            AppEvent::Terminal(Event::Key(KeyEvent::new(code, KeyModifiers::NONE))),
        );
    }
    // Up from the first theme wraps to the last one: previewed.
    assert_eq!(state.theme_name(), "zenburn-light");
    insta::assert_snapshot!(snapshot(&draw(&state, 100, 20)));
}

#[test]
fn palette_chats() {
    insta::assert_snapshot!(snapshot(&draw(&with_palette("@"), 100, 20)));
}

#[test]
fn palette_reports_its_rows_for_the_mouse() {
    let state = with_palette("");
    let theme = Theme::default();
    let tz = TimeZone::UTC;
    let now = jiff::Timestamp::from_millisecond(i64::try_from(NOW_MS).unwrap()).unwrap();
    let view = View {
        theme: &theme,
        tz: &tz,
        now,
    };
    let mut terminal = Terminal::new(TestBackend::new(100, 26)).unwrap();
    let mut geometry = None;
    terminal
        .draw(|frame| {
            geometry = render(frame, &state, &view).palette;
        })
        .unwrap();
    let geometry = geometry.unwrap();
    assert!(geometry.area.contains(ratatui::layout::Position::new(
        geometry.list.x,
        geometry.list.y
    )));
    assert_eq!(geometry.offset, 0);
    assert!(geometry.list.height >= 3);
}

#[test]
fn do_not_disturb_shows_in_the_footer() {
    let theme = Theme::default();
    let mut state = loaded_state();
    let text = snapshot(&draw(&state, 100, 14));
    assert!(!text.contains("bell off"), "{text}");
    state.config.notifications.bell = false;
    let buffer = draw(&state, 100, 14);
    let text = snapshot(&buffer);
    let footer = text.lines().last().unwrap();
    assert!(
        footer.starts_with(" ● connected · bell off · 1 unread"),
        "{footer}"
    );
    let x = u16::try_from(unicode_width::UnicodeWidthStr::width(
        &footer[..footer.find("bell off").unwrap()],
    ))
    .unwrap();
    assert_eq!(buffer[(x, 13)].fg, theme.warning);
}

// --- Bubbles layout (M4 task 9, D45) ---

fn bubbles_state() -> State {
    let mut state = typing_state();
    state.layout = MessageLayout::Bubbles;
    state
}

#[test]
fn bubbles_in_a_group() {
    insta::assert_snapshot!(snapshot(&draw(&bubbles_state(), 100, 34)));
}

#[test]
fn bubbles_narrow() {
    insta::assert_snapshot!(snapshot(&draw(&bubbles_state(), 60, 24)));
}

#[test]
fn bubbles_have_their_own_backgrounds_and_sides() {
    let theme = Theme::default();
    let buffer = draw(&bubbles_state(), 100, 34);
    let text = snapshot(&buffer);
    let find = |needle: &str| {
        let (row, line) = text
            .lines()
            .enumerate()
            .find(|(_, l)| l.contains(needle))
            .unwrap_or_else(|| panic!("{needle:?} not on screen"));
        let byte = line.find(needle).unwrap();
        let col = unicode_width::UnicodeWidthStr::width(&line[..byte]);
        (u16::try_from(col).unwrap(), u16::try_from(row).unwrap())
    };
    // Own message: own colour, on the right.
    let (own_x, own_y) = find("On my way");
    assert_eq!(buffer[(own_x, own_y)].bg, theme.bubble_own);
    // Others' messages: the other colour, on the left.
    let (x, y) = find("Running 10 min late");
    assert_eq!(buffer[(x, y)].bg, theme.bubble_other);
    assert!(own_x > x + 20, "own bubble on the right ({own_x} vs {x})");
}

// --- Emoji every terminal measures alike (M4.5, D47) ---

fn compatible(mut state: State) -> State {
    state.env.emoji.mode = crate::app::state::EmojiMode::Compatible;
    state
}

/// Every drawn cell is one every terminal agrees on, and all text went through
/// `text::sanitize` (no stand-ins were needed).
fn assert_drawable(state: &State, width: u16, height: u16) {
    let buffer = draw(state, width, height);
    assert_eq!(super::emoji::last_stand_ins(), 0, "text skipped sanitize");
    for (i, cell) in buffer.content.iter().enumerate() {
        assert!(
            super::emoji::drawable(cell.symbol()),
            "cell {i} draws {:?}",
            cell.symbol()
        );
    }
}

#[test]
fn compatible_mode_draws_only_what_every_terminal_measures_alike() {
    // The fixtures have ❤️ reactions and joined climbers (🧗‍♀️🧗‍♂️).
    for (width, height) in [(100, 34), (60, 24)] {
        assert_drawable(&compatible(typing_state()), width, height);
        assert_drawable(&compatible(bubbles_state()), width, height);
    }
    let mut menu = loaded_state();
    keys(&mut menu, &[Up, Char('k'), Enter]);
    assert_drawable(&compatible(menu), 100, 20);
    let mut picker = loaded_state();
    keys(&mut picker, &[Up, Char('+'), Char('l')]);
    assert_drawable(&compatible(picker), 100, 20);
    assert_drawable(&compatible(with_palette("")), 100, 26);
    assert_drawable(&compatible(with_palette("@climb")), 100, 26);
    assert_drawable(&compatible(with_palette(">")), 100, 26);
}

#[test]
fn compatible_mode_covers_the_input_too() {
    // The input isn't sanitised (its cursor follows the raw text), so the
    // frame gets stand-ins of the same width instead.
    let mut state = compatible(loaded_state());
    for c in "hi 👨‍👩‍👧‍👦 1️⃣ 👍🏽".chars() {
        keys(&mut state, &[Char(c)]);
    }
    let buffer = draw(&state, 100, 20);
    assert!(super::emoji::last_stand_ins() >= 3);
    assert!(
        buffer
            .content
            .iter()
            .all(|c| super::emoji::drawable(c.symbol()))
    );
    assert!(snapshot(&buffer).contains("hi 👨 1  👍"));
}

#[test]
fn full_mode_keeps_emoji_as_they_are() {
    let buffer = draw(&typing_state(), 100, 34);
    assert!(snapshot(&buffer).contains("🧗‍♀️"));
}
