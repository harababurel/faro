//! The centre pane: messages, a typing line, and the input box as a distinct
//! panel with its own background (OpenCode style, DESIGN.md §7.1).

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Position, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Padding, Paragraph};
use unicode_segmentation::UnicodeSegmentation as _;
use unicode_width::UnicodeWidthStr as _;

use super::text::{sanitize, truncate};
use super::{View, focused, messages};
use crate::app::input::Input;
use crate::app::state::{Focus, Geometry, OpenConversation, State};
use faro_core::domain::MessageRef;

/// The input box grows with its text up to this many lines.
const INPUT_MAX_LINES: usize = 6;

pub fn render(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &State,
    view: &View<'_>,
    geometry: &mut Geometry,
) {
    let theme = view.theme;
    let messages_focused = focused(state, geometry, Focus::Messages);
    let input_focused = focused(state, geometry, Focus::Input);
    let Some(open) = &state.open else {
        let block = Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(theme.border(messages_focused));
        let inner = block.inner(area);
        frame.render_widget(block, area);
        let text = if state.loading {
            "Loading…"
        } else {
            "No conversation open"
        };
        let middle = Rect {
            y: inner.y + inner.height / 2,
            height: 1.min(inner.height),
            ..inner
        };
        frame.render_widget(
            Paragraph::new(Span::styled(text, theme.muted())).alignment(Alignment::Center),
            middle,
        );
        return;
    };

    let title = state.conversation_title(open.id);
    let title = truncate(&sanitize(&title), usize::from(area.width.saturating_sub(4))).into_owned();
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(theme.border(messages_focused))
        .title(Span::styled(
            format!(" {title} "),
            Style::new().fg(theme.primary).add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let input_width = usize::from(inner.width.saturating_sub(4));
    let input_layout = layout_input(&open.input, input_width);
    let reply_rows = u16::from(open.reply_to.is_some());
    let input_height = u16::try_from(input_layout.lines.len().clamp(1, INPUT_MAX_LINES))
        .unwrap_or(1)
        + 2
        + reply_rows;
    let [history, typing, input] = Layout::vertical([
        Constraint::Fill(1),
        Constraint::Length(1),
        Constraint::Length(input_height),
    ])
    .areas(inner);

    geometry.history = Some(history);
    geometry.input = Some(input);
    render_history(
        frame,
        history,
        state,
        open,
        view,
        geometry,
        messages_focused,
    );
    render_typing(frame, typing, state, open, view);
    render_input(
        frame,
        input,
        state,
        open,
        &input_layout,
        view,
        input_focused,
    );
}

fn render_history(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &State,
    open: &OpenConversation,
    view: &View<'_>,
    geometry: &mut Geometry,
    focused: bool,
) {
    let outer = area;
    let area = Rect {
        x: area.x + 1,
        width: area.width.saturating_sub(2),
        ..area
    };
    geometry.at_top = true;
    let height = usize::from(area.height);
    if open.loading && open.messages.is_empty() {
        let p = Paragraph::new(Span::styled("Loading…", view.theme.muted()));
        frame.render_widget(p.alignment(Alignment::Center), area);
        return;
    }
    if open.messages.is_empty() {
        let p = Paragraph::new(Span::styled("No messages yet", view.theme.muted()));
        frame.render_widget(p.alignment(Alignment::Center), area);
        return;
    }
    let window = messages::window(state, open, view, usize::from(area.width), height);
    geometry.scroll = window.scroll;
    geometry.at_top = window.at_top;
    let mut lines = window.lines;
    let mut rows = window.rows;
    // Chat convention: the newest message sits just above the input.
    let blank = height.saturating_sub(lines.len());
    lines.splice(0..0, std::iter::repeat_n(Line::default(), blank));
    rows.splice(0..0, std::iter::repeat_n(None, blank));
    frame.render_widget(Paragraph::new(lines), area);
    let selected = open.selected.map(|s| s.message).filter(|_| focused);
    if let Some(selected) = selected {
        highlight(frame, outer, &rows, selected, view);
    }
    geometry.history_rows = rows;
    if window.scroll.is_some() {
        render_more_below(frame, area, open, view);
    }
}

/// The selected message's rows: a tinted background and a bar in the left
/// margin.
fn highlight(
    frame: &mut Frame<'_>,
    area: Rect,
    rows: &[Option<MessageRef>],
    selected: MessageRef,
    view: &View<'_>,
) {
    let tint = Style::new().bg(view.theme.background_element);
    let bar = Style::new().fg(view.theme.primary);
    for (i, row) in rows.iter().enumerate() {
        if *row != Some(selected) {
            continue;
        }
        let Ok(dy) = u16::try_from(i) else {
            break;
        };
        let line = Rect {
            y: area.y + dy,
            height: 1,
            ..area
        };
        frame.buffer_mut().set_style(line, tint);
        if let Some(cell) = frame.buffer_mut().cell_mut((line.x, line.y)) {
            cell.set_symbol("▌").set_style(bar);
        }
    }
}

/// While scrolled up: a badge on the bottom row saying there's more below.
fn render_more_below(frame: &mut Frame<'_>, area: Rect, open: &OpenConversation, view: &View<'_>) {
    let text = match open.unseen {
        0 => " ↓ more below ".to_owned(),
        1 => " ↓ 1 new message ".to_owned(),
        n => format!(" ↓ {n} new messages "),
    };
    let width = u16::try_from(text.width())
        .unwrap_or(u16::MAX)
        .min(area.width);
    let Some(row) = area.height.checked_sub(1) else {
        return;
    };
    let badge = Rect {
        x: area.x + area.width - width,
        y: area.y + row,
        width,
        height: 1,
    };
    let style = if open.unseen > 0 {
        Style::new()
            .fg(view.theme.background)
            .bg(view.theme.unread)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::new()
            .fg(view.theme.text)
            .bg(view.theme.background_element)
    };
    frame.render_widget(Paragraph::new(Span::styled(text, style)), badge);
}

fn render_typing(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &State,
    open: &OpenConversation,
    view: &View<'_>,
) {
    let names: Vec<String> = state
        .typists(open.id)
        .iter()
        .map(|t| sanitize(&state.person_name(t.who)).into_owned())
        .collect();
    let text = match names.as_slice() {
        [] => return,
        [one] => format!("{one} is typing…"),
        [a, b] => format!("{a} and {b} are typing…"),
        [..] => "Several people are typing…".to_owned(),
    };
    let text = truncate(&text, usize::from(area.width.saturating_sub(2))).into_owned();
    let style = view.theme.muted().add_modifier(Modifier::ITALIC);
    frame.render_widget(
        Paragraph::new(Line::from(vec![Span::raw(" "), Span::styled(text, style)])),
        area,
    );
}

fn render_input(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &State,
    open: &OpenConversation,
    layout: &InputLayout,
    view: &View<'_>,
    focused: bool,
) {
    let theme = view.theme;
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(theme.border(focused))
        .style(theme.panel())
        .padding(Padding::horizontal(1));
    let mut inner = block.inner(area);
    frame.render_widget(block, area);

    if let Some(quote) = &open.reply_to {
        let who = sanitize(&state.person_name(quote.target.sender)).into_owned();
        let preview = quote.preview.as_ref().map_or_else(
            || "…".to_owned(),
            |p| sanitize(p.expose()).replace('\n', " "),
        );
        let text = format!("↳ Replying to {who}: {preview}");
        let hint = "  esc cancel";
        let room = usize::from(inner.width).saturating_sub(hint.width());
        let line = Line::from(vec![
            Span::styled(
                truncate(&text, room).into_owned(),
                theme.muted().add_modifier(Modifier::ITALIC),
            ),
            Span::styled(hint, theme.muted()),
        ]);
        frame.render_widget(Paragraph::new(line), Rect { height: 1, ..inner });
        inner.y += 1;
        inner.height = inner.height.saturating_sub(1);
    }

    if open.input.is_empty() {
        let name = sanitize(&state.conversation_title(open.id)).into_owned();
        let hint = truncate(&format!("Message {name}…"), usize::from(inner.width)).into_owned();
        frame.render_widget(Paragraph::new(Span::styled(hint, theme.muted())), inner);
        if focused {
            frame.set_cursor_position(Position::new(inner.x, inner.y));
        }
        return;
    }
    // Keep the cursor's line in view.
    let visible = usize::from(inner.height).max(1);
    let (row, column) = layout.cursor;
    let skip = (row + 1).saturating_sub(visible);
    let shown: Vec<Line<'static>> = layout
        .lines
        .iter()
        .skip(skip)
        .take(visible)
        .map(|l| Line::from(l.clone()))
        .collect();
    let cursor_row = row - skip;
    let cursor_col = column;
    frame.render_widget(Paragraph::new(shown), inner);
    let x = inner.x
        + u16::try_from(cursor_col)
            .unwrap_or(0)
            .min(inner.width.saturating_sub(1));
    let y = inner.y + u16::try_from(cursor_row).unwrap_or(0);
    if focused {
        frame.set_cursor_position(Position::new(x, y));
    }
}

/// The input wrapped to the box, and where the cursor lands.
struct InputLayout {
    lines: Vec<String>,
    /// Row and display column of the cursor.
    cursor: (usize, usize),
}

/// Hard-wrap input between graphemes, keeping every character (including
/// trailing spaces) so the cursor can sit after any of them. A cursor at the
/// end of a full line moves to an extra empty line.
fn layout_input(input: &Input, width: usize) -> InputLayout {
    let width = width.max(1);
    // `Input` holds no control characters besides `\n` (see `insert_str`), so
    // byte offsets match the cursor.
    let text = input.text();
    let mut lines = vec![String::new()];
    let mut used = 0;
    let mut cursor = None;
    for (at, g) in text.grapheme_indices(true) {
        if g == "\n" {
            if at == input.cursor() {
                cursor = Some((lines.len() - 1, used));
            }
            lines.push(String::new());
            used = 0;
            continue;
        }
        let w = g.width();
        if used + w > width {
            lines.push(String::new());
            used = 0;
        }
        if at == input.cursor() {
            cursor = Some((lines.len() - 1, used));
        }
        if let Some(line) = lines.last_mut() {
            line.push_str(g);
        }
        used += w;
    }
    let cursor = cursor.unwrap_or_else(|| {
        if used >= width {
            lines.push(String::new());
            (lines.len() - 1, 0)
        } else {
            (lines.len() - 1, used)
        }
    });
    InputLayout { lines, cursor }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(text: &str, width: usize) -> Vec<String> {
        layout_input(&Input::new(text), width).lines
    }

    #[test]
    fn input_wraps_and_keeps_trailing_spaces() {
        assert_eq!(lines("", 5), vec![""]);
        assert_eq!(lines("hello ", 10), vec!["hello "]);
        assert_eq!(lines("abcdefg", 3), vec!["abc", "def", "g"]);
        assert_eq!(lines("abc", 3), vec!["abc", ""]);
        assert_eq!(lines("a\nb", 5), vec!["a", "b"]);
    }

    #[test]
    fn cursor_lands_where_the_text_is_edited() {
        let mut input = Input::new("abcdefg");
        assert_eq!(layout_input(&input, 3).cursor, (2, 1));
        input.home();
        assert_eq!(layout_input(&input, 3).cursor, (0, 0));
        for _ in 0..4 {
            input.right();
        }
        assert_eq!(layout_input(&input, 3).cursor, (1, 1));
        let mut input = Input::new("ab\n🧗x");
        input.left();
        assert_eq!(
            layout_input(&input, 10).cursor,
            (1, 2),
            "after a wide emoji"
        );
        input.up();
        assert_eq!(layout_input(&input, 10).cursor, (0, 2));
    }
}
