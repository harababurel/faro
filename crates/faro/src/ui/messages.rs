//! The message list, IRC layout (DESIGN.md §7.3): `time  name  text`, names
//! right-aligned in a column and coloured per person, continuation lines
//! indented under the text. Bubbles come in M4.

use std::collections::BTreeMap;

use faro_core::domain::{DeliveryStatus, Direction, Message, Timestamp};
use jiff::civil::Date;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use super::View;
use super::text::{pad_left, sanitize, truncate, width, wrap};
use crate::app::state::{Anchor, OpenConversation, State};

/// Consecutive messages from one sender within this window share one name.
const GROUPING_MS: u64 = 10 * 60 * 1000;

/// Column widths for a pane `width` cells wide.
struct Columns {
    name: usize,
    /// Where message text starts.
    text_start: usize,
    text_width: usize,
}

impl Columns {
    fn for_width(width: usize) -> Self {
        let name = match width {
            70.. => 14,
            45..70 => 10,
            _ => 6,
        };
        // "hh:mm " + name + " "
        let text_start = 6 + name + 1;
        Self {
            name,
            text_start,
            text_width: width.saturating_sub(text_start).max(8),
        }
    }
}

/// What the message area shows.
pub struct Window {
    /// At most `height` lines, oldest first.
    pub lines: Vec<Line<'static>>,
    /// The normalised scroll position (`None`: following the newest message).
    pub scroll: Option<Anchor>,
    /// The first loaded message is on screen.
    pub at_top: bool,
}

/// Everything needed to lay out one message.
struct Context<'a> {
    state: &'a State,
    open: &'a OpenConversation,
    view: &'a View<'a>,
    columns: Columns,
    width: usize,
    today: Date,
}

/// The lines to show for `height` rows, following `open.scroll`.
pub fn window(
    state: &State,
    open: &OpenConversation,
    view: &View<'_>,
    width: usize,
    height: usize,
) -> Window {
    let context = Context {
        state,
        open,
        view,
        columns: Columns::for_width(width),
        width,
        today: date_of(view.now, view),
    };
    let anchored = open.scroll.and_then(|anchor| {
        let at = open
            .messages
            .iter()
            .rposition(|m| m.reference() == anchor.message)?;
        Some((at, anchor.offset))
    });
    match anchored {
        Some((at, offset)) => scrolled(&context, at, offset, height),
        None => newest(&context, height),
    }
}

/// The last `height` lines. Only builds as many messages as fit.
fn newest(context: &Context<'_>, height: usize) -> Window {
    let mut blocks: Vec<Vec<Line<'static>>> = Vec::new();
    let mut total = 0;
    let mut built_all = true;
    for i in (0..context.open.messages.len()).rev() {
        let block = block(context, i);
        total += block.len();
        blocks.push(block);
        if total >= height && i > 0 {
            built_all = false;
            break;
        }
    }
    let mut lines: Vec<Line<'static>> = blocks.into_iter().rev().flatten().collect();
    let excess = lines.len().saturating_sub(height);
    lines.drain(..excess);
    Window {
        lines,
        scroll: None,
        at_top: built_all && excess == 0,
    }
}

/// The view pinned to message `at`, `offset` rows up from its last row. Lays
/// out every loaded message (only while scrolled), then clamps to the loaded
/// history and normalises the anchor to the message on the bottom row.
fn scrolled(context: &Context<'_>, at: usize, offset: isize, height: usize) -> Window {
    let messages = &context.open.messages;
    let blocks: Vec<Vec<Line<'static>>> = (0..messages.len()).map(|i| block(context, i)).collect();
    let ends: Vec<usize> = blocks
        .iter()
        .scan(0, |end, b| {
            *end += b.len();
            Some(*end)
        })
        .collect();
    let total = ends.last().copied().unwrap_or(0);
    let Some(last_row) = total.checked_sub(1) else {
        return Window {
            lines: Vec::new(),
            scroll: None,
            at_top: true,
        };
    };
    let lowest_top_fill = height.min(total).saturating_sub(1);
    let wanted = ends
        .get(at)
        .map_or(last_row, |end| end.saturating_sub(1))
        .cast_signed()
        .saturating_sub(offset);
    let bottom = wanted.clamp(lowest_top_fill.cast_signed(), last_row.cast_signed());
    let bottom = bottom.cast_unsigned();
    let scroll = (bottom < last_row)
        .then(|| {
            let j = ends.partition_point(|end| *end <= bottom);
            let end = ends.get(j)?;
            Some(Anchor {
                message: messages.get(j)?.reference(),
                offset: (end - 1 - bottom).cast_signed(),
            })
        })
        .flatten();
    let top = (bottom + 1).saturating_sub(height);
    let lines = blocks
        .into_iter()
        .flatten()
        .skip(top)
        .take(bottom + 1 - top)
        .collect();
    Window {
        lines,
        scroll,
        at_top: top == 0,
    }
}

/// Message `i` with the date rule and unread rule that go above it.
fn block(context: &Context<'_>, i: usize) -> Vec<Line<'static>> {
    let Context {
        state,
        open,
        view,
        columns,
        width,
        today,
    } = context;
    let mut block = Vec::new();
    let Some(message) = open.messages.get(i) else {
        return block;
    };
    let previous = i.checked_sub(1).and_then(|p| open.messages.get(p));
    let date = date_of_message(message, view);
    let new_day = previous.is_none_or(|p| date_of_message(p, view) != date);
    if new_day {
        block.push(rule(&day_label(date, *today), *width, view.theme.muted()));
    }
    if open.unread_marker == Some(message.reference()) {
        let style = Style::new().fg(view.theme.unread);
        block.push(rule("new messages", *width, style));
    }
    let continued = !new_day
        && previous.is_some_and(|p| {
            p.sender == message.sender
                && message
                    .sent_at
                    .as_millis()
                    .saturating_sub(p.sent_at.as_millis())
                    < GROUPING_MS
        });
    render_message(state, message, !continued, columns, view, &mut block);
    block
}

fn render_message(
    state: &State,
    message: &Message,
    show_name: bool,
    columns: &Columns,
    view: &View<'_>,
    out: &mut Vec<Line<'static>>,
) {
    let theme = view.theme;
    let indent = " ".repeat(columns.text_start);
    let time = format_time(message.sent_at, view);
    let name = if show_name {
        let name = sanitize(&state.sender_name(message)).into_owned();
        pad_left(&truncate(&name, columns.name), columns.name)
    } else {
        " ".repeat(columns.name)
    };
    let name_style = if message.direction == Direction::Incoming {
        theme.sender(message.sender)
    } else {
        Style::new().fg(theme.primary).add_modifier(Modifier::BOLD)
    };
    let mut prefix = vec![
        Span::styled(format!("{time} "), theme.muted()),
        Span::styled(name, name_style),
        Span::raw(" "),
    ];
    let mut body: Vec<Vec<Span<'static>>> = Vec::new();

    if let Some(quote) = &message.quote {
        let who = state.person_name(quote.target.sender);
        let preview = quote.preview.as_ref().map_or_else(
            || "…".to_owned(),
            |p| sanitize(p.expose()).replace('\n', " "),
        );
        let text = format!("▎ {who}: {preview}");
        body.push(vec![Span::styled(
            truncate(&text, columns.text_width).into_owned(),
            theme.muted().add_modifier(Modifier::ITALIC),
        )]);
    }

    if message.deleted {
        body.push(vec![Span::styled(
            "This message was deleted",
            theme.muted().add_modifier(Modifier::ITALIC),
        )]);
    } else if let Some(text) = &message.body {
        for line in wrap(text.expose(), columns.text_width) {
            body.push(vec![Span::styled(line, Style::new().fg(theme.text))]);
        }
    }
    for attachment in &message.attachments {
        let name = attachment.file_name.as_deref().map_or_else(
            || attachment.content_type.clone(),
            |n| sanitize(n).into_owned(),
        );
        let size = attachment.size.map(format_size).unwrap_or_default();
        let text = format!("📎 {name}{size}");
        body.push(vec![Span::styled(
            truncate(&text, columns.text_width).into_owned(),
            Style::new().fg(theme.secondary),
        )]);
    }
    if body.is_empty() {
        body.push(vec![Span::styled("(empty message)", theme.muted())]);
    }

    append_suffix(&mut body, suffix(message, view), columns.text_width);

    if !message.reactions.is_empty() {
        body.push(vec![Span::styled(reactions(message), theme.muted())]);
    }

    for (i, spans) in body.into_iter().enumerate() {
        let mut line = if i == 0 {
            std::mem::take(&mut prefix)
        } else {
            vec![Span::raw(indent.clone())]
        };
        line.extend(spans);
        out.push(Line::from(line));
    }
}

/// `(edited)` and the delivery status of own messages.
fn suffix(message: &Message, view: &View<'_>) -> Vec<Span<'static>> {
    let theme = view.theme;
    let mut spans = Vec::new();
    if message.edited_at.is_some() && !message.deleted {
        spans.push(Span::styled(" (edited)", theme.muted()));
    }
    if let Direction::Outgoing(status) = message.direction {
        let (glyph, style) = match status {
            DeliveryStatus::Sending => (" ◌", theme.muted()),
            DeliveryStatus::Sent => (" ✓", theme.muted()),
            DeliveryStatus::Delivered => (" ✓✓", theme.muted()),
            DeliveryStatus::Read => (" ✓✓", Style::new().fg(theme.secondary)),
            DeliveryStatus::Failed => (" ! not sent", Style::new().fg(theme.error)),
        };
        spans.push(Span::styled(glyph, style));
    }
    spans
}

/// Put `suffix` at the end of the last line, or on its own line if it won't fit.
fn append_suffix(body: &mut Vec<Vec<Span<'static>>>, suffix: Vec<Span<'static>>, max: usize) {
    if suffix.is_empty() {
        return;
    }
    let suffix_width: usize = suffix.iter().map(|s| width(&s.content)).sum();
    match body.last_mut() {
        Some(last)
            if last.iter().map(|s| width(&s.content)).sum::<usize>() + suffix_width <= max =>
        {
            last.extend(suffix);
        }
        Some(_) | None => body.push(suffix),
    }
}

/// `👍 2  ❤️ 1`, in order of first appearance.
fn reactions(message: &Message) -> String {
    let mut order: Vec<&str> = Vec::new();
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for reaction in &message.reactions {
        let emoji = reaction.emoji.as_str();
        let count = counts.entry(emoji).or_insert(0);
        if *count == 0 {
            order.push(emoji);
        }
        *count += 1;
    }
    order
        .iter()
        .map(|emoji| {
            let n = counts.get(emoji).copied().unwrap_or(0);
            let emoji = sanitize(emoji);
            if n > 1 {
                format!("{emoji} {n}")
            } else {
                emoji.into_owned()
            }
        })
        .collect::<Vec<_>>()
        .join("  ")
}

/// `── label ──` centred across `width`.
fn rule(label: &str, width: usize, style: Style) -> Line<'static> {
    let label = format!(" {label} ");
    let side = width.saturating_sub(super::text::width(&label));
    let left = side / 2;
    Line::from(Span::styled(
        format!("{}{label}{}", "─".repeat(left), "─".repeat(side - left)),
        style,
    ))
}

fn zoned(ts: jiff::Timestamp, view: &View<'_>) -> jiff::Zoned {
    ts.to_zoned(view.tz.clone())
}

fn to_jiff(ts: Timestamp) -> Option<jiff::Timestamp> {
    let ms = i64::try_from(ts.as_millis()).ok()?;
    jiff::Timestamp::from_millisecond(ms).ok()
}

fn date_of(ts: jiff::Timestamp, view: &View<'_>) -> Date {
    zoned(ts, view).date()
}

fn date_of_message(message: &Message, view: &View<'_>) -> Option<Date> {
    to_jiff(message.sent_at).map(|ts| date_of(ts, view))
}

fn format_time(ts: Timestamp, view: &View<'_>) -> String {
    to_jiff(ts).map_or_else(
        || "--:--".to_owned(),
        |ts| zoned(ts, view).strftime("%H:%M").to_string(),
    )
}

fn day_label(date: Option<Date>, today: Date) -> String {
    let Some(date) = date else {
        return "Unknown date".to_owned();
    };
    if date == today {
        "Today".to_owned()
    } else if today.yesterday().is_ok_and(|y| y == date) {
        "Yesterday".to_owned()
    } else if date.year() == today.year() {
        date.strftime("%A, %-d %B").to_string()
    } else {
        date.strftime("%A, %-d %B %Y").to_string()
    }
}

#[expect(
    clippy::cast_precision_loss,
    reason = "sizes are shown to one decimal; precision beyond 2^52 bytes doesn't matter"
)]
fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["KB", "MB", "GB", "TB"];
    if bytes < 1024 {
        return format!(" · {bytes} B");
    }
    let mut size = bytes as f64 / 1024.0;
    let mut unit = 0;
    while size >= 1024.0 && unit + 1 < UNITS.len() {
        size /= 1024.0;
        unit += 1;
    }
    format!(" · {size:.1} {}", UNITS.get(unit).copied().unwrap_or("TB"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_read_well() {
        assert_eq!(format_size(512), " · 512 B");
        assert_eq!(format_size(12_345), " · 12.1 KB");
        assert_eq!(format_size(5 * 1024 * 1024), " · 5.0 MB");
    }

    #[test]
    fn rules_fill_the_width() {
        let line = rule("Today", 21, Style::new());
        assert_eq!(line.to_string(), "─────── Today ───────");
    }
}
