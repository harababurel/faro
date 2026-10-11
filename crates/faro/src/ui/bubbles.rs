//! The bubbles layout (DESIGN.md §7.3, D45): others' messages on the left,
//! own messages on the right, each in a box with its own background. In
//! groups the sender's name goes above the first bubble of a run; the time,
//! `edited` and the delivery status sit at the bottom right of the bubble, and
//! reactions just under it.
//!
//! Styles (`[messages] bubble_style`, D58, D66, D98):
//! - **rounded**: a one-line bubble gets Powerline half-circle caps (U+E0B6,
//!   U+E0B4) in place of its side cells, a capsule. A taller one is a rounded
//!   rectangle drawn on a grid of octants (`bubbles/grid.rs`): half-cell sides,
//!   quarter-row edges above and below, quarter circles at the corners.
//!   `[messages] bubble_padding` adds a blank cell inside each rounded side.
//! - **bar**: a heavy bar `┃` on the bubble's outer side (left for others, in
//!   the sender's colour; right for own messages, in the theme's primary) in
//!   place of that side's padding cell, like OpenCode's message boxes.
//! - **mixed**: the bar outside, rounded on the inner side.
//!
//! In compact density, rounded bubbles from one sender join (D98): they touch,
//! share one edge on their aligned side, and a wider bubble's corner is drawn
//! in its neighbour's row. The caller passes each bubble its neighbours'
//! [`Outline`]s in a [`Run`]; one-line bubbles in a run are rectangles too.
//!
//! `auto` is rounded only where the terminal draws those glyphs itself
//! (kitty); fonts often lack them. The `system` theme (no bubble fill)
//! always draws bars.

mod grid;

use faro_core::config::BubbleStyle;
use faro_core::domain::{ConversationId, Direction, Message};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};

use self::grid::{Cell, Grid, Rect};
use super::View;
use super::messages::{ImageSlot, content, delivery, message_time, reactions};
use super::text::{sanitize, truncate, width};
use crate::app::state::{BlockGlyphs, State};
use crate::app::update::selection::find_links;
use crate::theme::Theme;

pub(super) use self::grid::is_edge;

/// A bubble is at most this share of the pane, in percent…
const MAX_SHARE: usize = 75;
/// …but never narrower than this (unless the pane is).
const MIN_WIDTH: usize = 20;
/// Cells between the text and the time when they share a line.
const META_GAP: usize = 2;
/// A one-line rounded bubble's ends: left and right half circles (Powerline).
const CAPS: [&str; 2] = ["\u{E0B6}", "\u{E0B4}"];
/// The bar on a bubble's outer side (OpenCode's message border).
pub(super) const BAR: &str = "\u{2503}";

/// How bubbles are drawn, once `auto` and the theme are decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Shape {
    Rounded,
    Bar,
    /// The bar on the outer side, rounded on the inner one (D66).
    Mixed,
}

impl Shape {
    /// Whether some side is rounded (so bubbles have edges and runs join).
    pub(super) const fn rounds(self) -> bool {
        matches!(self, Self::Rounded | Self::Mixed)
    }
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
        BubbleStyle::Mixed => Shape::Mixed,
        BubbleStyle::Auto if state.env.glyphs == BlockGlyphs::Native => Shape::Rounded,
        BubbleStyle::Auto | BubbleStyle::Bar => Shape::Bar,
    }
}

/// Where a bubble sits across the pane, as its neighbours in a compact run
/// see it (D98): cells `x0..x1`, side cells included, and which sides are
/// rounded (a bar side is square and fills its whole cell).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Outline {
    x0: usize,
    x1: usize,
    round_left: bool,
    round_right: bool,
}

impl Outline {
    /// Its octants for text rows `top..bottom` of a grid (rows may be outside
    /// it): half a cell in from rounded side cells, a quarter row above and
    /// below the text.
    fn rect(self, top: isize, bottom: isize) -> Rect {
        Rect {
            x: 2 * self.x0 + usize::from(self.round_left)
                ..(2 * self.x1).saturating_sub(usize::from(self.round_right)),
            y: 4 * top - 1..4 * bottom + 1,
            round_left: self.round_left,
            round_right: self.round_right,
        }
    }
}

/// A bubble's place among its sender's messages: whether it starts their
/// run (the name goes above it in groups) and, in a joined compact run
/// (D98), the neighbours it joins and the width to draw its text area at
/// (at least its own; wider when `match_widths` lines it up with them).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) struct Run {
    pub first: bool,
    pub above: Option<Outline>,
    pub below: Option<Outline>,
    pub inner: usize,
}

/// What depends only on the pane, the settings and the side.
struct Frame {
    shape: Shape,
    /// Blank cells inside the left and right rounded sides.
    pads: (usize, usize),
    /// The widest the text area may be.
    max_inner: usize,
}

impl Frame {
    fn new(state: &State, theme: &Theme, pane: usize, own: bool) -> Self {
        let shape = shape(state, theme);
        let pad = usize::from(state.config.messages.bubble_padding);
        let pads = match shape {
            Shape::Rounded => (pad, pad),
            Shape::Bar => (0, 0),
            Shape::Mixed if own => (pad, 0),
            Shape::Mixed => (0, pad),
        };
        let max_inner = (pane * MAX_SHARE / 100)
            .max(MIN_WIDTH)
            .min(pane)
            .saturating_sub(2 + pads.0 + pads.1)
            .max(1);
        Self {
            shape,
            pads,
            max_inner,
        }
    }

    /// The widest `message`'s text area may be: `max_inner`, or wider, up to
    /// the whole pane, so that a link fits on one line and stays clickable
    /// (D102). Text wraps at spaces, so it's the widest word holding a link.
    fn max_inner_for(&self, message: &Message, pane: usize) -> usize {
        let full = pane.saturating_sub(2 + self.pads.0 + self.pads.1).max(1);
        let longest = message.body.as_ref().map_or(0, |body| {
            body.expose()
                .split_whitespace()
                .filter(|word| !find_links(word).is_empty())
                .map(|word| width(&sanitize(word)))
                .max()
                .unwrap_or(0)
        });
        if longest > self.max_inner {
            longest.min(full)
        } else {
            self.max_inner
        }
    }

    /// The outline of a bubble whose text area is `inner` wide.
    fn outline(&self, inner: usize, pane: usize, own: bool) -> Outline {
        let width = inner + 2 + self.pads.0 + self.pads.1;
        let x0 = if own { pane.saturating_sub(width) } else { 0 };
        let (round_left, round_right) = match self.shape {
            Shape::Rounded => (true, true),
            Shape::Bar => (false, false),
            Shape::Mixed => (own, !own),
        };
        Outline {
            x0,
            x1: x0 + width,
            round_left,
            round_right,
        }
    }
}

/// The width of `message`'s text area, before any lining up (D98).
pub(super) fn measure(state: &State, message: &Message, pane: usize, view: &View<'_>) -> usize {
    let own = matches!(message.direction, Direction::Outgoing(_));
    let frame = Frame::new(state, view.theme, pane, own);
    let max_inner = frame.max_inner_for(message, pane);
    let (lines, _) = content(state, message, max_inner, view);
    with_meta(lines, state, message, view, max_inner, 0).1
}

/// Where `message` sits when its text area is `inner` wide (D98).
pub(super) fn outline(
    state: &State,
    message: &Message,
    inner: usize,
    pane: usize,
    view: &View<'_>,
) -> Outline {
    let own = matches!(message.direction, Direction::Outgoing(_));
    Frame::new(state, view.theme, pane, own).outline(inner, pane, own)
}

/// Lay out `message` across a pane `pane` cells wide, placed in its
/// sender's run as `run` says.
pub(super) fn render(
    state: &State,
    message: &Message,
    pane: usize,
    view: &View<'_>,
    run: Run,
    out: &mut Vec<Line<'static>>,
    images: &mut Vec<ImageSlot>,
) {
    let theme = view.theme;
    let own = matches!(message.direction, Direction::Outgoing(_));
    let frame = Frame::new(state, theme, pane, own);

    if run.first && !own && matches!(message.conversation, ConversationId::Group(_)) {
        let name = sanitize(&state.sender_name(message)).into_owned();
        out.push(Line::from(vec![
            Span::raw(" "),
            Span::styled(
                truncate(&name, pane.saturating_sub(1)).into_owned(),
                theme.sender(message.sender),
            ),
        ]));
    }

    let max_inner = frame.max_inner_for(message, pane);
    let (content_lines, pending_images) = content(state, message, max_inner, view);
    let (lines, inner) = with_meta(content_lines, state, message, view, max_inner, run.inner);
    let colour = if own {
        theme.bubble_own
    } else {
        theme.bubble_other
    };
    let bar = if own {
        Span::styled(BAR, Style::new().bg(colour).fg(theme.primary))
    } else {
        let sender = theme.sender(message.sender).fg.unwrap_or(theme.text);
        Span::styled(BAR, Style::new().bg(colour).fg(sender))
    };
    let joined = run.above.is_some() || run.below.is_some();
    let pill = frame.shape.rounds() && lines.len() == 1 && !joined;
    let bubble = Bubble {
        lines,
        inner,
        own,
        colour,
        bar,
    };
    let (body_start, text_x) = if frame.shape == Shape::Bar || pill {
        bubble.flat(&frame, pane, out)
    } else {
        bubble.rounded(&frame, pane, run, out)
    };
    let x = u16::try_from(text_x).unwrap_or(0);
    for pending in pending_images {
        if let Ok(y) = i16::try_from(body_start + pending.row) {
            images.push(ImageSlot {
                x,
                y,
                width: pending.width,
                height: pending.height,
                path: pending.path,
            });
        }
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

/// A laid-out bubble about to be drawn.
struct Bubble {
    lines: Vec<Vec<Span<'static>>>,
    inner: usize,
    own: bool,
    colour: Color,
    bar: Span<'static>,
}

impl Bubble {
    /// Bars, and one-line capsules: one row per line, a side cell each end.
    /// Returns the first text row's index in `out` and the text's column.
    fn flat(self, frame: &Frame, pane: usize, out: &mut Vec<Line<'static>>) -> (usize, usize) {
        let bg = Style::new().bg(self.colour);
        let margin = if self.own {
            pane.saturating_sub(self.inner + 2)
        } else {
            0
        };
        let padding = Span::styled(" ", bg);
        let cap = |glyph: &'static str| Span::styled(glyph, Style::new().fg(self.colour));
        let (left, right) = match frame.shape {
            Shape::Rounded => (cap(CAPS[0]), cap(CAPS[1])),
            Shape::Bar if self.own => (padding, self.bar),
            Shape::Bar => (self.bar, padding),
            Shape::Mixed if self.own => (cap(CAPS[0]), self.bar),
            Shape::Mixed => (self.bar, cap(CAPS[1])),
        };
        let body_start = out.len();
        for spans in self.lines {
            let fill = self.inner.saturating_sub(spans_width(&spans));
            let mut line = vec![Span::raw(" ".repeat(margin)), left.clone()];
            line.extend(spans.into_iter().map(|span| span.patch_style(bg)));
            line.push(Span::styled(" ".repeat(fill), bg));
            line.push(right.clone());
            out.push(Line::from(line));
        }
        (body_start, margin + 1)
    }

    /// A rounded rectangle on the octant grid (D98), with an edge row above
    /// and below unless it joins a neighbour there; the neighbours'
    /// rectangles are added so their edges and corners land in its rows.
    fn rounded(
        self,
        frame: &Frame,
        pane: usize,
        run: Run,
        out: &mut Vec<Line<'static>>,
    ) -> (usize, usize) {
        let outline = frame.outline(self.inner, pane, self.own);
        let top = usize::from(run.above.is_none());
        let text_rows = self.lines.len();
        let rows = top + text_rows + usize::from(run.below.is_none());
        let cols = [Some(outline), run.above, run.below]
            .into_iter()
            .flatten()
            .map(|o| o.x1)
            .max()
            .unwrap_or(outline.x1);
        let mut grid = Grid::new(cols, rows);
        let first = top.cast_signed();
        let end = (top + text_rows).cast_signed();
        grid.add(&outline.rect(first, end));
        if let Some(above) = run.above {
            grid.add(&above.rect(-1, 0));
        }
        if let Some(below) = run.below {
            let rows = rows.cast_signed();
            grid.add(&below.rect(rows, rows + 1));
        }
        let bg = Style::new().bg(self.colour);
        let edge = Style::new().fg(self.colour);
        let draw = |row: &mut RowBuilder, col: usize, y: usize| match grid.cell(col, y) {
            Cell::Empty => row.push(" ", Style::new()),
            Cell::Full => row.push(" ", bg),
            Cell::Glyph(c) => row.push(c.encode_utf8(&mut [0; 4]), edge),
        };
        let body_start = out.len() + top;
        let mut lines = self.lines.into_iter();
        for y in 0..rows {
            let mut row = RowBuilder::default();
            let text = (top..top + text_rows)
                .contains(&y)
                .then(|| lines.next())
                .flatten();
            let Some(spans) = text else {
                (0..cols).for_each(|col| draw(&mut row, col, y));
                out.push(row.finish());
                continue;
            };
            (0..outline.x0).for_each(|col| draw(&mut row, col, y));
            if outline.round_left {
                draw(&mut row, outline.x0, y);
            } else {
                row.span(self.bar.clone());
            }
            row.push(&" ".repeat(frame.pads.0), bg);
            let fill = self.inner.saturating_sub(spans_width(&spans));
            for span in spans {
                row.span(span.patch_style(bg));
            }
            row.push(&" ".repeat(fill + frame.pads.1), bg);
            if outline.round_right {
                draw(&mut row, outline.x1 - 1, y);
            } else {
                row.span(self.bar.clone());
            }
            (outline.x1..cols).for_each(|col| draw(&mut row, col, y));
            out.push(row.finish());
        }
        (body_start, outline.x0 + 1 + frame.pads.0)
    }
}

/// A line built cell by cell, runs of one style merged into one span.
#[derive(Default)]
struct RowBuilder {
    spans: Vec<Span<'static>>,
    text: String,
    style: Style,
}

impl RowBuilder {
    fn push(&mut self, text: &str, style: Style) {
        if style != self.style {
            self.flush();
            self.style = style;
        }
        self.text.push_str(text);
    }

    fn span(&mut self, span: Span<'static>) {
        self.flush();
        self.spans.push(span);
    }

    fn flush(&mut self) {
        if !self.text.is_empty() {
            let text = std::mem::take(&mut self.text);
            self.spans.push(Span::styled(text, self.style));
        }
    }

    /// The line, without trailing blank cells on the pane's background.
    fn finish(mut self) -> Line<'static> {
        if self.style == Style::new() {
            let kept = self.text.trim_end().len();
            self.text.truncate(kept);
        }
        self.flush();
        Line::from(self.spans)
    }
}

/// Add the time (with `edited` and the delivery status) right-aligned on the
/// last line, or on a line of its own if it doesn't fit. Returns the lines
/// and the bubble's inner width: at least `at_least` (a run lining up, D98),
/// which is never more than `max_inner`.
fn with_meta(
    mut lines: Vec<Vec<Span<'static>>>,
    state: &State,
    message: &Message,
    view: &View<'_>,
    max_inner: usize,
    at_least: usize,
) -> (Vec<Vec<Span<'static>>>, usize) {
    let at_least = at_least.min(max_inner);
    let meta = meta(state, message, view);
    let widest = lines.iter().map(|l| spans_width(l)).max().unwrap_or(0);
    if meta.is_empty() {
        return (lines, widest.min(max_inner).max(at_least));
    }
    let meta_width = spans_width(&meta);
    let last = lines.last().map_or(0, |l| spans_width(l));
    let shared = last + META_GAP + meta_width;
    if shared <= max_inner
        && let Some(line) = lines.last_mut()
    {
        let inner = widest.max(shared).max(at_least);
        line.push(Span::raw(" ".repeat(inner - last - meta_width)));
        line.extend(meta);
        (lines, inner)
    } else {
        let inner = widest.max(meta_width).min(max_inner).max(at_least);
        let mut line = vec![Span::raw(" ".repeat(inner.saturating_sub(meta_width)))];
        line.extend(meta);
        lines.push(line);
        (lines, inner)
    }
}

/// `edited 19:02 ✓✓`, dimmed.
fn meta(state: &State, message: &Message, view: &View<'_>) -> Vec<Span<'static>> {
    let muted = view.theme.muted();
    let time = message_time(state, message, view);
    let mut spans = Vec::new();
    if message.edited_at.is_some() && !message.deleted {
        let label = if time.is_some() { "edited " } else { "edited" };
        spans.push(Span::styled(label, muted));
    }
    if let Some(time) = time {
        spans.push(Span::styled(time, muted));
    }
    if let Some(status) = delivery(message, view) {
        if spans.is_empty() {
            spans.push(Span::styled(
                status.content.trim_start().to_owned(),
                status.style,
            ));
        } else {
            spans.push(status);
        }
    }
    spans
}

fn spans_width(spans: &[Span<'_>]) -> usize {
    spans.iter().map(|s| width(&s.content)).sum()
}
