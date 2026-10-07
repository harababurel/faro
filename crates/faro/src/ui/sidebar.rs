//! Left sidebar: the conversation list (DESIGN.md §7.1, §7.7).

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Paragraph};

use faro_core::store::{ConversationSummary, NotifyLevel};

use super::View;
use super::text::{self, sanitize, truncate};
use crate::app::state::{Focus, Geometry, Section, SidebarRow, SidebarView, State};

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
    let title = match (state.list.view, state.list.allowlist) {
        (SidebarView::Chats, false) => " Conversations ",
        (SidebarView::Chats, true) => " Conversations · allowlist ",
        (SidebarView::Archive, _) => " Archive ",
    };
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(theme.border(focused))
        .title(Span::styled(title, title_style));
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
        geometry.sidebar_rows.push(row.kind);
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
    kind: SidebarRow,
    selected: bool,
}

impl Row {
    fn label(text: impl Into<String>, style: Style) -> Self {
        Self {
            line: Line::from(Span::styled(text.into(), style)),
            kind: SidebarRow::Label,
            selected: false,
        }
    }
}

/// The current view's rows (D41). Chats: pinned and recent (headed only when
/// something is pinned), then a row into the archive. Archive: a row back,
/// then archived, hidden and not-allowed conversations under headers.
fn rows(state: &State, view: &View<'_>, width: usize) -> Vec<Row> {
    let theme = view.theme;
    let listed: Vec<&ConversationSummary> = state.visible_conversations().collect();
    let archive = state.list.view == SidebarView::Archive;
    let headed = archive || listed.iter().any(|c| state.section(c) == Section::Pinned);
    let mut rows = Vec::with_capacity(listed.len() + 6);
    if archive {
        rows.push(Row {
            line: Line::from(Span::styled(" ◂ Chats", Style::new().fg(theme.accent))),
            kind: SidebarRow::SwitchView,
            selected: false,
        });
    }
    let mut current = None;
    for summary in &listed {
        let section = state.section(summary);
        if headed && current != Some(section) {
            current = Some(section);
            rows.push(Row::label(section_label(section), theme.muted()));
        }
        rows.push(row(state, view, summary, width));
    }
    if listed.is_empty() {
        let note = match (archive, state.list.allowlist) {
            (true, _) => " Nothing archived or hidden",
            (false, true) => " Allowlist is empty",
            (false, false) => " No conversations yet",
        };
        rows.push(Row::label(note, theme.muted()));
    }
    let elsewhere = state.listed_in(SidebarView::Archive).count();
    if !archive && elsewhere > 0 {
        rows.push(Row::label("", theme.muted()));
        rows.push(Row {
            line: Line::from(vec![
                Span::styled(" ▸ Archive", Style::new().fg(theme.accent)),
                Span::styled(format!(" ({elsewhere})"), theme.muted()),
            ]),
            kind: SidebarRow::SwitchView,
            selected: false,
        });
    }
    rows
}

fn section_label(section: Section) -> &'static str {
    match section {
        Section::Pinned => " ▾ Pinned",
        Section::Recent => " ▾ Recent",
        Section::Archived => " ▾ Archived",
        Section::Hidden => " ▾ Hidden until a message",
        Section::NotAllowed => " ▾ Not on the allowlist",
    }
}

fn row(state: &State, view: &View<'_>, summary: &ConversationSummary, width: usize) -> Row {
    let theme = view.theme;
    let selected = state.open.as_ref().is_some_and(|o| o.id == summary.id);
    let unread = summary.unread > 0;
    let typing = !state.typists(summary.id).is_empty();
    // Muted or mentions only: drawn quietly, as it won't notify (D49).
    let muted = summary.state.muted_at(view.now_timestamp())
        || summary.state.notify == NotifyLevel::Mentions;

    let marker = if unread {
        let style = if muted {
            theme.muted()
        } else {
            Style::new().fg(theme.unread)
        };
        Span::styled(" ● ", style)
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

    let mut name_style = if muted {
        theme.muted()
    } else {
        Style::new().fg(theme.text)
    };
    if unread && !muted {
        name_style = name_style.add_modifier(Modifier::BOLD);
    }
    if selected {
        name_style = name_style
            .fg(theme.primary)
            .remove_modifier(Modifier::DIM)
            .add_modifier(Modifier::BOLD);
    }
    let count_style = if muted {
        theme.muted()
    } else {
        Style::new().fg(theme.unread).add_modifier(Modifier::BOLD)
    };
    let mut line = Line::from(vec![
        marker,
        Span::styled(name, name_style),
        Span::styled(suffix, Style::new().fg(theme.accent)),
        Span::raw(" ".repeat(pad)),
        Span::styled(count, count_style),
    ]);
    if selected {
        line = line.style(theme.selected());
    }
    Row {
        line,
        kind: SidebarRow::Conversation(summary.id),
        selected,
    }
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
