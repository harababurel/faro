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
use super::{View, messages};
use crate::app::state::{OpenConversation, State};

/// The input box grows with its text up to this many lines.
const INPUT_MAX_LINES: usize = 6;

pub fn render(frame: &mut Frame<'_>, area: Rect, state: &State, view: &View<'_>) {
    let theme = view.theme;
    let Some(open) = &state.open else {
        let block = Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(theme.border(false));
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
        .border_style(theme.border(false))
        .title(Span::styled(
            format!(" {title} "),
            Style::new().fg(theme.primary).add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let input_width = usize::from(inner.width.saturating_sub(4));
    let input_lines = input_lines(&open.input, input_width);
    let input_height = u16::try_from(input_lines.len().clamp(1, INPUT_MAX_LINES)).unwrap_or(1) + 2;
    let [history, typing, input] = Layout::vertical([
        Constraint::Fill(1),
        Constraint::Length(1),
        Constraint::Length(input_height),
    ])
    .areas(inner);

    render_history(frame, history, state, open, view);
    render_typing(frame, typing, state, open, view);
    render_input(frame, input, state, open, &input_lines, view);
}

fn render_history(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &State,
    open: &OpenConversation,
    view: &View<'_>,
) {
    let area = Rect {
        x: area.x + 1,
        width: area.width.saturating_sub(2),
        ..area
    };
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
    let mut lines = messages::lines(state, open, view, usize::from(area.width), height);
    // Chat convention: the newest message sits just above the input.
    let blank = height.saturating_sub(lines.len());
    lines.splice(0..0, std::iter::repeat_n(Line::default(), blank));
    frame.render_widget(Paragraph::new(lines), area);
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
    lines: &[String],
    view: &View<'_>,
) {
    let theme = view.theme;
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(theme.border(true))
        .style(theme.panel())
        .padding(Padding::horizontal(1));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if open.input.is_empty() {
        let name = sanitize(&state.conversation_title(open.id)).into_owned();
        let hint = truncate(&format!("Message {name}…"), usize::from(inner.width)).into_owned();
        frame.render_widget(Paragraph::new(Span::styled(hint, theme.muted())), inner);
        frame.set_cursor_position(Position::new(inner.x, inner.y));
        return;
    }
    let visible = usize::from(inner.height);
    let skip = lines.len().saturating_sub(visible);
    let shown: Vec<Line<'static>> = lines
        .iter()
        .skip(skip)
        .map(|l| Line::from(l.clone()))
        .collect();
    let cursor_row = shown.len().saturating_sub(1);
    let cursor_col = lines.last().map_or(0, |l| l.width());
    frame.render_widget(Paragraph::new(shown), inner);
    let x = inner.x
        + u16::try_from(cursor_col)
            .unwrap_or(0)
            .min(inner.width.saturating_sub(1));
    let y = inner.y + u16::try_from(cursor_row).unwrap_or(0);
    frame.set_cursor_position(Position::new(x, y));
}

/// Hard-wrap input between graphemes, keeping every character (including
/// trailing spaces) so the cursor lands right after the last one. A full last
/// line gets an empty line after it for the cursor.
fn input_lines(input: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut lines = vec![String::new()];
    let mut used = 0;
    for g in sanitize_keep_newlines(input).graphemes(true) {
        if g == "\n" {
            lines.push(String::new());
            used = 0;
            continue;
        }
        let w = g.width();
        if used + w > width {
            lines.push(String::new());
            used = 0;
        }
        if let Some(line) = lines.last_mut() {
            line.push_str(g);
        }
        used += w;
    }
    if used >= width {
        lines.push(String::new());
    }
    lines
}

fn sanitize_keep_newlines(text: &str) -> String {
    text.split('\n')
        .map(|part| sanitize(part).into_owned())
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_wraps_and_keeps_trailing_spaces() {
        assert_eq!(input_lines("", 5), vec![""]);
        assert_eq!(input_lines("hello ", 10), vec!["hello "]);
        assert_eq!(input_lines("abcdefg", 3), vec!["abc", "def", "g"]);
        assert_eq!(input_lines("abc", 3), vec!["abc", ""]);
        assert_eq!(input_lines("a\nb", 5), vec!["a", "b"]);
    }
}
