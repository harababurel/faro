//! Full-screen lock and passphrase prompt card (D168).

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};

use super::View;
use crate::app::state::{LockScreen, State};
use crate::theme::Theme;

const CARD_WIDTH: u16 = 56;
const CARD_HEIGHT: u16 = 9;

/// Render the in-session lock or passphrase-setup screen over `area`.
pub fn render(
    frame: &mut Frame<'_>,
    area: Rect,
    screen: &LockScreen,
    state: &State,
    view: &View<'_>,
) {
    let unread = state.unread_total(view.now_timestamp());
    match screen {
        LockScreen::Locked { input, error } => {
            let status = if error.is_none() && unread > 0 {
                Some(if unread == 1 {
                    "1 unread message".to_owned()
                } else {
                    format!("{unread} unread messages")
                })
            } else {
                None
            };
            render_prompt_card(
                frame,
                area,
                view.theme,
                " faro locked ",
                "Enter your passphrase to unlock:",
                input.chars().count(),
                error.as_deref(),
                status.as_deref(),
                "enter unlock · ctrl+q quit",
            );
        }
        LockScreen::SetPassphrase {
            input,
            confirm,
            error,
        } => {
            let prompt = if confirm.is_some() {
                "Confirm your session passphrase:"
            } else {
                "Set a passphrase for locking this session:"
            };
            render_prompt_card(
                frame,
                area,
                view.theme,
                " lock screen ",
                prompt,
                input.chars().count(),
                error.as_deref(),
                Some("For full disk encryption at rest, run `faro encrypt`."),
                "enter continue · esc cancel",
            );
        }
    }
}

/// Render a centered passphrase card (shared by the in-session lock screen and
/// the startup vault unlock prompt).
#[expect(
    clippy::too_many_arguments,
    reason = "card parameters cover title, prompt, masked length, error/status, and key hints"
)]
pub fn render_prompt_card(
    frame: &mut Frame<'_>,
    area: Rect,
    theme: &Theme,
    title: &str,
    prompt: &str,
    input_chars: usize,
    error: Option<&str>,
    note: Option<&str>,
    hints: &str,
) {
    if area.width < 16 || area.height < 5 {
        return;
    }
    let width = CARD_WIDTH.min(area.width.saturating_sub(2)).max(16);
    let height = CARD_HEIGHT.min(area.height).max(5);
    let card = Rect {
        x: area.x + (area.width.saturating_sub(width)) / 2,
        y: area.y + (area.height.saturating_sub(height)) / 2,
        width,
        height,
    };

    frame.render_widget(Clear, card);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(theme.border_active))
        .style(theme.panel())
        .title(Span::styled(
            title,
            Style::new().fg(theme.accent).add_modifier(Modifier::BOLD),
        ))
        .title_alignment(Alignment::Center);
    let inner = block.inner(card);
    frame.render_widget(block, card);

    if inner.height == 0 || inner.width == 0 {
        return;
    }

    let max_dots = usize::from(inner.width.saturating_sub(6)).max(1);
    let shown_dots = input_chars.min(max_dots);
    let dots = "•".repeat(shown_dots);
    let field_line = Line::from(vec![
        Span::styled("  ", theme.panel()),
        Span::styled(format!(" {dots}"), theme.base()),
        Span::styled("█", theme.base().fg(theme.accent)),
    ]);

    let status_line = if let Some(err) = error {
        Line::from(Span::styled(
            format!("  {err}"),
            Style::new().fg(theme.error),
        ))
    } else if let Some(info) = note {
        Line::from(Span::styled(format!("  {info}"), theme.muted()))
    } else {
        Line::default()
    };

    let [_, prompt_row, field_row, status_row, _, hints_row] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Fill(1),
        Constraint::Length(1),
    ])
    .areas(inner);

    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            format!("  {prompt}"),
            Style::new().fg(theme.text),
        ))),
        prompt_row,
    );
    frame.render_widget(Paragraph::new(field_line), field_row);
    frame.render_widget(Paragraph::new(status_line), status_row);
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(hints, theme.muted()))).alignment(Alignment::Center),
        hints_row,
    );
}
