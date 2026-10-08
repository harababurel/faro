//! The bubbles layout (DESIGN.md §7.3, D45): others' messages on the left,
//! own messages on the right, each in a box with its own background. In
//! groups the sender's name goes above the first bubble of a run; the time,
//! `edited` and the delivery status sit at the bottom right of the bubble, and
//! reactions just under it.
//!
//! Two styles (`[messages] bubble_style`, D58), both as wide as the bubble's
//! text plus a cell on each side:
//! - **rounded**: a one-line bubble gets Powerline half-circle caps (U+E0B6,
//!   U+E0B4) in place of its padding cells; a taller one gets an edge row
//!   above and below, two thirds of a row high with diagonal corners (Legacy
//!   Computing wedges and sextants: `🭊🬹🬹🬿` … `🭥🬎🬎🭚`). Caps and edges are
//!   drawn in the bubble's colour on the pane's background.
//! - **bar**: a heavy bar `┃` on the bubble's outer side (left for others, in
//!   the sender's colour; right for own messages, in the theme's primary) in
//!   place of that side's padding cell, like OpenCode's message boxes.
//!
//! `auto` is rounded only where the terminal draws those glyphs itself
//! (kitty); fonts often lack them. The `system` theme (no bubble fill)
//! always draws bars.

use faro_core::config::BubbleStyle;
use faro_core::domain::{ConversationId, Direction, Message};
use ratatui::style::Style;
use ratatui::text::{Line, Span};

use super::View;
use super::messages::{content, delivery, format_time, reactions};
use super::text::{sanitize, truncate, width};
use crate::app::state::{BlockGlyphs, State};
use crate::theme::Theme;

/// A bubble is at most this share of the pane, in percent…
const MAX_SHARE: usize = 75;
/// …but never narrower than this (unless the pane is).
const MIN_WIDTH: usize = 20;
/// Cells between the text and the time when they share a line.
const META_GAP: usize = 2;
/// A one-line rounded bubble's ends: left and right half circles (Powerline).
const CAPS: [&str; 2] = ["\u{E0B6}", "\u{E0B4}"];
/// A taller rounded bubble's edges: corner, fill and corner, two thirds of a
/// row high (Legacy Computing: a wedge, `BLOCK SEXTANT-3456` or `-1234`, a
/// wedge).
const TOP: [&str; 3] = ["\u{1FB4A}", "\u{1FB39}", "\u{1FB3F}"];
const BOTTOM: [&str; 3] = ["\u{1FB65}", "\u{1FB0E}", "\u{1FB5A}"];
/// Every glyph drawn on the pane's background around a rounded bubble (the
/// selection tint blanks them).
pub(super) const EDGE_GLYPHS: [&str; 8] = [
    CAPS[0], CAPS[1], TOP[0], TOP[1], TOP[2], BOTTOM[0], BOTTOM[1], BOTTOM[2],
];
/// The bar on a bubble's outer side (OpenCode's message border).
pub(super) const BAR: &str = "\u{2503}";

/// How bubbles are drawn, once `auto` and the theme are decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Shape {
    Rounded,
    Bar,
}

/// The style in use: the setting (`auto`: rounded where the terminal draws
/// the glyphs itself), and always bars for a theme that leaves bubbles
/// unfilled (`system`).
pub(super) fn shape(state: &State, theme: &Theme) -> Shape {
    if theme.terminal_palette {
        return Shape::Bar;
    }
    match state.bubble_style() {
        BubbleStyle::Rounded => Shape::Rounded,
        BubbleStyle::Auto if state.env.glyphs == BlockGlyphs::Native => Shape::Rounded,
        BubbleStyle::Auto | BubbleStyle::Bar => Shape::Bar,
    }
}

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
    let colour = if own {
        theme.bubble_own
    } else {
        theme.bubble_other
    };
    let bg = Style::new().bg(colour);
    let margin = if own {
        pane.saturating_sub(inner + 2)
    } else {
        0
    };
    let shape = shape(state, theme);
    let pill = shape == Shape::Rounded && lines.len() == 1;
    let edges = shape == Shape::Rounded && !pill;
    let padding = Span::styled(" ", bg);
    let (left, right) = match shape {
        Shape::Rounded if pill => {
            let cap = Style::new().fg(colour);
            (Span::styled(CAPS[0], cap), Span::styled(CAPS[1], cap))
        }
        Shape::Rounded => (padding.clone(), padding),
        Shape::Bar if own => (padding, Span::styled(BAR, bg.fg(theme.primary))),
        Shape::Bar => {
            let sender = theme.sender(message.sender).fg.unwrap_or(theme.text);
            (Span::styled(BAR, bg.fg(sender)), padding)
        }
    };
    if edges {
        out.push(edge(margin, inner, TOP, colour));
    }
    for spans in lines {
        let fill = inner.saturating_sub(spans_width(&spans));
        let mut line = vec![Span::raw(" ".repeat(margin)), left.clone()];
        line.extend(spans.into_iter().map(|span| span.patch_style(bg)));
        line.push(Span::styled(" ".repeat(fill), bg));
        line.push(right.clone());
        out.push(Line::from(line));
    }
    if edges {
        out.push(edge(margin, inner, BOTTOM, colour));
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

/// A tall rounded bubble's top or bottom edge: `glyphs` are the left corner, the
/// fill and the right corner, in the bubble's colour on the pane's.
fn edge(
    margin: usize,
    inner: usize,
    glyphs: [&str; 3],
    colour: ratatui::style::Color,
) -> Line<'static> {
    let [left, fill, right] = glyphs;
    Line::from(vec![
        Span::raw(" ".repeat(margin)),
        Span::styled(
            format!("{left}{}{right}", fill.repeat(inner)),
            Style::new().fg(colour),
        ),
    ])
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
