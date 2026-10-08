//! The message list (DESIGN.md §7.3): windowing, date and unread rules, and
//! the IRC layout: `time  name  text`, names right-aligned in a column and
//! coloured per person, continuation lines indented under the text. The
//! bubbles layout is in `bubbles.rs` (D45).

use std::collections::BTreeMap;

use faro_core::domain::{
    AttachmentStatus, BodyPart, DeliveryStatus, Direction, Message, MessageRef, Timestamp,
    mention_parts,
};
use jiff::civil::Date;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use super::View;
use super::bubbles;
use super::text::{one_line, pad_left, sanitize, truncate, width, wrap_pieces};
use crate::app::state::{Anchor, MessageLayout, OpenConversation, State};

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

/// Where an inline image box sits inside the message list (D78).
#[derive(Debug, Clone)]
pub struct ImageSlot {
    /// Column offset from the left edge of the history area.
    pub x: u16,
    /// Row offset from the top of `Window::lines` (negative when the top of
    /// the box is scrolled above the visible window).
    pub y: i16,
    /// Width of the box in cells.
    pub width: u16,
    /// Height of the box in rows.
    pub height: u16,
    /// Local file path once downloaded, or `None` while downloading.
    pub path: Option<std::path::PathBuf>,
}

/// An inline image box reserved inside `content` at line index `row`.
pub(super) struct PendingImage {
    pub row: usize,
    pub width: u16,
    pub height: u16,
    pub path: Option<std::path::PathBuf>,
}

/// What the message area shows.
pub struct Window {
    /// At most `height` lines, oldest first.
    pub lines: Vec<Line<'static>>,
    /// The normalised scroll position (`None`: following the newest message).
    pub scroll: Option<Anchor>,
    /// The first loaded message is on screen.
    pub at_top: bool,
    /// The message each line belongs to (`None` for date and unread rules).
    pub rows: Vec<Option<MessageRef>>,
    /// Inline image boxes that overlap this window (D78).
    pub images: Vec<ImageSlot>,
}

/// One message's lines, with the rules above it.
struct Block {
    lines: Vec<Line<'static>>,
    /// How many of the first lines are rules (date, unread).
    rules: usize,
    message: Option<MessageRef>,
    images: Vec<ImageSlot>,
}

impl Block {
    fn rows(&self) -> impl Iterator<Item = Option<MessageRef>> + '_ {
        (0..self.lines.len()).map(|i| self.message.filter(|_| i >= self.rules))
    }
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

/// The lines to show for `height` rows, following `open.scroll`, and moved
/// if needed so a selected message waiting to be revealed is on screen.
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
    let position = |target: MessageRef| open.messages.iter().rposition(|m| m.reference() == target);
    let anchored = open
        .scroll
        .and_then(|anchor| Some((position(anchor.message)?, anchor.offset)));
    let reveal = open
        .selected
        .filter(|s| s.reveal)
        .and_then(|s| position(s.message));
    if anchored.is_none() && reveal.is_none() {
        newest(&context, height)
    } else {
        scrolled(&context, anchored, reveal, height)
    }
}

/// The last `height` lines. Only builds as many messages as fit.
fn newest(context: &Context<'_>, height: usize) -> Window {
    let mut blocks: Vec<Block> = Vec::new();
    let mut total = 0;
    let mut built_all = true;
    for i in (0..context.open.messages.len()).rev() {
        let block = block(context, i);
        total += block.lines.len();
        blocks.push(block);
        if total >= height && i > 0 {
            built_all = false;
            break;
        }
    }
    blocks.reverse();
    let mut rows: Vec<Option<MessageRef>> = blocks.iter().flat_map(Block::rows).collect();
    let mut lines: Vec<Line<'static>> = Vec::with_capacity(total);
    let mut raw_images: Vec<(usize, ImageSlot)> = Vec::new();
    for b in blocks {
        let start = lines.len();
        for slot in b.images {
            raw_images.push((start, slot));
        }
        lines.extend(b.lines);
    }
    let excess = lines.len().saturating_sub(height);
    lines.drain(..excess);
    rows.drain(..excess);
    let images = visible_slots(raw_images, excess, height);
    Window {
        lines,
        scroll: None,
        at_top: built_all && excess == 0,
        rows,
        images,
    }
}

/// The view pinned to message `at`, `offset` rows up from its last row (or
/// following the newest message if `anchored` is `None`), then moved just
/// enough to show message `reveal`. Lays out every loaded message (only while
/// scrolled or revealing), clamps to the loaded history and normalises the
/// anchor to the message on the bottom row.
fn scrolled(
    context: &Context<'_>,
    anchored: Option<(usize, isize)>,
    reveal: Option<usize>,
    height: usize,
) -> Window {
    let messages = &context.open.messages;
    let blocks: Vec<Block> = (0..messages.len()).map(|i| block(context, i)).collect();
    let ends: Vec<usize> = blocks
        .iter()
        .scan(0, |end, b| {
            *end += b.lines.len();
            Some(*end)
        })
        .collect();
    let total = ends.last().copied().unwrap_or(0);
    let Some(last_row) = total.checked_sub(1) else {
        return Window {
            lines: Vec::new(),
            scroll: None,
            at_top: true,
            rows: Vec::new(),
            images: Vec::new(),
        };
    };
    let lowest_top_fill = height.min(total).saturating_sub(1);
    let clamp = |row: isize| {
        row.clamp(lowest_top_fill.cast_signed(), last_row.cast_signed())
            .cast_unsigned()
    };
    let wanted = anchored.map_or(last_row.cast_signed(), |(at, offset)| {
        ends.get(at)
            .map_or(last_row, |end| end.saturating_sub(1))
            .cast_signed()
            .saturating_sub(offset)
    });
    let mut bottom = clamp(wanted);
    if let Some(r) = reveal
        && let Some(end) = ends.get(r)
    {
        // Rows of the message, from its first rule to its last line.
        let start = r
            .checked_sub(1)
            .and_then(|p| ends.get(p))
            .copied()
            .unwrap_or(0);
        let top = (bottom + 1).saturating_sub(height);
        if start < top {
            bottom = clamp((start + height).saturating_sub(1).cast_signed());
        } else if end - 1 > bottom {
            bottom = clamp((end - 1).cast_signed());
        }
    }
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
    let shown = bottom + 1 - top;
    let mut raw_images: Vec<(usize, ImageSlot)> = Vec::new();
    for (b, end) in blocks.iter().zip(&ends) {
        let start = end - b.lines.len();
        for slot in &b.images {
            raw_images.push((start, slot.clone()));
        }
    }
    let images = visible_slots(raw_images, top, shown);
    let rows = blocks
        .iter()
        .flat_map(Block::rows)
        .skip(top)
        .take(shown)
        .collect();
    let lines = blocks
        .into_iter()
        .flat_map(|b| b.lines)
        .skip(top)
        .take(shown)
        .collect();
    Window {
        lines,
        scroll,
        at_top: top == 0,
        rows,
        images,
    }
}

fn visible_slots(
    raw: Vec<(usize, ImageSlot)>,
    window_top: usize,
    window_height: usize,
) -> Vec<ImageSlot> {
    let top = window_top.cast_signed();
    let win_h = window_height.cast_signed();
    raw.into_iter()
        .filter_map(|(block_start, mut slot)| {
            let y = block_start.cast_signed() + isize::from(slot.y) - top;
            let bottom = y + usize::from(slot.height).cast_signed();
            if bottom <= 0 || y >= win_h {
                return None;
            }
            slot.y = i16::try_from(y).ok()?;
            Some(slot)
        })
        .collect()
}

/// Message `i` with the date rule and unread rule that go above it.
fn block(context: &Context<'_>, i: usize) -> Block {
    let Context {
        state,
        open,
        view,
        columns,
        width,
        today,
    } = context;
    let mut block = Vec::new();
    let mut images = Vec::new();
    let Some(message) = open.messages.get(i) else {
        return Block {
            lines: block,
            rules: 0,
            message: None,
            images,
        };
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
    let rules = match state.message_layout() {
        MessageLayout::Irc => {
            let rules = block.len();
            render_message(
                state,
                message,
                !continued,
                columns,
                view,
                &mut block,
                &mut images,
            );
            rules
        }
        MessageLayout::Bubbles => {
            // A blank line between bubbles (even from the same sender, or
            // they'd merge into one box), unless a rule already separates them.
            if block.is_empty() && previous.is_some() {
                block.push(Line::default());
            }
            let rules = block.len();
            bubbles::render(
                state,
                message,
                !continued,
                *width,
                view,
                &mut block,
                &mut images,
            );
            rules
        }
    };
    Block {
        lines: block,
        rules,
        message: Some(message.reference()),
        images,
    }
}

fn render_message(
    state: &State,
    message: &Message,
    show_name: bool,
    columns: &Columns,
    view: &View<'_>,
    out: &mut Vec<Line<'static>>,
    images: &mut Vec<ImageSlot>,
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
    let (mut body, pending_images) = content(state, message, columns.text_width, view);
    append_suffix(&mut body, suffix(message, view), columns.text_width);
    if !message.reactions.is_empty() {
        body.push(vec![Span::styled(reactions(message), theme.muted())]);
    }

    let body_start = out.len();
    for (i, spans) in body.into_iter().enumerate() {
        let mut line = if i == 0 {
            std::mem::take(&mut prefix)
        } else {
            vec![Span::raw(indent.clone())]
        };
        line.extend(spans);
        out.push(Line::from(line));
    }
    let x = u16::try_from(columns.text_start).unwrap_or(0);
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
}

/// What a message says, as lines of at most `max` cells: the quoted
/// message, the text (or the deleted placeholder) and attachments, plus
/// any inline image boxes reserved above attachment lines (D78, D79).
pub(super) fn content(
    state: &State,
    message: &Message,
    max: usize,
    view: &View<'_>,
) -> (Vec<Vec<Span<'static>>>, Vec<PendingImage>) {
    let theme = view.theme;
    let mut body: Vec<Vec<Span<'static>>> = Vec::new();
    let mut pending_images: Vec<PendingImage> = Vec::new();
    if let Some(quote) = &message.quote {
        let who = state.person_name(quote.target.sender);
        let preview = quote
            .preview
            .as_ref()
            .map_or_else(|| "…".to_owned(), |p| one_line(p.expose()));
        let text = format!("▎ {who}: {preview}");
        body.push(vec![Span::styled(
            truncate(&text, max).into_owned(),
            theme.muted().add_modifier(Modifier::ITALIC),
        )]);
    }

    if message.deleted {
        body.push(vec![Span::styled(
            "This message was deleted",
            theme.muted().add_modifier(Modifier::ITALIC),
        )]);
    } else if let Some(text) = &message.body {
        body.extend(text_lines(state, message, text.expose(), max, view));
    }
    let msg_ref = message.reference();
    for (index, attachment) in message.attachments.iter().enumerate() {
        let show_box = view.images.active_protocol().is_some()
            && attachment.is_image()
            && match &attachment.status {
                AttachmentStatus::Downloaded(_) => true,
                AttachmentStatus::Remote | AttachmentStatus::Failed => {
                    state.is_downloading(msg_ref, index)
                }
            };
        if show_box
            && let (Some(w), Some(h)) = (attachment.width, attachment.height)
            && w > 0
            && h > 0
        {
            let is_sticker = attachment.file_name.is_none()
                && (attachment.content_type == "image/webp"
                    || message
                        .body
                        .as_ref()
                        .is_some_and(|b| b.expose().starts_with("[sticker]")));
            let max_rows = if is_sticker {
                crate::app::images::MAX_STICKER_ROWS
            } else {
                crate::app::images::MAX_IMAGE_ROWS
            };
            let max_cols = crate::app::images::MAX_IMAGE_COLS
                .min(u16::try_from(max).unwrap_or(u16::MAX))
                .max(1);
            let size =
                crate::app::images::box_size(w, h, view.images.font_size(), max_cols, max_rows);
            pending_images.push(PendingImage {
                row: body.len(),
                width: size.width,
                height: size.height,
                path: attachment.local_path().map(std::path::Path::to_owned),
            });
            let pad = " ".repeat(usize::from(size.width));
            for _ in 0..size.height {
                body.push(vec![Span::raw(pad.clone())]);
            }
        }
        if attachment.is_voice_note() {
            body.extend(voice_note_lines(
                state, msg_ref, index, attachment, max, view,
            ));
            continue;
        }
        let name = attachment.file_name.as_deref().map_or_else(
            || attachment.content_type.clone(),
            |n| sanitize(n).into_owned(),
        );
        let size = attachment.size.map(format_size).unwrap_or_default();
        let status = match &attachment.status {
            AttachmentStatus::Downloaded(_) => "",
            AttachmentStatus::Remote | AttachmentStatus::Failed
                if state.is_downloading(msg_ref, index) =>
            {
                " · downloading…"
            }
            AttachmentStatus::Remote => " · not downloaded (d)",
            AttachmentStatus::Failed => " · download failed (d)",
        };
        let text = format!("📎 {name}{size}{status}");
        body.push(vec![Span::styled(
            truncate(&text, max).into_owned(),
            Style::new().fg(theme.secondary),
        )]);
    }
    if body.is_empty() {
        body.push(vec![Span::styled("(empty message)", theme.muted())]);
    }
    (body, pending_images)
}

/// Render a voice note row (`▶ 0:14 ▂▄▆█… · size` or `■ 0:05 / 0:14 ▂▄▆█… · playing`)
/// and its transcript below when present (D80, D81, D82).
fn voice_note_lines(
    state: &State,
    msg_ref: MessageRef,
    index: usize,
    attachment: &faro_core::domain::AttachmentRef,
    max: usize,
    view: &View<'_>,
) -> Vec<Vec<Span<'static>>> {
    let theme = view.theme;
    let mut lines: Vec<Vec<Span<'static>>> = Vec::new();
    let levels = attachment
        .waveform
        .as_deref()
        .filter(|w| !w.is_empty())
        .map_or_else(
            || faro_core::audio::synthetic_waveform(attachment.id.0.as_bytes()),
            <[u8]>::to_vec,
        );
    let waveform = faro_core::audio::format_waveform(&levels);
    let bars: Vec<char> = waveform.chars().collect();
    let duration = attachment
        .duration_secs
        .map_or_else(|| "0:00".to_owned(), faro_core::audio::format_duration);

    let spans = if let Some(elapsed) = state.playing_voice_note(msg_ref, index) {
        let elapsed_str = faro_core::audio::format_duration(elapsed);
        let head = format!("■ {elapsed_str} / {duration} ");
        let tail = " · playing";
        let played_count = match attachment.duration_secs.filter(|&d| d > 0) {
            Some(total) => {
                (usize::try_from(elapsed.min(total)).unwrap_or(0) * bars.len())
                    / usize::try_from(total).unwrap_or(1)
            }
            None => usize::try_from(elapsed).unwrap_or(0) % (bars.len() + 1),
        }
        .min(bars.len());
        let played: String = bars.iter().take(played_count).collect();
        let remaining: String = bars.iter().skip(played_count).collect();
        let full = format!("{head}{played}{remaining}{tail}");
        if width(&full) <= max {
            let mut row = vec![Span::styled(
                head,
                Style::new().fg(theme.accent).add_modifier(Modifier::BOLD),
            )];
            if !played.is_empty() {
                row.push(Span::styled(played, Style::new().fg(theme.accent)));
            }
            if !remaining.is_empty() {
                row.push(Span::styled(remaining, theme.muted()));
            }
            row.push(Span::styled(tail.to_owned(), Style::new().fg(theme.accent)));
            row
        } else {
            vec![Span::styled(
                truncate(&full, max).into_owned(),
                Style::new().fg(theme.accent),
            )]
        }
    } else {
        let size = attachment.size.map(format_size).unwrap_or_default();
        let status = match &attachment.status {
            AttachmentStatus::Downloaded(_) => "",
            AttachmentStatus::Remote | AttachmentStatus::Failed
                if state.is_downloading(msg_ref, index) =>
            {
                " · downloading…"
            }
            AttachmentStatus::Remote => " · not downloaded (d)",
            AttachmentStatus::Failed => " · download failed (d)",
        };
        let head = format!("▶ {duration} ");
        let tail = format!("{size}{status}");
        let full = format!("{head}{waveform}{tail}");
        if width(&full) <= max {
            let bar_style = if attachment.is_downloaded() {
                Style::new().fg(theme.accent)
            } else {
                theme.muted()
            };
            vec![
                Span::styled(head, Style::new().fg(theme.secondary)),
                Span::styled(waveform, bar_style),
                Span::styled(tail, theme.muted()),
            ]
        } else {
            vec![Span::styled(
                truncate(&full, max).into_owned(),
                Style::new().fg(theme.secondary),
            )]
        }
    };
    lines.push(spans);
    lines.extend(transcript_lines(
        state, msg_ref, index, attachment, max, view,
    ));
    lines
}

fn transcript_lines(
    state: &State,
    msg_ref: MessageRef,
    index: usize,
    attachment: &faro_core::domain::AttachmentRef,
    max: usize,
    view: &View<'_>,
) -> Vec<Vec<Span<'static>>> {
    let theme = view.theme;
    let Some(transcript) = attachment.transcript.as_deref().filter(|t| !t.is_empty()) else {
        if state.is_transcribing(msg_ref, index) {
            return vec![vec![Span::styled(
                truncate("❝ transcribing…", max).into_owned(),
                theme.muted().add_modifier(Modifier::ITALIC),
            )]];
        }
        return Vec::new();
    };
    let quoted = format!("❝ {}", sanitize(transcript));
    let tag = attachment
        .transcript_source
        .map(|s| format!(" [{}]", s.as_str()))
        .unwrap_or_default();
    let text_style = Style::new().fg(theme.text).add_modifier(Modifier::ITALIC);
    let mut pieces = vec![(quoted.as_str(), text_style)];
    if !tag.is_empty() {
        pieces.push((tag.as_str(), theme.muted()));
    }
    wrap_pieces(&pieces, max)
        .into_iter()
        .map(|line| {
            line.into_iter()
                .map(|(text, style)| Span::styled(text, style))
                .collect()
        })
        .collect()
}

/// A message's text wrapped to `max` cells, mentions drawn as `@Name` in
/// the accent colour, and mentions of this account reversed (D72).
fn text_lines(
    state: &State,
    message: &Message,
    text: &str,
    max: usize,
    view: &View<'_>,
) -> Vec<Vec<Span<'static>>> {
    let theme = view.theme;
    let plain = Style::new().fg(theme.text);
    let mention = Style::new().fg(theme.accent).add_modifier(Modifier::BOLD);
    let names: Vec<(String, Style)> = mention_parts(text, &message.mentions)
        .into_iter()
        .map(|part| match part {
            BodyPart::Text(text) => (text.to_owned(), plain),
            BodyPart::Mention(who) if who == state.me => (
                state.mention_name(who),
                mention.add_modifier(Modifier::REVERSED),
            ),
            BodyPart::Mention(who) => (state.mention_name(who), mention),
        })
        .collect();
    let pieces: Vec<(&str, Style)> = names.iter().map(|(t, s)| (t.as_str(), *s)).collect();
    wrap_pieces(&pieces, max)
        .into_iter()
        .map(|line| {
            line.into_iter()
                .map(|(text, style)| Span::styled(text, style))
                .collect()
        })
        .collect()
}

/// `(edited)` and the delivery status of own messages.
fn suffix(message: &Message, view: &View<'_>) -> Vec<Span<'static>> {
    let theme = view.theme;
    let mut spans = Vec::new();
    if message.edited_at.is_some() && !message.deleted {
        spans.push(Span::styled(" (edited)", theme.muted()));
    }
    spans.extend(delivery(message, view));
    spans
}

/// The delivery status of an own message (` ✓✓`), with a leading space.
pub(super) fn delivery(message: &Message, view: &View<'_>) -> Option<Span<'static>> {
    let theme = view.theme;
    let Direction::Outgoing(status) = message.direction else {
        return None;
    };
    let (glyph, style) = match status {
        DeliveryStatus::Sending => (" ◌", theme.muted()),
        DeliveryStatus::Sent => (" ✓", theme.muted()),
        DeliveryStatus::Delivered => (" ✓✓", theme.muted()),
        DeliveryStatus::Read => (" ✓✓", Style::new().fg(theme.accent)),
        DeliveryStatus::Failed => (" ! not sent", Style::new().fg(theme.error)),
    };
    Some(Span::styled(glyph, style))
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
pub(super) fn reactions(message: &Message) -> String {
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

pub(super) fn zoned(ts: jiff::Timestamp, view: &View<'_>) -> jiff::Zoned {
    ts.to_zoned(view.tz.clone())
}

pub(super) fn to_jiff(ts: Timestamp) -> Option<jiff::Timestamp> {
    let ms = i64::try_from(ts.as_millis()).ok()?;
    jiff::Timestamp::from_millisecond(ms).ok()
}

fn date_of(ts: jiff::Timestamp, view: &View<'_>) -> Date {
    zoned(ts, view).date()
}

fn date_of_message(message: &Message, view: &View<'_>) -> Option<Date> {
    to_jiff(message.sent_at).map(|ts| date_of(ts, view))
}

pub(super) fn format_time(ts: Timestamp, view: &View<'_>) -> String {
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
