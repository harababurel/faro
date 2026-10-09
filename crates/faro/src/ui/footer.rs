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
use crate::app::keymap::{Action, Curate};
use crate::app::palette::Level;
use crate::app::state::{Focus, Geometry, NoticeLevel, Popup, SidebarView, State};

/// Key hints for each focus, most important first; dropped from the end when
/// space runs out (DESIGN.md §7.1: context-sensitive). The keys come from the
/// key map, so rebinding shows here (D43).
type Hints = &'static [(&'static [Action], &'static str)];

const INPUT_HINTS: Hints = &[
    (&[Action::Send], "send"),
    (
        &[Action::NextConversation, Action::PreviousConversation],
        "conversations",
    ),
    (&[Action::Escape], "close"),
    (&[Action::FocusNext], "focus"),
    (&[Action::PageUp], "scroll"),
    (&[Action::ToggleLeftSidebar], "sidebar"),
    (&[Action::ToggleRightSidebar], "info"),
    (&[Action::Quit], "quit"),
];
const CONVERSATION_HINTS: Hints = &[
    (
        &[Action::PreviousConversation, Action::NextConversation],
        "choose",
    ),
    (&[Action::FocusInput], "write"),
    (&[Action::Curate(Curate::Menu)], "actions"),
    (&[Action::Curate(Curate::Pin)], "pin"),
    (&[Action::Curate(Curate::Mute)], "mute"),
    (&[Action::Curate(Curate::Hide)], "hide"),
    (&[Action::Curate(Curate::Archive)], "archive"),
    (&[Action::Curate(Curate::SwitchView)], "archive list"),
    (&[Action::FocusNext], "focus"),
];
const ARCHIVE_HINTS: Hints = &[
    (
        &[Action::PreviousConversation, Action::NextConversation],
        "choose",
    ),
    (&[Action::FocusInput], "write"),
    (&[Action::Curate(Curate::Menu)], "actions"),
    (&[Action::Curate(Curate::Archive)], "unarchive"),
    (&[Action::Curate(Curate::Hide)], "unhide"),
    (&[Action::Curate(Curate::SwitchView)], "back to chats"),
    (&[Action::FocusNext], "focus"),
];
const MESSAGE_HINTS: Hints = &[
    (&[Action::SelectOlder, Action::SelectNewer], "select"),
    (&[Action::MessageMenu], "actions"),
    (&[Action::Reply], "reply"),
    (&[Action::ReactPicker], "react"),
    (&[Action::Copy], "copy"),
    (&[Action::OpenLink], "open link"),
    (&[Action::Escape], "back"),
];
/// Shown after the connection status while the bell is off (plain text: a
/// bell-with-a-slash emoji is drawn 1 or 2 cells wide depending on the
/// terminal).
const BELL_OFF: &str = "bell off";
/// Shown after the connection status without an allowlist (D67).
const UNGUARDED: &str = "no allowlist";
/// Popups and the palette have fixed keys (D43).
const PALETTE_HINTS: &[(&str, &str)] = &[
    ("↑↓", "choose"),
    ("enter", "open"),
    ("esc", "close"),
    ("@", "chats"),
    (":", "settings"),
    (">", "commands"),
    ("?", "keys"),
];
const PALETTE_LEVEL_HINTS: &[(&str, &str)] = &[
    ("↑↓", "move"),
    ("enter", "choose"),
    ("←", "back"),
    ("esc", "close"),
];
/// Typing a nickname: the query is the name (D65).
const NICKNAME_HINTS: &[(&str, &str)] = &[("enter", "save"), ("esc", "cancel")];
/// Settings cycle in place with `←`/`→`; `Enter` flips on/off or opens values (D57).
const SETTINGS_HINTS: &[(&str, &str)] = &[
    ("↑↓", "move"),
    ("←→", "cycle"),
    ("enter", "flip/open"),
    ("esc", "close"),
];
/// Moving through themes, or the values of a setting that changes how things
/// look, tries each on (D54, D59); esc puts the old one back.
const PREVIEW_HINTS: &[(&str, &str)] = &[
    ("↑↓", "preview"),
    ("enter", "use it"),
    ("←", "back"),
    ("esc", "cancel"),
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
const REACT_SEARCH_HINTS: &[(&str, &str)] =
    &[("↑↓", "choose"), ("enter", "react"), ("esc", "close")];
const IMAGE_VIEWER_HINTS: &[(&str, &str)] = &[("s", "save"), ("y", "copy path"), ("esc", "close")];
const CONFIRM_DELETE_HINTS: &[(&str, &str)] = &[("enter/y", "delete"), ("esc/n", "cancel")];
const MESSAGE_INFO_HINTS: &[(&str, &str)] = &[("esc", "close")];

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
    if state.guard == crate::app::state::Guard::Off {
        left.push(Span::styled(" · ", theme.muted()));
        left.push(Span::styled(UNGUARDED, theme.muted()));
    }
    if !state.config.notifications.bell {
        // Do not disturb (D53): say so while it lasts.
        left.push(Span::styled(" · ", theme.muted()));
        left.push(Span::styled(BELL_OFF, Style::new().fg(theme.warning)));
    }
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
        render_hints(&hints(state, geometry), room, view)
    };
    frame.render_widget(Paragraph::new(right).alignment(Alignment::Right), area);
}

/// The hints for what has the keys now, with the keys spelled out.
fn hints(state: &State, geometry: &Geometry) -> Vec<(String, &'static str)> {
    let fixed = |hints: &[(&str, &'static str)]| {
        hints
            .iter()
            .map(|&(key, action)| (key.to_owned(), action))
            .collect()
    };
    if let Some(palette) = &state.palette {
        return fixed(match palette.level() {
            Level::Root => PALETTE_HINTS,
            Level::Themes => PREVIEW_HINTS,
            Level::Setting(setting) if setting.previews() => PREVIEW_HINTS,
            Level::Settings => SETTINGS_HINTS,
            Level::Nickname(_) => NICKNAME_HINTS,
            Level::Chats
            | Level::Setting(_)
            | Level::Commands
            | Level::Keys
            | Level::People
            | Level::AttachFile => PALETTE_LEVEL_HINTS,
        });
    }
    if let Some(popup) = &state.popup {
        return match popup {
            Popup::Actions { .. }
            | Popup::Conversation { .. }
            | Popup::ConfirmRemoteTranscription { .. } => fixed(MENU_HINTS),
            Popup::ConfirmDelete { .. } => fixed(CONFIRM_DELETE_HINTS),
            Popup::React { query: None, .. } => fixed(REACT_HINTS),
            Popup::React { query: Some(_), .. } => fixed(REACT_SEARCH_HINTS),
            Popup::Mute { .. } => fixed(MUTE_HINTS),
            Popup::MessageInfo { .. } => fixed(MESSAGE_INFO_HINTS),
            Popup::ImageViewer { .. } => fixed(IMAGE_VIEWER_HINTS),
        };
    }
    let (focus, hints) = if focused(state, geometry, Focus::Messages) {
        (Focus::Messages, MESSAGE_HINTS)
    } else if focused(state, geometry, Focus::Conversations) {
        let hints = match state.list.view {
            SidebarView::Chats => CONVERSATION_HINTS,
            SidebarView::Archive => ARCHIVE_HINTS,
        };
        (Focus::Conversations, hints)
    } else {
        (Focus::Input, INPUT_HINTS)
    };
    // An action with no key (unbound in the config) has no hint. With
    // nothing open, `Enter` in the list opens the highlighted one (D60).
    hints
        .iter()
        .filter_map(|&(actions, label)| {
            let label = match (actions, state.open.is_none()) {
                ([Action::FocusInput], true) => "open",
                _ => label,
            };
            Some((state.keymap.hint(actions, focus)?, label))
        })
        .collect()
}

fn render_hints(hints: &[(String, &'static str)], room: usize, view: &View<'_>) -> Line<'static> {
    let theme = view.theme;
    let mut spans = Vec::new();
    let mut used = 1;
    for (key, action) in hints {
        let sep = if spans.is_empty() { "" } else { " · " };
        let w = width(sep) + width(key) + 1 + width(action);
        if used + w > room {
            break;
        }
        used += w;
        spans.push(Span::styled(sep, theme.muted()));
        spans.push(Span::styled(key.clone(), Style::new().fg(theme.text)));
        spans.push(Span::styled(format!(" {action}"), theme.muted()));
    }
    spans.push(Span::raw(" "));
    Line::from(spans)
}
