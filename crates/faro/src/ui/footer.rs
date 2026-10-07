//! Footer: connection status and unread total on the left, key hints (or a
//! notice) on the right (DESIGN.md §7.1).

use faro_core::domain::ConnectionState;
use ratatui::Frame;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use super::text::{sanitize, truncate, width};
use super::{View, focused};
use crate::app::state::{Focus, Geometry, NoticeLevel, Popup, SidebarView, State};

/// Key hints for each focus, most important first; dropped from the end when
/// space runs out (DESIGN.md §7.1: context-sensitive).
const INPUT_HINTS: &[(&str, &str)] = &[
    ("enter", "send"),
    ("alt+j/k", "conversations"),
    ("tab", "focus"),
    ("pgup", "scroll"),
    ("alt+s", "sidebar"),
    ("alt+i", "info"),
    ("ctrl+q", "quit"),
];
const CONVERSATION_HINTS: &[(&str, &str)] = &[
    ("↑↓", "choose"),
    ("enter", "write"),
    ("space", "actions"),
    ("p", "pin"),
    ("m", "mute"),
    ("h", "hide"),
    ("a", "archive"),
    ("A", "archive list"),
    ("tab", "focus"),
];
const ARCHIVE_HINTS: &[(&str, &str)] = &[
    ("↑↓", "choose"),
    ("enter", "write"),
    ("space", "actions"),
    ("a", "unarchive"),
    ("h", "unhide"),
    ("A", "back to chats"),
    ("tab", "focus"),
];
const MESSAGE_HINTS: &[(&str, &str)] = &[
    ("↑↓", "select"),
    ("enter", "actions"),
    ("r", "reply"),
    ("+", "react"),
    ("y", "copy"),
    ("o", "open link"),
    ("esc", "back"),
];
const MENU_HINTS: &[(&str, &str)] = &[("↑↓", "choose"), ("enter", "do it"), ("esc", "close")];
const MUTE_HINTS: &[(&str, &str)] = &[
    ("↑↓", "choose"),
    ("1-5", "pick"),
    ("enter", "mute"),
    ("esc", "close"),
];
const REACT_HINTS: &[(&str, &str)] = &[
    ("←→", "choose"),
    ("1-6", "pick"),
    ("enter", "react"),
    ("esc", "close"),
];

pub fn render(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &State,
    view: &View<'_>,
    geometry: &Geometry,
) {
    let theme = view.theme;
    let (glyph, label, colour) = match state.connection {
        ConnectionState::Connected => ("●", "connected", theme.success),
        ConnectionState::Connecting => ("◌", "connecting…", theme.warning),
        ConnectionState::Disconnected => ("○", "offline, retrying", theme.error),
        ConnectionState::Unlinked => ("✗", "unlinked", theme.error),
    };
    let mut left = vec![
        Span::raw(" "),
        Span::styled(glyph, Style::new().fg(colour)),
        Span::styled(format!(" {label}"), theme.muted()),
    ];
    let unread = state.unread_total(view.now_timestamp());
    if unread > 0 {
        left.push(Span::styled(" · ", theme.muted()));
        left.push(Span::styled(
            format!("{unread} unread"),
            Style::new().fg(theme.unread).add_modifier(Modifier::BOLD),
        ));
    }
    let left_width: usize = left.iter().map(|s| width(&s.content)).sum();
    frame.render_widget(Paragraph::new(Line::from(left)), area);

    let room = usize::from(area.width).saturating_sub(left_width + 2);
    let right = if let Some(notice) = &state.notice {
        let colour = match notice.level {
            NoticeLevel::Info => theme.success,
            NoticeLevel::Warning => theme.warning,
            NoticeLevel::Error => theme.error,
        };
        Line::from(Span::styled(
            format!(
                "{} ",
                truncate(&sanitize(&notice.text), room.saturating_sub(1))
            ),
            Style::new().fg(colour),
        ))
    } else {
        let hints = if let Some(popup) = &state.popup {
            match popup {
                Popup::Actions { .. } | Popup::Conversation { .. } => MENU_HINTS,
                Popup::React { .. } => REACT_HINTS,
                Popup::Mute { .. } => MUTE_HINTS,
            }
        } else if focused(state, geometry, Focus::Messages) {
            MESSAGE_HINTS
        } else if focused(state, geometry, Focus::Conversations) {
            match state.list.view {
                SidebarView::Chats => CONVERSATION_HINTS,
                SidebarView::Archive => ARCHIVE_HINTS,
            }
        } else {
            INPUT_HINTS
        };
        render_hints(hints, room, view)
    };
    frame.render_widget(Paragraph::new(right).alignment(Alignment::Right), area);
}

fn render_hints(
    hints: &[(&'static str, &'static str)],
    room: usize,
    view: &View<'_>,
) -> Line<'static> {
    let theme = view.theme;
    let mut spans = Vec::new();
    let mut used = 1;
    for &(key, action) in hints {
        let sep = if spans.is_empty() { "" } else { " · " };
        let w = width(sep) + width(key) + 1 + width(action);
        if used + w > room {
            break;
        }
        used += w;
        spans.push(Span::styled(sep, theme.muted()));
        spans.push(Span::styled(key, Style::new().fg(theme.text)));
        spans.push(Span::styled(format!(" {action}"), theme.muted()));
    }
    spans.push(Span::raw(" "));
    Line::from(spans)
}
