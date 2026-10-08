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
        logo_time: None,
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
        logo_time: None,
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
    // Every setting under its section's header, current values on the right.
    insta::assert_snapshot!(snapshot(&draw(&with_palette(":"), 100, 30)));
}

#[test]
fn palette_settings_filtered() {
    // Only the sections with a match keep their header.
    insta::assert_snapshot!(snapshot(&draw(&with_palette(":notif"), 100, 20)));
}

#[test]
fn palette_themes_at_256_colours() {
    use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
    let mut state = with_palette("themes");
    state.env.colors = crate::theme::ColorDepth::Indexed256;
    update(
        &mut state,
        AppEvent::Terminal(Event::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        ))),
    );
    // The note sits between the rule and the list.
    let buffer = draw(&state, 100, 20);
    insta::assert_snapshot!(snapshot(&buffer));
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
        logo_time: None,
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

#[test]
fn running_without_an_allowlist_shows_in_the_footer() {
    let mut state = loaded_state();
    let text = snapshot(&draw(&state, 100, 14));
    assert!(!text.contains("no allowlist"), "{text}");
    state.guard = crate::app::state::Guard::Off;
    let text = snapshot(&draw(&state, 100, 14));
    let footer = text.lines().last().unwrap();
    assert!(
        footer.starts_with(" ● connected · no allowlist · 1 unread"),
        "{footer}"
    );
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

// --- Bubble styles (D58) ---

fn styled(mut state: State, style: faro_core::config::BubbleStyle) -> State {
    state.config.messages.bubble_style = style;
    state
}

fn rounded(state: State) -> State {
    styled(state, faro_core::config::BubbleStyle::Rounded)
}

fn bar(state: State) -> State {
    styled(state, faro_core::config::BubbleStyle::Bar)
}

fn mixed(state: State) -> State {
    styled(state, faro_core::config::BubbleStyle::Mixed)
}

#[test]
fn bubbles_rounded() {
    insta::assert_snapshot!(snapshot(&draw(&rounded(bubbles_state()), 100, 34)));
}

#[test]
fn bubbles_rounded_narrow() {
    insta::assert_snapshot!(snapshot(&draw(&rounded(bubbles_state()), 60, 24)));
}

#[test]
fn bubbles_rounded_very_narrow() {
    insta::assert_snapshot!(snapshot(&draw(&rounded(bubbles_state()), 32, 18)));
}

#[test]
fn bubbles_mixed() {
    insta::assert_snapshot!(snapshot(&draw(&mixed(bubbles_state()), 100, 34)));
}

#[test]
fn bubbles_mixed_narrow() {
    insta::assert_snapshot!(snapshot(&draw(&mixed(bubbles_state()), 60, 24)));
}

#[test]
fn bubbles_bar_very_narrow() {
    insta::assert_snapshot!(snapshot(&draw(&bar(bubbles_state()), 32, 18)));
}

/// The cell where `needle` starts on screen.
fn find(buffer: &Buffer, needle: &str) -> (u16, u16) {
    let text = snapshot(buffer);
    let (row, line) = text
        .lines()
        .enumerate()
        .find(|(_, l)| l.contains(needle))
        .unwrap_or_else(|| panic!("{needle:?} not on screen"));
    let byte = line.find(needle).unwrap();
    let col = unicode_width::UnicodeWidthStr::width(&line[..byte]);
    (u16::try_from(col).unwrap(), u16::try_from(row).unwrap())
}

/// The first cell on row `y` from `x` on whose symbol is `symbol`.
fn next(buffer: &Buffer, x: u16, y: u16, symbol: &str) -> u16 {
    (x..buffer.area.right())
        .find(|&cx| buffer[(cx, y)].symbol() == symbol)
        .unwrap_or_else(|| panic!("no {symbol:?} on row {y} after {x}"))
}

#[test]
fn auto_is_rounded_only_where_the_terminal_draws_the_glyphs() {
    let auto = bubbles_state();
    assert_eq!(
        snapshot(&draw(&auto, 100, 34)),
        snapshot(&draw(&bar(bubbles_state()), 100, 34))
    );
    let mut kitty = bubbles_state();
    kitty.env.glyphs = crate::app::state::BlockGlyphs::Native;
    assert_eq!(
        snapshot(&draw(&kitty, 100, 34)),
        snapshot(&draw(&rounded(bubbles_state()), 100, 34))
    );
}

#[test]
fn one_line_rounded_bubbles_have_half_circle_caps_in_their_colour() {
    let theme = Theme::default();
    let buffer = draw(&rounded(bubbles_state()), 100, 34);
    let (x, y) = find(&buffer, "On my way");
    let pane = buffer[(x, y - 1)].bg;
    let left = &buffer[(x - 1, y)];
    assert_eq!(left.symbol(), "\u{E0B6}");
    assert_eq!((left.fg, left.bg), (theme.bubble_own, pane));
    let right = next(&buffer, x, y, "\u{E0B4}");
    assert_eq!(buffer[(right, y)].fg, theme.bubble_own);
    assert_eq!(buffer[(right, y)].bg, pane);
    assert_eq!(buffer[(right - 1, y)].bg, theme.bubble_own);
    // No edge rows around a one-line bubble.
    for row in [y - 1, y + 1] {
        assert!(
            (0..buffer.area.width)
                .all(|cx| !super::bubbles::EDGE_GLYPHS.contains(&buffer[(cx, row)].symbol())),
            "edge on row {row}"
        );
    }
    let (ox, oy) = find(&buffer, "Running 10 min late");
    assert_eq!(buffer[(ox - 1, oy)].fg, theme.bubble_other);
}

#[test]
fn taller_rounded_bubbles_have_two_thirds_edges_with_diagonal_corners() {
    let theme = Theme::default();
    let buffer = draw(&rounded(bubbles_state()), 60, 24);
    let fill_top = "\u{1FB39}";
    let (_, top) = find(&buffer, fill_top);
    let left = next(&buffer, 0, top, "\u{1FB4A}");
    let right = next(&buffer, left, top, "\u{1FB3F}");
    let bottom = (top + 2..buffer.area.height)
        .find(|&y| buffer[(left, y)].symbol() == "\u{1FB65}")
        .expect("bottom edge");
    assert_eq!(buffer[(right, bottom)].symbol(), "\u{1FB5A}");
    let colour = buffer[(left + 1, top + 1)].bg;
    assert!(colour == theme.bubble_own || colour == theme.bubble_other);
    for x in left..=right {
        for y in [top, bottom] {
            let cell = &buffer[(x, y)];
            assert_eq!(cell.fg, colour, "({x}, {y})");
            assert_ne!(cell.bg, colour, "({x}, {y})");
        }
        assert_eq!(
            buffer[(x, bottom)].symbol() == "\u{1FB0E}",
            x > left && x < right
        );
        // The edges are exactly as wide as the bubble.
        assert_eq!(buffer[(x, top + 1)].bg, colour);
    }
    assert_ne!(buffer[(right + 1, top + 1)].bg, colour);
}

#[test]
fn bars_go_on_the_outer_side_in_the_senders_colour() {
    let theme = Theme::default();
    let state = bar(bubbles_state());
    let buffer = draw(&state, 100, 34);
    let (x, y) = find(&buffer, "On my way");
    let right = next(&buffer, x, y, super::bubbles::BAR);
    assert_eq!(buffer[(right, y)].fg, theme.primary);
    assert_eq!(buffer[(right, y)].bg, theme.bubble_own);
    assert_eq!(buffer[(x - 1, y)].symbol(), " ");
    assert_eq!(buffer[(x - 1, y)].bg, theme.bubble_own);
    let (ox, oy) = find(&buffer, "Running 10 min late");
    let cell = &buffer[(ox - 1, oy)];
    assert_eq!(cell.symbol(), super::bubbles::BAR);
    assert_eq!(cell.bg, theme.bubble_other);
    let open = state.open.as_ref().unwrap();
    let sender = open
        .messages
        .iter()
        .find(|m| {
            m.body
                .as_ref()
                .is_some_and(|b| b.expose().contains("Running 10 min late"))
        })
        .unwrap()
        .sender;
    assert_eq!(Some(cell.fg), theme.sender(sender).fg);
    // No rounded glyphs.
    let text = snapshot(&buffer);
    assert!(
        !super::bubbles::EDGE_GLYPHS.iter().any(|g| text.contains(g)),
        "{text}"
    );
}

#[test]
fn mixed_bubbles_have_the_bar_outside_and_round_inside() {
    let theme = Theme::default();
    let buffer = draw(&mixed(bubbles_state()), 100, 34);
    // One line, own: a cap on the left (inner), the bar on the right.
    let (x, y) = find(&buffer, "On my way");
    let pane = buffer[(x, y - 1)].bg;
    let cap = &buffer[(x - 1, y)];
    assert_eq!(cap.symbol(), "\u{E0B6}");
    assert_eq!((cap.fg, cap.bg), (theme.bubble_own, pane));
    let right = next(&buffer, x, y, super::bubbles::BAR);
    assert_eq!(buffer[(right, y)].fg, theme.primary);
    assert_eq!(buffer[(right, y)].bg, theme.bubble_own);
    // Others: the bar on the left (outer).
    let (ox, oy) = find(&buffer, "Running 10 min late");
    let bar = &buffer[(ox - 1, oy)];
    assert_eq!(bar.symbol(), super::bubbles::BAR);
    assert_eq!(bar.bg, theme.bubble_other);
}

#[test]
fn taller_mixed_bubbles_have_a_square_corner_on_the_bars_side() {
    let buffer = draw(&mixed(bubbles_state()), 60, 24);
    let text = snapshot(&buffer);
    let fill_top = "\u{1FB39}";
    let fill_bottom = "\u{1FB0E}";
    let mut seen = 0;
    for y in 0..buffer.area.height {
        let row: Vec<&str> = (0..buffer.area.width)
            .map(|x| buffer[(x, y)].symbol())
            .collect();
        for (fill, rounded) in [
            (fill_top, ["\u{1FB4A}", "\u{1FB3F}"]),
            (fill_bottom, ["\u{1FB65}", "\u{1FB5A}"]),
        ] {
            let Some(start) = row.iter().position(|s| *s == fill || rounded.contains(s)) else {
                continue;
            };
            let end = row
                .iter()
                .rposition(|s| *s == fill || rounded.contains(s))
                .unwrap();
            let ends = [row[start], row[end]];
            // Exactly one end is rounded, the other is the fill (square).
            let rounded_ends = ends.iter().filter(|s| rounded.contains(s)).count();
            assert_eq!(rounded_ends, 1, "row {y}: {ends:?}\n{text}");
            assert!(ends.contains(&fill), "row {y}: {ends:?}\n{text}");
            seen += 1;
        }
    }
    assert!(seen >= 2, "{text}");
}

#[test]
fn a_blank_line_separates_bubbles_in_both_styles() {
    for state in [
        rounded(bubbles_state()),
        bar(bubbles_state()),
        mixed(bubbles_state()),
    ] {
        let text = snapshot(&draw(&state, 100, 34));
        let lines: Vec<&str> = text.lines().collect();
        let (y, _) = lines
            .iter()
            .enumerate()
            .find(|(_, l)| l.contains("On my way"))
            .unwrap();
        assert_eq!(lines[y - 1].trim_matches(['│', ' ']), "", "{text}");
    }
}

#[test]
fn a_selected_bubble_is_a_clean_tinted_block_in_both_styles() {
    let theme = Theme::default();
    for style in [rounded, bar, mixed] {
        let mut state = style(loaded_state());
        state.layout = MessageLayout::Bubbles;
        keys(&mut state, &[Up]);
        let buffer = draw(&state, 100, 34);
        let selected: Vec<u16> = (0..buffer.area.height)
            .filter(|&y| (0..buffer.area.width).any(|x| buffer[(x, y)].symbol() == "▌"))
            .collect();
        assert_ne!(selected, [] as [u16; 0]);
        for y in selected {
            for x in 0..buffer.area.width {
                let cell = &buffer[(x, y)];
                if cell.bg == theme.background_element {
                    assert!(
                        !super::bubbles::EDGE_GLYPHS.contains(&cell.symbol()),
                        "edge left at ({x}, {y})"
                    );
                }
            }
        }
    }
}

#[test]
fn the_system_theme_always_draws_bars() {
    for state in [
        rounded(bubbles_state()),
        bar(bubbles_state()),
        mixed(bubbles_state()),
    ] {
        let buffer = draw_with(&state, 100, 34, &Theme::system());
        let text = snapshot(&buffer);
        assert!(
            !super::bubbles::EDGE_GLYPHS.iter().any(|g| text.contains(g)),
            "{text}"
        );
        assert!(text.contains(super::bubbles::BAR), "{text}");
    }
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
        assert_drawable(&compatible(rounded(bubbles_state())), width, height);
        assert_drawable(&compatible(mixed(bubbles_state())), width, height);
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

// --- The logo screen (D61) ---

/// After startup: conversations loaded, nothing open (D60).
fn logo_state() -> State {
    let mut state = State::new(ME, fixtures::contacts(), fixtures::groups());
    update(
        &mut state,
        AppEvent::Loaded {
            conversations: fixtures::summaries(),
            last_open: Some(climbing()),
        },
    );
    state
}

/// Draw with the logo `logo_time` seconds in (`None`: still), returning the
/// geometry too.
fn draw_logo(
    state: &State,
    size: (u16, u16),
    theme: &Theme,
    logo_time: Option<f64>,
) -> (Buffer, crate::app::state::Geometry) {
    let tz = TimeZone::UTC;
    let now = jiff::Timestamp::from_millisecond(i64::try_from(NOW_MS).unwrap()).unwrap();
    let view = View {
        theme,
        tz: &tz,
        now,
        logo_time,
    };
    let mut terminal = Terminal::new(TestBackend::new(size.0, size.1)).unwrap();
    let mut geometry = crate::app::state::Geometry::default();
    terminal
        .draw(|frame| geometry = render(frame, state, &view))
        .unwrap();
    (terminal.backend().buffer().clone(), geometry)
}

#[test]
fn logo_full() {
    insta::assert_snapshot!(snapshot(&draw(&logo_state(), 140, 24)));
}

#[test]
fn logo_stacked() {
    insta::assert_snapshot!(snapshot(&draw(&logo_state(), 100, 32)));
}

#[test]
fn logo_small() {
    insta::assert_snapshot!(snapshot(&draw(&logo_state(), 100, 20)));
}

#[test]
fn logo_word_only() {
    insta::assert_snapshot!(snapshot(&draw(&logo_state(), 60, 12)));
}

#[test]
fn logo_at_16_colours() {
    let mut state = logo_state();
    state.config.ui.colors = faro_core::config::ColorChoice::Colors16;
    let theme = Theme::default().for_depth(crate::theme::ColorDepth::Ansi16);
    let buffer = draw_with(&state, 140, 24, &theme);
    insta::assert_snapshot!(snapshot(&buffer));
    // The small lighthouse in the theme's (the terminal's) colours.
    let cells: Vec<_> = buffer.content().iter().collect();
    assert!(
        cells
            .iter()
            .any(|c| c.symbol() == "█" && c.fg == theme.error)
    );
    assert!(
        cells
            .iter()
            .all(|c| !matches!(c.fg, ratatui::style::Color::Rgb(..)))
    );
}

#[test]
fn logo_with_no_conversations() {
    let mut state = State::new(ME, fixtures::contacts(), fixtures::groups());
    update(
        &mut state,
        AppEvent::Loaded {
            conversations: vec![],
            last_open: None,
        },
    );
    insta::assert_snapshot!(snapshot(&draw(&state, 120, 26)));
}

#[test]
fn the_logo_s_colours_follow_the_terminal() {
    use crate::theme::ColorDepth;
    use ratatui::style::Color;
    let state = logo_state();
    // Truecolor: the logo's own red stripes.
    let (buffer, _) = draw_logo(&state, (140, 24), &Theme::default(), None);
    assert!(
        buffer
            .content()
            .iter()
            .any(|c| c.fg == Color::Rgb(210, 60, 60))
    );
    // 256 colours: nothing but indexed colours anywhere.
    let mut state = logo_state();
    state.config.ui.colors = faro_core::config::ColorChoice::Colors256;
    let theme = Theme::default().for_depth(ColorDepth::Indexed256);
    let (buffer, _) = draw_logo(&state, (140, 24), &theme, None);
    assert!(
        buffer
            .content()
            .iter()
            .all(|c| !matches!(c.fg, Color::Rgb(..)) && !matches!(c.bg, Color::Rgb(..)))
    );
    assert!(buffer.content().iter().any(|c| c.symbol() == "▀"));
}

#[test]
fn only_the_big_logo_asks_for_more_frames() {
    let state = logo_state();
    let theme = Theme::default();
    let moving = |size, time| draw_logo(&state, size, &theme, time).1.logo_animated;
    assert!(moving((140, 24), Some(1.0)));
    assert!(moving((100, 32), Some(1.0)));
    assert!(!moving((140, 24), None), "still");
    assert!(!moving((100, 20), Some(1.0)), "the small one never moves");
    // Different moments, different pictures; the same moment, the same.
    let at = |time| snapshot(&draw_logo(&state, (140, 24), &theme, time).0);
    assert_ne!(at(Some(1.0)), at(Some(2.0)));
    assert_eq!(at(Some(1.0)), at(Some(1.0)));
    assert_eq!(at(None), at(Some(super::logo::STILL)));
}

/// Climbing with a new message from Alice that mentions Bob and us (D72).
fn mentions_state(layout: MessageLayout) -> State {
    let mut state = loaded_state();
    state.layout = layout;
    let mut message = fixtures::text(
        climbing(),
        ALICE,
        NOW_MS + 1,
        "\u{FFFC} \u{FFFC} the wall is free, coming?",
    );
    let at = |who, start| faro_core::domain::Mention {
        who,
        start,
        len: '\u{FFFC}'.len_utf8(),
    };
    message.mentions = vec![at(fixtures::BOB, 0), at(ME, 4)];
    update(
        &mut state,
        AppEvent::Backend(BackendEvent::Message(message)),
    );
    state
}

#[test]
fn mentions_in_irc() {
    let state = mentions_state(MessageLayout::Irc);
    insta::assert_snapshot!(snapshot(&draw(&state, 100, 20)));
}

#[test]
fn mentions_in_bubbles() {
    let state = mentions_state(MessageLayout::Bubbles);
    insta::assert_snapshot!(snapshot(&draw(&state, 100, 20)));
}

#[test]
fn mentions_are_names_in_the_accent_colour_and_ours_stand_out() {
    use ratatui::style::Modifier;
    let theme = Theme::default();
    for layout in [MessageLayout::Irc, MessageLayout::Bubbles] {
        let state = mentions_state(layout);
        let buffer = draw(&state, 100, 20);
        let text = snapshot(&buffer);
        assert!(!text.contains('\u{FFFC}'), "no placeholder left");
        let find = |needle: &str| {
            let (row, line) = text
                .lines()
                .enumerate()
                .find(|(_, l)| l.contains(needle))
                .unwrap_or_else(|| panic!("{needle:?} not on screen"));
            let col = line
                .find(needle)
                .map(|byte| unicode_width::UnicodeWidthStr::width(&line[..byte]));
            let cell = (
                u16::try_from(col.unwrap()).unwrap(),
                u16::try_from(row).unwrap(),
            );
            buffer[cell].clone()
        };
        let bob = find(&state.mention_name(fixtures::BOB));
        assert_eq!(bob.fg, theme.accent, "{layout:?}");
        assert!(bob.modifier.contains(Modifier::BOLD));
        assert!(!bob.modifier.contains(Modifier::REVERSED));
        let us = find("@You");
        assert_eq!(us.fg, theme.accent, "{layout:?}");
        assert!(us.modifier.contains(Modifier::REVERSED));
        assert!(!find("the wall").modifier.contains(Modifier::BOLD));
    }
}
