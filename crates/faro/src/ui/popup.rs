//! Popups (D40, D41): the message action menu and the reaction picker over
//! the message area; the conversation menu and the mute picker next to the
//! sidebar. Drawn last, over everything else.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Paragraph};

use super::View;
use super::text::{pad_left, truncate, width};
use faro_core::domain::{ConversationId, MessageRef};

use crate::app::state::{
    Geometry, MUTE_CHOICES, MessageAction, Popup, REACTIONS, SidebarRow, State,
};

/// Shortcut keys of `MUTE_CHOICES`.
const MUTE_KEYS: [&str; 5] = ["1", "2", "3", "4", "5"];

pub fn render(frame: &mut Frame<'_>, state: &State, view: &View<'_>, geometry: &Geometry) {
    let Some(popup) = &state.popup else {
        return;
    };
    match popup {
        Popup::Actions {
            target,
            items,
            selected,
        } => {
            let Some(anchor) = over_message(geometry, *target) else {
                return;
            };
            let labels: Vec<(String, &str)> = items.iter().map(MessageAction::label).collect();
            menu(frame, anchor, "Message", &labels, *selected, view);
        }
        Popup::React { target, selected } => {
            let Some(anchor) = over_message(geometry, *target) else {
                return;
            };
            let mine = state
                .open
                .as_ref()
                .and_then(|o| o.messages.iter().rfind(|m| m.reference() == *target))
                .and_then(|m| m.reactions.iter().find(|r| r.sender == state.me))
                .map(|r| r.emoji.as_str());
            react(frame, anchor, *selected, mine, view);
        }
        Popup::Conversation {
            target,
            items,
            selected,
        } => {
            let Some(anchor) = beside_sidebar(frame.area(), geometry, *target) else {
                return;
            };
            let labels: Vec<(String, &str)> = items
                .iter()
                .map(|a| {
                    let (label, key) = a.label();
                    (label.to_owned(), key)
                })
                .collect();
            menu(frame, anchor, "Conversation", &labels, *selected, view);
        }
        Popup::Mute { target, selected } => {
            let Some(anchor) = beside_sidebar(frame.area(), geometry, *target) else {
                return;
            };
            let labels: Vec<(String, &str)> = MUTE_CHOICES
                .iter()
                .zip(MUTE_KEYS)
                .map(|((label, _), key)| ((*label).to_owned(), key))
                .collect();
            menu(frame, anchor, "Mute", &labels, *selected, view);
        }
    }
}

/// In the message area, next to the rows of the message `target` (so the
/// popup doesn't cover them).
fn over_message(geometry: &Geometry, target: MessageRef) -> Option<Anchor> {
    let area = geometry.history?;
    let mut rows = geometry
        .history_rows
        .iter()
        .enumerate()
        .filter(|(_, m)| **m == Some(target))
        .map(|(i, _)| i);
    let first = rows.next();
    let around = first.map(|first| (first, rows.next_back().unwrap_or(first)));
    Some(Anchor { area, around })
}

/// Next to `target`'s sidebar row, over whatever is right of the sidebar's
/// left edge; at the bottom of the message area if the sidebar is hidden.
fn beside_sidebar(screen: Rect, geometry: &Geometry, target: ConversationId) -> Option<Anchor> {
    let Some(sidebar) = geometry.sidebar else {
        return geometry.history.map(|area| Anchor { area, around: None });
    };
    let row = geometry
        .sidebar_rows
        .iter()
        .position(|r| *r == SidebarRow::Conversation(target));
    Some(Anchor {
        area: Rect {
            width: screen.width.saturating_sub(sidebar.x),
            ..sidebar
        },
        around: row.map(|r| (r, r)),
    })
}

/// Where a popup goes: inside `area`, next to the rows `around` (first and
/// last, from the top of `area`) if known.
#[derive(Clone, Copy)]
struct Anchor {
    area: Rect,
    around: Option<(usize, usize)>,
}

/// A bordered box for `inner_width` x `inner_height`: just above the message
/// it's about, else just below it, else at the bottom of the area.
fn place(anchor: Anchor, inner_width: usize, inner_height: usize) -> Rect {
    let area = anchor.area;
    let width = u16::try_from(inner_width + 2)
        .unwrap_or(u16::MAX)
        .min(area.width.saturating_sub(2));
    let height = u16::try_from(inner_height + 2)
        .unwrap_or(u16::MAX)
        .min(area.height);
    let bottom_y = area.y + area.height - height;
    let y = anchor.around.map_or(bottom_y, |(first, last)| {
        let first = u16::try_from(first).unwrap_or(u16::MAX);
        let below = u16::try_from(last + 1).unwrap_or(u16::MAX);
        if first >= height {
            area.y + first - height
        } else if below.saturating_add(height) <= area.height {
            area.y + below
        } else {
            bottom_y
        }
    });
    Rect {
        x: area.x + 1,
        y,
        width,
        height,
    }
}

fn frame_box(frame: &mut Frame<'_>, rect: Rect, title: &str, view: &View<'_>) -> Rect {
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(view.theme.border(true))
        .style(view.theme.panel())
        .title(Span::styled(
            format!(" {title} "),
            Style::new()
                .fg(view.theme.primary)
                .add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(rect);
    frame.render_widget(Clear, rect);
    frame.render_widget(block, rect);
    inner
}

/// A list of labelled items with their shortcut keys, `selected` marked.
fn menu(
    frame: &mut Frame<'_>,
    anchor: Anchor,
    title: &str,
    labels: &[(String, &str)],
    selected: usize,
    view: &View<'_>,
) {
    let theme = view.theme;
    // " › label   key "
    let widest = labels
        .iter()
        .map(|(label, key)| width(label) + width(key))
        .max()
        .unwrap_or(0);
    let max_inner = usize::from(anchor.area.width.saturating_sub(4));
    let inner_width = (widest + 6).clamp(20, max_inner.max(20));
    let rect = place(anchor, inner_width, labels.len());
    let inner = frame_box(frame, rect, title, view);
    let row_width = usize::from(inner.width);
    let lines: Vec<Line<'static>> = labels
        .iter()
        .enumerate()
        .map(|(i, (label, key))| {
            let chosen = i == selected;
            let marker = if chosen { " › " } else { "   " };
            let key_width = width(key) + 1;
            let room = row_width.saturating_sub(3 + key_width);
            let label = truncate(label, room).into_owned();
            let gap = row_width.saturating_sub(3 + width(&label) + key_width);
            let style = if chosen {
                Style::new()
                    .fg(theme.text)
                    .bg(theme.background_element)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::new().fg(theme.text)
            };
            Line::from(vec![
                Span::styled(marker, style.fg(theme.primary)),
                Span::styled(label, style),
                Span::styled(" ".repeat(gap), style),
                Span::styled(pad_left(key, key_width), style.fg(theme.text_muted)),
            ])
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), inner);
}

fn react(
    frame: &mut Frame<'_>,
    anchor: Anchor,
    selected: usize,
    mine: Option<&str>,
    view: &View<'_>,
) {
    let theme = view.theme;
    let mut spans = vec![Span::raw(" ")];
    for (i, emoji) in REACTIONS.iter().enumerate() {
        let mut style = Style::new().fg(theme.text);
        if Some(*emoji) == mine {
            style = style.add_modifier(Modifier::UNDERLINED);
        }
        if i == selected {
            style = style
                .bg(theme.background_element)
                .add_modifier(Modifier::BOLD);
        }
        spans.push(Span::styled(
            format!("{} ", i + 1),
            style.fg(theme.text_muted),
        ));
        spans.push(Span::styled(format!("{emoji} "), style));
        spans.push(Span::raw(" "));
    }
    let line = Line::from(spans);
    let rect = place(anchor, line.width(), 1);
    let inner = frame_box(frame, rect, "React", view);
    frame.render_widget(Paragraph::new(line), inner);
}
