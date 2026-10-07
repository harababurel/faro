//! The bubbles layout (DESIGN.md §7.3, D45): others' messages on the left,
//! own messages on the right, each in a box with its own background. In
//! groups the sender's name goes above the first bubble of a run; the time,
//! `edited` and the delivery status sit at the bottom right of the bubble, and
//! reactions just under it.

use faro_core::domain::{ConversationId, Direction, Message};
use ratatui::style::Style;
use ratatui::text::{Line, Span};

use super::View;
use super::messages::{content, delivery, format_time, reactions};
use super::text::{sanitize, truncate, width};
use crate::app::state::State;

/// A bubble is at most this share of the pane, in percent…
const MAX_SHARE: usize = 75;
/// …but never narrower than this (unless the pane is).
const MIN_WIDTH: usize = 20;
/// Cells between the text and the time when they share a line.
const META_GAP: usize = 2;

/// Lay out `message` across a pane `pane` cells wide. `first`: it starts a
/// run from its sender (not a continuation).
pub(super) fn render(
    state: &State,
    message: &Message,
    first: bool,
    pane: usize,
    view: &View<'_>,
    out: &mut Vec<Line<'static>>,
) {
    let theme = view.theme;
    let own = matches!(message.direction, Direction::Outgoing(_));
    let max_inner = (pane * MAX_SHARE / 100)
        .max(MIN_WIDTH)
        .min(pane)
        .saturating_sub(2)
        .max(1);

    if first && !own && matches!(message.conversation, ConversationId::Group(_)) {
        let name = sanitize(&state.sender_name(message)).into_owned();
        out.push(Line::from(vec![
            Span::raw(" "),
            Span::styled(
                truncate(&name, pane.saturating_sub(1)).into_owned(),
                theme.sender(message.sender),
            ),
        ]));
    }

    let (lines, inner) = with_meta(
        content(state, message, max_inner, view),
        message,
        view,
        max_inner,
    );
    let bg = Style::new().bg(if own {
        theme.bubble_own
    } else {
        theme.bubble_other
    });
    let margin = if own {
        pane.saturating_sub(inner + 2)
    } else {
        0
    };
    for spans in lines {
        let fill = inner.saturating_sub(spans_width(&spans));
        let mut line = vec![Span::raw(" ".repeat(margin)), Span::styled(" ", bg)];
        line.extend(spans.into_iter().map(|span| span.patch_style(bg)));
        line.push(Span::styled(" ".repeat(fill + 1), bg));
        out.push(Line::from(line));
    }

    if !message.reactions.is_empty() {
        let text = truncate(&reactions(message), pane.saturating_sub(2)).into_owned();
        let indent = if own {
            pane.saturating_sub(width(&text) + 1)
        } else {
            1
        };
        out.push(Line::from(vec![
            Span::raw(" ".repeat(indent)),
            Span::styled(text, theme.muted()),
        ]));
    }
}

/// Add the time (with `edited` and the delivery status) right-aligned on the
/// last line, or on a line of its own if it doesn't fit. Returns the lines
/// and the bubble's inner width.
fn with_meta(
    mut lines: Vec<Vec<Span<'static>>>,
    message: &Message,
    view: &View<'_>,
    max_inner: usize,
) -> (Vec<Vec<Span<'static>>>, usize) {
    let meta = meta(message, view);
    let meta_width = spans_width(&meta);
    let widest = lines.iter().map(|l| spans_width(l)).max().unwrap_or(0);
    let last = lines.last().map_or(0, |l| spans_width(l));
    let shared = last + META_GAP + meta_width;
    if shared <= max_inner
        && let Some(line) = lines.last_mut()
    {
        let inner = widest.max(shared);
        line.push(Span::raw(" ".repeat(inner - last - meta_width)));
        line.extend(meta);
        (lines, inner)
    } else {
        let inner = widest.max(meta_width).min(max_inner);
        let mut line = vec![Span::raw(" ".repeat(inner.saturating_sub(meta_width)))];
        line.extend(meta);
        lines.push(line);
        (lines, inner)
    }
}

/// `edited 19:02 ✓✓`, dimmed.
fn meta(message: &Message, view: &View<'_>) -> Vec<Span<'static>> {
    let muted = view.theme.muted();
    let mut spans = Vec::new();
    if message.edited_at.is_some() && !message.deleted {
        spans.push(Span::styled("edited ", muted));
    }
    spans.push(Span::styled(format_time(message.sent_at, view), muted));
    spans.extend(delivery(message, view));
    spans
}

fn spans_width(spans: &[Span<'_>]) -> usize {
    spans.iter().map(|s| width(&s.content)).sum()
}
