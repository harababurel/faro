//! Right sidebar: about the open conversation (DESIGN.md §7.1). Toggled with Alt+I.

use faro_core::domain::ConversationId;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Paragraph};

use faro_core::store::ConversationState;

use super::View;
use super::messages::{to_jiff, zoned};
use super::text::{sanitize, truncate};
use crate::app::state::{Section, State};

pub fn render(frame: &mut Frame<'_>, area: Rect, state: &State, view: &View<'_>) {
    let theme = view.theme;
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(theme.border(false))
        .title(Span::styled(" Info ", theme.muted()));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let Some(open) = &state.open else {
        return;
    };
    let room = usize::from(inner.width.saturating_sub(1));
    let fit = |text: String| truncate(&sanitize(&text), room).into_owned();
    let label = |text: &'static str| Line::from(Span::styled(format!(" {text}"), theme.muted()));
    let value =
        |text: String, style: Style| Line::from(Span::styled(format!(" {}", fit(text)), style));
    let plain = Style::new().fg(theme.text);

    let mut lines = vec![value(
        state.conversation_title(open.id),
        Style::new().fg(theme.primary).add_modifier(Modifier::BOLD),
    )];
    let summary = state.summary(open.id);
    match open.id {
        ConversationId::Direct(_) => {
            lines.push(value("Direct message".to_owned(), theme.muted()));
            if let Some(phone) = summary.and_then(|s| s.phone.as_ref()) {
                lines.push(Line::default());
                lines.push(label("Phone"));
                lines.push(value(phone.expose().to_owned(), plain));
            }
        }
        ConversationId::Group(group) => {
            let members = state.groups.get(&group).map_or(0, |g| g.members.len());
            lines.push(value(format!("Group · {members} members"), theme.muted()));
            if let Some(group) = state.groups.get(&group) {
                lines.push(Line::default());
                lines.push(label("Members"));
                for member in &group.members {
                    let style = theme.sender(*member).remove_modifier(Modifier::BOLD);
                    lines.push(value(state.person_name(*member), style));
                }
            }
        }
    }
    lines.push(Line::default());
    lines.push(label("Notifications"));
    let conversation_state = summary.map(|s| &s.state);
    lines.push(value(notifications(conversation_state, view), plain));
    // Where it's listed, unless that's just among the recent ones.
    let place = summary.and_then(|s| match state.section(s) {
        Section::Pinned => Some("Pinned"),
        Section::Recent => None,
        Section::Archived => Some("Archived"),
        Section::Hidden => Some("Hidden until a message"),
        Section::NotAllowed => Some("Not on the allowlist"),
    });
    if let Some(place) = place {
        lines.push(Line::default());
        lines.push(label("List"));
        lines.push(value(place.to_owned(), plain));
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

/// "On", "Muted", or "Muted until Thu 19:00".
fn notifications(state: Option<&ConversationState>, view: &View<'_>) -> String {
    let Some(until) = state
        .filter(|s| s.muted_at(view.now_timestamp()))
        .and_then(|s| s.muted_until)
    else {
        return "On".to_owned();
    };
    match to_jiff(until).filter(|_| until != ConversationState::MUTED_FOREVER) {
        Some(until) => format!("Muted until {}", zoned(until, view).strftime("%a %H:%M")),
        None => "Muted".to_owned(),
    }
}
