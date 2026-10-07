//! Left sidebar: the conversation list (DESIGN.md §7.1, §7.7).

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Paragraph};

use faro_core::domain::ConversationId;
use faro_core::store::ConversationSummary;

use super::View;
use super::text::{self, sanitize, truncate};
use crate::app::state::{Focus, Geometry, State};

pub fn render(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &State,
    view: &View<'_>,
    geometry: &mut Geometry,
) {
    let theme = view.theme;
    let focused = state.focus == Focus::Conversations;
    let title_style = if focused {
        Style::new().fg(theme.primary).add_modifier(Modifier::BOLD)
    } else {
        theme.muted()
    };
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(theme.border(focused))
        .title(Span::styled(" Conversations ", title_style));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    geometry.sidebar = Some(inner);

    if state.loading {
        frame.render_widget(
            Paragraph::new(Span::styled(" Loading…", theme.muted())),
            inner,
        );
        return;
    }
    let rows = rows(state, view, usize::from(inner.width));
    if rows.is_empty() {
        let empty = Paragraph::new(Span::styled(" No conversations yet", theme.muted()));
        frame.render_widget(empty, inner);
        return;
    }
    let height = usize::from(inner.height);
    let selected = rows.iter().position(|r| r.selected);
    let offset = visible_offset(
        state.sidebar.offset,
        rows.len(),
        height,
        selected,
        state.sidebar.reveal,
    );
    geometry.sidebar_offset = offset;
    let mut lines = Vec::with_capacity(height);
    for row in rows.into_iter().skip(offset).take(height) {
        geometry.sidebar_rows.push(row.id);
        lines.push(row.line);
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

/// The first row to show: the requested offset, clamped so the list doesn't
/// scroll past its end, and moved just enough to show the selection when asked.
fn visible_offset(
    requested: usize,
    rows: usize,
    height: usize,
    selected: Option<usize>,
    reveal: bool,
) -> usize {
    let mut offset = requested.min(rows.saturating_sub(height));
    if reveal && let Some(selected) = selected {
        if selected < offset {
            offset = selected;
        } else if selected >= offset + height {
            offset = (selected + 1).saturating_sub(height);
        }
    }
    offset
}

struct Row {
    line: Line<'static>,
    /// `None` for section headers.
    id: Option<ConversationId>,
    selected: bool,
}

fn rows(state: &State, view: &View<'_>, width: usize) -> Vec<Row> {
    let theme = view.theme;
    let visible: Vec<&ConversationSummary> = state.visible_conversations().collect();
    let pinned = visible.iter().any(|c| c.state.pinned.is_some());
    let mut rows = Vec::with_capacity(visible.len() + 2);
    let mut in_pinned = false;
    for (i, summary) in visible.iter().enumerate() {
        if pinned && (i == 0 || (in_pinned && summary.state.pinned.is_none())) {
            in_pinned = summary.state.pinned.is_some();
            let label = if in_pinned {
                " ▾ Pinned"
            } else {
                " ▾ Recent"
            };
            rows.push(Row {
                line: Line::from(Span::styled(label, theme.muted())),
                id: None,
                selected: false,
            });
        }
        rows.push(row(state, view, summary, width));
    }
    rows
}

fn row(state: &State, view: &View<'_>, summary: &ConversationSummary, width: usize) -> Row {
    let theme = view.theme;
    let selected = state.open.as_ref().is_some_and(|o| o.id == summary.id);
    let unread = summary.unread > 0;
    let typing = !state.typists(summary.id).is_empty();
    let muted = summary
        .state
        .muted_until
        .is_some_and(|until| until.as_millis() > now_millis(view));

    let marker = if unread {
        Span::styled(" ● ", Style::new().fg(theme.unread))
    } else {
        Span::raw("   ")
    };
    let count = if unread {
        format!(" {} ", summary.unread)
    } else {
        String::new()
    };
    let suffix = if typing { " …" } else { "" };
    let room = width.saturating_sub(3 + text::width(&count) + text::width(suffix));
    let title = state.conversation_title(summary.id);
    let name = truncate(&sanitize(&title), room).into_owned();
    let pad = room.saturating_sub(text::width(&name));

    let mut name_style = Style::new().fg(if muted { theme.text_muted } else { theme.text });
    if unread {
        name_style = name_style.add_modifier(Modifier::BOLD);
    }
    if selected {
        name_style = name_style.fg(theme.primary).add_modifier(Modifier::BOLD);
    }
    let mut line = Line::from(vec![
        marker,
        Span::styled(name, name_style),
        Span::styled(suffix, Style::new().fg(theme.accent)),
        Span::raw(" ".repeat(pad)),
        Span::styled(
            count,
            Style::new().fg(theme.unread).add_modifier(Modifier::BOLD),
        ),
    ]);
    if selected {
        line = line.style(Style::new().bg(theme.background_element));
    }
    Row {
        line,
        id: Some(summary.id),
        selected,
    }
}

fn now_millis(view: &View<'_>) -> u64 {
    u64::try_from(view.now.as_millisecond()).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::visible_offset;

    #[test]
    fn offset_is_clamped_and_reveals_the_selection() {
        // 20 rows, 5 visible.
        assert_eq!(visible_offset(0, 20, 5, Some(2), true), 0);
        assert_eq!(visible_offset(0, 20, 5, Some(9), true), 5);
        assert_eq!(visible_offset(8, 20, 5, Some(2), true), 2);
        // Scrolled away with the wheel: the selection may be off screen.
        assert_eq!(visible_offset(8, 20, 5, Some(2), false), 8);
        // Never past the end.
        assert_eq!(visible_offset(99, 20, 5, None, false), 15);
        assert_eq!(visible_offset(3, 4, 5, None, false), 0);
    }
}
