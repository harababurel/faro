//! Right sidebar: about the open conversation (DESIGN.md §7.1). Toggled with Alt+I.

use faro_core::domain::ConversationId;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Paragraph};

use super::View;
use super::text::{sanitize, truncate};
use crate::app::state::State;

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
    let conversation_state = summary.map(|s| &s.state);
    lines.push(Line::default());
    lines.push(label("Notifications"));
    let muted = conversation_state
        .and_then(|s| s.muted_until)
        .is_some_and(|until| {
            i64::try_from(until.as_millis()).unwrap_or(i64::MAX) > view.now.as_millisecond()
        });
    lines.push(value(
        (if muted { "Muted" } else { "On" }).to_owned(),
        plain,
    ));
    frame.render_widget(Paragraph::new(lines), inner);
}
