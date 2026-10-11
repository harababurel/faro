//! Popups (D40, D41): the message action menu and the reaction picker over
//! the message area; the conversation menu and the mute picker next to the
//! sidebar. Drawn last, over everything else.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Paragraph};

use super::View;
use super::messages::{to_jiff, zoned};
use super::shortcodes::{REACT_SEARCH_LIMIT, REACT_SEARCH_ROWS, search_shortcodes};
use super::text::{pad_left, sanitize, truncate, width};
use faro_core::domain::{
    ConversationId, DeliveryStatus, Direction, MessageRef, Timestamp, format_byte_size,
};
use faro_core::store::RecipientReceipt;

use crate::app::state::{
    Geometry, MUTE_CHOICES, MessageAction, Popup, REMOTE_CONSENT_CHOICES, SidebarRow, State,
    same_emoji,
};

/// Shortcut keys of `MUTE_CHOICES`.
const MUTE_KEYS: [&str; 5] = ["1", "2", "3", "4", "5"];

pub fn render(frame: &mut Frame<'_>, state: &State, view: &View<'_>, geometry: &mut Geometry) {
    let Some(popup) = &state.popup else {
        return;
    };
    match popup {
        Popup::Actions {
            target,
            items,
            selected,
        } => {
            let Some(anchor) = over_message(geometry, *target) else {
                return;
            };
            let labels: Vec<(String, &str)> = items.iter().map(MessageAction::label).collect();
            menu(frame, anchor, "Message", &labels, *selected, view);
        }
        Popup::React {
            target,
            selected,
            query,
        } => {
            let Some(anchor) = over_message(geometry, *target) else {
                return;
            };
            let mine = state
                .open
                .as_ref()
                .and_then(|o| o.messages.iter().rfind(|m| m.reference() == *target))
                .and_then(|m| m.reactions.iter().find(|r| r.sender == state.me))
                .map(|r| r.emoji.as_str());
            if let Some(query) = query {
                react_search(frame, anchor, *selected, query, mine, view);
            } else {
                react(frame, anchor, state, *selected, mine, view);
            }
        }
        Popup::MessageInfo { target, receipts } => {
            let Some(anchor) = over_message(geometry, *target) else {
                return;
            };
            message_info(frame, anchor, state, *target, receipts, view);
        }
        Popup::Conversation {
            target,
            items,
            selected,
        } => {
            let Some(anchor) = beside_sidebar(frame.area(), geometry, *target) else {
                return;
            };
            let labels: Vec<(String, &str)> = items
                .iter()
                .map(|a| {
                    let (label, key) = a.label();
                    (label.to_owned(), key)
                })
                .collect();
            menu(frame, anchor, "Conversation", &labels, *selected, view);
        }
        Popup::Mute { target, selected } => {
            let Some(anchor) = beside_sidebar(frame.area(), geometry, *target) else {
                return;
            };
            let labels: Vec<(String, &str)> = MUTE_CHOICES
                .iter()
                .zip(MUTE_KEYS)
                .map(|((label, _), key)| ((*label).to_owned(), key))
                .collect();
            menu(frame, anchor, "Mute", &labels, *selected, view);
        }
        Popup::ImageViewer { target, index } => {
            attachment_viewer(frame, state, *target, *index, geometry, view);
        }
        Popup::StagedImage => staged_viewer(frame, state, geometry, view),
        Popup::About => about(frame, state, view),
        Popup::ConfirmDelete { target } => {
            let Some(anchor) = over_message(geometry, *target) else {
                return;
            };
            confirm_delete(frame, anchor, view);
        }
        Popup::ConfirmRemoteTranscription {
            conversation,
            target,
            selected,
            ..
        } => {
            let Some(anchor) = over_message(geometry, *target) else {
                return;
            };
            remote_consent(frame, anchor, state, *conversation, *selected, view);
        }
    }
}

fn message_info(
    frame: &mut Frame<'_>,
    anchor: Anchor,
    state: &State,
    target: MessageRef,
    receipts: &[RecipientReceipt],
    view: &View<'_>,
) {
    let Some(message) = state
        .open
        .as_ref()
        .and_then(|o| o.messages.iter().rfind(|m| m.reference() == target))
    else {
        return;
    };
    let theme = view.theme;
    let rows = message_info_rows(state, message, receipts, view);
    let widest = rows
        .iter()
        .map(|(k, v, _)| width(k) + width(v) + 4)
        .max()
        .unwrap_or(0);
    let max_inner = usize::from(anchor.area.width.saturating_sub(4));
    let inner_width = widest.clamp(24, max_inner.max(24));
    let rect = place(anchor, inner_width, rows.len());
    let inner = frame_box(frame, rect, "Message info", view);
    let row_width = usize::from(inner.width);
    let lines: Vec<Line<'static>> = rows
        .into_iter()
        .map(|(key, val, val_style)| {
            let prefix = format!(" {key}: ");
            let room = row_width.saturating_sub(width(&prefix) + 1);
            let val = truncate(&val, room).into_owned();
            Line::from(vec![
                Span::styled(prefix, theme.muted()),
                Span::styled(val, val_style),
            ])
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), inner);
}

fn message_info_rows(
    state: &State,
    message: &faro_core::domain::Message,
    receipts: &[RecipientReceipt],
    view: &View<'_>,
) -> Vec<(&'static str, String, Style)> {
    let theme = view.theme;
    let text_style = Style::new().fg(theme.text);
    let sender = if message.sender == state.me {
        "You".to_owned()
    } else {
        sanitize(&state.person_name(message.sender)).into_owned()
    };
    let mut rows: Vec<(&'static str, String, Style)> = vec![
        ("From", sender, text_style.add_modifier(Modifier::BOLD)),
        ("Sent", format_datetime(message.sent_at, view), text_style),
    ];
    if let Some(edited_at) = message.edited_at {
        rows.push(("Edited", format_datetime(edited_at, view), text_style));
    }
    if let Direction::Outgoing(status) = message.direction {
        let (label, style) = match status {
            DeliveryStatus::Sending => ("Sending…", theme.muted()),
            DeliveryStatus::Sent => ("Sent ✓", theme.muted()),
            DeliveryStatus::Delivered => ("Delivered ✓✓", theme.muted()),
            DeliveryStatus::Read => ("Read ✓✓", Style::new().fg(theme.accent)),
            DeliveryStatus::Failed => ("Failed !", Style::new().fg(theme.error)),
        };
        rows.push(("Status", label.to_owned(), style));
    }
    for receipt in receipts.iter().filter(|r| r.status == DeliveryStatus::Read) {
        let name = state.person_name(receipt.recipient);
        let who = sanitize(&name);
        let at = format_time(receipt.updated_at, view);
        rows.push((
            "Read by",
            format!("{who} {at}"),
            Style::new().fg(theme.accent),
        ));
    }
    for receipt in receipts
        .iter()
        .filter(|r| r.status == DeliveryStatus::Delivered)
    {
        let name = state.person_name(receipt.recipient);
        let who = sanitize(&name);
        let at = format_time(receipt.updated_at, view);
        rows.push(("Delivered to", format!("{who} {at}"), text_style));
    }
    // Who reacted with what, the same emoji together (in the order each
    // first appeared); ours first within its emoji.
    let mut reactions: Vec<&faro_core::domain::Reaction> = message.reactions.iter().collect();
    let first_seen = |emoji: &str| {
        message
            .reactions
            .iter()
            .position(|r| same_emoji(&r.emoji, emoji))
            .unwrap_or(usize::MAX)
    };
    reactions.sort_by_key(|r| (first_seen(&r.emoji), r.sender != state.me));
    for reaction in reactions {
        let who = if reaction.sender == state.me {
            "You".to_owned()
        } else {
            sanitize(&state.person_name(reaction.sender)).into_owned()
        };
        let emoji = sanitize(&reaction.emoji);
        rows.push(("Reaction", format!("{emoji} {who}"), text_style));
    }
    for att in &message.attachments {
        let name = att.file_name.as_deref().map_or_else(
            || {
                if att.is_voice_note() {
                    "voice note".to_owned()
                } else {
                    sanitize(&att.content_type).into_owned()
                }
            },
            |n| sanitize(n).into_owned(),
        );
        let mut meta = vec![sanitize(&att.content_type).into_owned()];
        if let Some(size) = att.size {
            meta.push(format_byte_size(size));
        }
        if let (Some(w), Some(h)) = (att.width, att.height)
            && w > 0
            && h > 0
        {
            meta.push(format!("{w}×{h}"));
        }
        if let Some(secs) = att.duration_secs {
            meta.push(format!("{secs}s"));
        }
        rows.push((
            "Attachment",
            format!("{name} ({})", meta.join(" · ")),
            text_style,
        ));
        if let Some(path) = att.local_path() {
            rows.push((
                "Path",
                sanitize(&path.display().to_string()).into_owned(),
                Style::new().fg(theme.secondary),
            ));
        }
    }
    rows
}

fn format_datetime(ts: Timestamp, view: &View<'_>) -> String {
    to_jiff(ts).map_or_else(
        || format!("{} ms", ts.as_millis()),
        |t| zoned(t, view).strftime("%Y-%m-%d %H:%M:%S").to_string(),
    )
}

fn format_time(ts: Timestamp, view: &View<'_>) -> String {
    to_jiff(ts).map_or_else(
        || format!("{} ms", ts.as_millis()),
        |t| zoned(t, view).strftime("%H:%M:%S").to_string(),
    )
}

/// What faro is and where it comes from (D94): the small lighthouse beside
/// the version, links, licence and credits, centred on the screen.
fn about(frame: &mut Frame<'_>, state: &State, view: &View<'_>) {
    let theme = view.theme;
    let data_dir = state
        .attachments_dir
        .parent()
        .map(|dir| dir.display().to_string())
        .unwrap_or_default();
    // Full URLs, scheme included, so terminals detect them as links.
    let repository = env!("CARGO_PKG_REPOSITORY");
    let mut rows: Vec<(String, Style)> = vec![
        (
            format!("faro {}", crate::version::shown()),
            Style::new().fg(theme.primary).add_modifier(Modifier::BOLD),
        ),
        (super::logo::TAGLINE.to_owned(), theme.muted()),
        (String::new(), theme.muted()),
        (
            "https://getfaro.dev".to_owned(),
            Style::new().fg(theme.accent),
        ),
        (repository.to_owned(), Style::new().fg(theme.accent)),
        (String::new(), theme.muted()),
        (
            format!("{} · not affiliated with Signal", env!("CARGO_PKG_LICENSE")),
            Style::new().fg(theme.text),
        ),
        (
            "Signal protocol: presage and libsignal".to_owned(),
            theme.muted(),
        ),
        (
            "Themes: faro's own and OpenCode's (MIT)".to_owned(),
            theme.muted(),
        ),
    ];
    let config_path = state.config_path.display().to_string();
    if !config_path.is_empty() {
        rows.push((format!("Config: {}", sanitize(&config_path)), theme.muted()));
    }
    if !data_dir.is_empty() {
        rows.push((format!("Data:   {}", sanitize(&data_dir)), theme.muted()));
    }
    let (logo_width, logo_height) = super::logo::SMALL_SIZE;
    let text_width = rows.iter().map(|(text, _)| width(text)).max().unwrap_or(0);
    let wanted_width = usize::from(logo_width) + 3 + text_width;
    let area = frame.area();
    let inner_width = u16::try_from(wanted_width)
        .unwrap_or(u16::MAX)
        .min(area.width.saturating_sub(4));
    let inner_height = u16::try_from(rows.len())
        .unwrap_or(u16::MAX)
        .max(logo_height)
        .min(area.height.saturating_sub(2));
    let rect = Rect {
        x: area.x + area.width.saturating_sub(inner_width + 2) / 2,
        y: area.y + area.height.saturating_sub(inner_height + 2) / 2,
        width: (inner_width + 2).min(area.width),
        height: (inner_height + 2).min(area.height),
    };
    let inner = frame_box(frame, rect, "About", view);
    // The lighthouse only if there's room for it beside the text.
    let text_area = if inner.width > logo_width + 3 + 20 {
        let y = inner.y + inner.height.saturating_sub(logo_height) / 2;
        super::logo::draw_small_lighthouse(frame.buffer_mut(), inner, inner.x + 1, y, theme);
        Rect {
            x: inner.x + logo_width + 3,
            width: inner.width - logo_width - 3,
            ..inner
        }
    } else {
        inner
    };
    let room = usize::from(text_area.width);
    let lines: Vec<Line<'_>> = rows
        .into_iter()
        .map(|(text, style)| Line::from(Span::styled(truncate(&text, room).into_owned(), style)))
        .collect();
    frame.render_widget(Paragraph::new(lines), text_area);
}

/// `v` on a downloaded image (D76).
fn attachment_viewer(
    frame: &mut Frame<'_>,
    state: &State,
    target: MessageRef,
    index: usize,
    geometry: &mut Geometry,
    view: &View<'_>,
) {
    let Some(attachment) = state
        .open
        .as_ref()
        .and_then(|o| o.messages.iter().rfind(|m| m.reference() == target))
        .and_then(|m| m.attachments.get(index))
    else {
        return;
    };
    let shown = Shown {
        name: attachment.file_name.as_deref().map_or_else(
            || attachment.content_type.clone(),
            |n| sanitize(n).into_owned(),
        ),
        width: attachment.width,
        height: attachment.height,
        path: attachment.local_path(),
        keys: "s save to Downloads · y copy path · Esc close",
    };
    image_viewer(frame, state, &shown, geometry, view);
}

/// `Ctrl+O`: the staged image, before it's sent (D91).
fn staged_viewer(frame: &mut Frame<'_>, state: &State, geometry: &mut Geometry, view: &View<'_>) {
    let Some(staged) = state
        .open
        .as_ref()
        .and_then(|o| o.staged_attachment.as_ref())
    else {
        return;
    };
    let shown = Shown {
        name: sanitize(&staged.name).into_owned(),
        width: staged.width,
        height: staged.height,
        path: Some(staged.path.as_path()),
        keys: "Not sent yet · Enter in the input sends it · Esc close",
    };
    image_viewer(frame, state, &shown, geometry, view);
}

/// An image for the full-size viewer: a downloaded attachment (`v`, D76) or
/// the staged one before it's sent (`Ctrl+O`, D91).
struct Shown<'a> {
    name: String,
    width: Option<u32>,
    height: Option<u32>,
    path: Option<&'a std::path::Path>,
    /// The keys line under the image.
    keys: &'static str,
}

fn image_viewer(
    frame: &mut Frame<'_>,
    state: &State,
    shown: &Shown<'_>,
    geometry: &mut Geometry,
    view: &View<'_>,
) {
    let area = geometry.history.unwrap_or_else(|| frame.area());
    let width = area.width.saturating_sub(4).max(20).min(area.width);
    let height = area.height.saturating_sub(2).max(6).min(area.height);
    let rect = Rect {
        x: area.x + (area.width.saturating_sub(width)) / 2,
        y: area.y + (area.height.saturating_sub(height)) / 2,
        width,
        height,
    };
    let dims = match (shown.width, shown.height) {
        (Some(w), Some(h)) if w > 0 && h > 0 => format!(" ({w}×{h})"),
        _ => String::new(),
    };
    let title = format!("{}{dims}", shown.name);
    let inner = frame_box(frame, rect, &title, view);
    let info_rows = 2u16.min(inner.height);
    let img_avail_h = inner.height.saturating_sub(info_rows);
    if img_avail_h > 0 && inner.width > 0 {
        let size = match (shown.width, shown.height) {
            (Some(w), Some(h)) if w > 0 && h > 0 => crate::app::image_cache::box_size(
                w,
                h,
                view.images.font_size(),
                inner.width,
                img_avail_h,
            ),
            _ => ratatui::layout::Size::new(inner.width, img_avail_h),
        };
        let img_x = (inner.width.saturating_sub(size.width)) / 2;
        let img_y = i16::try_from((img_avail_h.saturating_sub(size.height)) / 2).unwrap_or(0);
        let img_clip = Rect {
            height: img_avail_h,
            ..inner
        };
        super::render_image_box(
            frame,
            state,
            view,
            geometry,
            img_clip,
            img_x,
            img_y,
            size.width,
            size.height,
            shown.path,
            state.palette.is_some(),
        );
    }
    let path_line = shown
        .path
        .map_or_else(String::new, |p| p.display().to_string());
    let info = vec![
        Line::from(Span::styled(
            truncate(&sanitize(&path_line), usize::from(inner.width)).into_owned(),
            view.theme.muted(),
        )),
        Line::from(Span::styled(shown.keys, view.theme.muted())),
    ];
    let info_area = Rect {
        y: inner.y + img_avail_h,
        height: info_rows,
        ..inner
    };
    frame.render_widget(Paragraph::new(info), info_area);
}

/// In the message area, next to the rows of the message `target` (so the
/// popup doesn't cover them).
fn over_message(geometry: &Geometry, target: MessageRef) -> Option<Anchor> {
    let area = geometry.history?;
    let mut rows = geometry
        .history_rows
        .iter()
        .enumerate()
        .filter(|(_, m)| **m == Some(target))
        .map(|(i, _)| i);
    let first = rows.next();
    let around = first.map(|first| (first, rows.next_back().unwrap_or(first)));
    Some(Anchor { area, around })
}

/// Next to `target`'s sidebar row, over whatever is right of the sidebar's
/// left edge; at the bottom of the message area if the sidebar is hidden.
fn beside_sidebar(screen: Rect, geometry: &Geometry, target: ConversationId) -> Option<Anchor> {
    let Some(sidebar) = geometry.sidebar else {
        return geometry.history.map(|area| Anchor { area, around: None });
    };
    let row = geometry
        .sidebar_rows
        .iter()
        .position(|r| *r == SidebarRow::Conversation(target));
    Some(Anchor {
        area: Rect {
            width: screen.width.saturating_sub(sidebar.x),
            ..sidebar
        },
        around: row.map(|r| (r, r)),
    })
}

/// Where a popup goes: inside `area`, next to the rows `around` (first and
/// last, from the top of `area`) if known.
#[derive(Clone, Copy)]
struct Anchor {
    area: Rect,
    around: Option<(usize, usize)>,
}

/// A bordered box for `inner_width` x `inner_height`: just above the message
/// it's about, else just below it, else at the bottom of the area.
fn place(anchor: Anchor, inner_width: usize, inner_height: usize) -> Rect {
    let area = anchor.area;
    let width = u16::try_from(inner_width + 2)
        .unwrap_or(u16::MAX)
        .min(area.width.saturating_sub(2));
    let height = u16::try_from(inner_height + 2)
        .unwrap_or(u16::MAX)
        .min(area.height);
    let bottom_y = area.y + area.height - height;
    let y = anchor.around.map_or(bottom_y, |(first, last)| {
        let first = u16::try_from(first).unwrap_or(u16::MAX);
        let below = u16::try_from(last + 1).unwrap_or(u16::MAX);
        if first >= height {
            area.y + first - height
        } else if below.saturating_add(height) <= area.height {
            area.y + below
        } else {
            bottom_y
        }
    });
    Rect {
        x: area.x + 1,
        y,
        width,
        height,
    }
}

fn frame_box(frame: &mut Frame<'_>, rect: Rect, title: &str, view: &View<'_>) -> Rect {
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(view.theme.border(true))
        .style(view.theme.panel())
        .title(Span::styled(
            format!(" {title} "),
            Style::new()
                .fg(view.theme.primary)
                .add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(rect);
    frame.render_widget(Clear, rect);
    frame.render_widget(block, rect);
    inner
}

/// A list of labelled items with their shortcut keys, `selected` marked.
fn menu(
    frame: &mut Frame<'_>,
    anchor: Anchor,
    title: &str,
    labels: &[(String, &str)],
    selected: usize,
    view: &View<'_>,
) {
    let theme = view.theme;
    // " › label   key "
    let widest = labels
        .iter()
        .map(|(label, key)| width(label) + width(key))
        .max()
        .unwrap_or(0);
    let max_inner = usize::from(anchor.area.width.saturating_sub(4));
    let inner_width = (widest + 6).clamp(20, max_inner.max(20));
    let rect = place(anchor, inner_width, labels.len());
    let inner = frame_box(frame, rect, title, view);
    let row_width = usize::from(inner.width);
    let lines: Vec<Line<'static>> = labels
        .iter()
        .enumerate()
        .map(|(i, (label, key))| {
            let chosen = i == selected;
            let marker = if chosen { " › " } else { "   " };
            let key_width = width(key) + 1;
            let room = row_width.saturating_sub(3 + key_width);
            let label = truncate(label, room).into_owned();
            let gap = row_width.saturating_sub(3 + width(&label) + key_width);
            let style = if chosen {
                theme.selected().fg(theme.text).add_modifier(Modifier::BOLD)
            } else {
                Style::new().fg(theme.text)
            };
            Line::from(vec![
                Span::styled(marker, style.fg(theme.primary)),
                Span::styled(label, style),
                Span::styled(" ".repeat(gap), style),
                Span::styled(pad_left(key, key_width), style.patch(theme.muted())),
            ])
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), inner);
}

fn react(
    frame: &mut Frame<'_>,
    anchor: Anchor,
    state: &State,
    selected: usize,
    mine: Option<&str>,
    view: &View<'_>,
) {
    let theme = view.theme;
    let mut spans = vec![Span::raw(" ")];
    for (i, emoji) in state.quick_reactions().into_iter().enumerate() {
        let mut style = Style::new().fg(theme.text);
        if mine.is_some_and(|m| same_emoji(m, emoji)) {
            style = style.add_modifier(Modifier::UNDERLINED);
        }
        if i == selected {
            style = style.patch(theme.selected()).add_modifier(Modifier::BOLD);
        }
        spans.push(Span::styled(
            format!("{} ", i + 1),
            style.patch(theme.muted()),
        ));
        spans.push(Span::styled(format!("{} ", sanitize(emoji)), style));
        spans.push(Span::raw(" "));
    }
    // Every other emoji is a search away (D104).
    spans.push(Span::styled("/ search ", theme.muted()));
    let line = Line::from(spans);
    let rect = place(anchor, line.width(), 1);
    let inner = frame_box(frame, rect, "React", view);
    frame.render_widget(Paragraph::new(line), inner);
}

fn react_search(
    frame: &mut Frame<'_>,
    anchor: Anchor,
    selected: usize,
    query: &str,
    mine: Option<&str>,
    view: &View<'_>,
) {
    let theme = view.theme;
    let matches = search_shortcodes(query, REACT_SEARCH_LIMIT);
    // At most `REACT_SEARCH_ROWS` at once, fewer if the pane is short (two
    // border rows and the prompt); the window follows the selection (D104).
    let room = usize::from(anchor.area.height).saturating_sub(3).max(1);
    let visible = matches.len().min(REACT_SEARCH_ROWS).min(room).max(1);
    let clamped = selected.min(matches.len().saturating_sub(1));
    let offset = (clamped + 1).saturating_sub(visible);
    let counter = if matches.len() > visible {
        format!("{}/{}", clamped + 1, matches.len())
    } else {
        String::new()
    };
    let widest = matches
        .iter()
        .map(|(code, emoji)| 3 + width(&sanitize(emoji)) + 3 + width(code) + 1)
        .max()
        .unwrap_or(22)
        .max(width(query) + 6 + width(&counter));
    let max_inner = usize::from(anchor.area.width.saturating_sub(4));
    let inner_width = widest.clamp(22, max_inner.max(22));
    let rect = place(anchor, inner_width, 1 + visible);
    let inner = frame_box(frame, rect, "React", view);
    let row_width = usize::from(inner.width);

    let counter_w = width(&counter);
    let prompt_query = truncate(
        &sanitize(query),
        row_width.saturating_sub(4 + counter_w + 1),
    )
    .into_owned();
    let prompt_pad = row_width.saturating_sub(3 + width(&prompt_query) + counter_w + 1);
    let mut lines = Vec::with_capacity(1 + visible);
    lines.push(Line::from(vec![
        Span::styled(
            " :",
            Style::new().fg(theme.primary).add_modifier(Modifier::BOLD),
        ),
        Span::styled(prompt_query, Style::new().fg(theme.text)),
        Span::styled("█", Style::new().fg(theme.primary)),
        Span::raw(" ".repeat(prompt_pad)),
        Span::styled(counter, theme.muted()),
    ]));

    if matches.is_empty() {
        lines.push(Line::from(Span::styled(
            truncate("   No matching emoji", row_width).into_owned(),
            theme.muted(),
        )));
    } else {
        let shown = matches.iter().enumerate().skip(offset).take(visible);
        for (i, &(code, emoji)) in shown {
            let chosen = i == clamped;
            let marker = if chosen { " › " } else { "   " };
            let clean_emoji = sanitize(emoji).into_owned();
            let emoji_w = width(&clean_emoji);
            let label = format!(" :{code}:");
            let label = truncate(&label, row_width.saturating_sub(3 + emoji_w)).into_owned();
            let pad = row_width.saturating_sub(3 + emoji_w + width(&label));
            let mut style = if chosen {
                theme.selected().fg(theme.text).add_modifier(Modifier::BOLD)
            } else {
                Style::new().fg(theme.text)
            };
            if mine.is_some_and(|m| same_emoji(m, emoji)) {
                style = style.add_modifier(Modifier::UNDERLINED);
            }
            let code_style = if chosen {
                style
            } else {
                style.patch(theme.muted())
            };
            lines.push(Line::from(vec![
                Span::styled(marker, style.fg(theme.primary)),
                Span::styled(clean_emoji, style),
                Span::styled(label, code_style),
                Span::styled(" ".repeat(pad), style),
            ]));
        }
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

fn remote_consent(
    frame: &mut Frame<'_>,
    anchor: Anchor,
    state: &State,
    conversation: ConversationId,
    selected: usize,
    view: &View<'_>,
) {
    let theme = view.theme;
    let chat = sanitize(&state.conversation_title(conversation)).into_owned();
    let header = format!(" Send audio from {chat} to:");
    let endpoint = format!(" {}", sanitize(&state.config.voice.transcribe_url));
    let labels: Vec<(String, &str)> = REMOTE_CONSENT_CHOICES
        .iter()
        .map(|&(label, key)| (label.to_owned(), key))
        .collect();
    let widest_item = labels
        .iter()
        .map(|(label, key)| width(label) + width(key) + 6)
        .max()
        .unwrap_or(0);
    let max_inner = usize::from(anchor.area.width.saturating_sub(4));
    let inner_width = widest_item
        .max(width(&header) + 1)
        .max(width(&endpoint) + 1)
        .clamp(24, max_inner.max(24));
    let rect = place(anchor, inner_width, labels.len() + 2);
    let inner = frame_box(frame, rect, "Remote transcription", view);
    let row_width = usize::from(inner.width);
    let mut lines = vec![
        Line::from(Span::styled(
            truncate(&header, row_width).into_owned(),
            theme.muted(),
        )),
        Line::from(Span::styled(
            truncate(&endpoint, row_width).into_owned(),
            Style::new().fg(theme.secondary),
        )),
    ];
    for (i, (label, key)) in labels.iter().enumerate() {
        let chosen = i == selected;
        let marker = if chosen { " › " } else { "   " };
        let key_width = width(key) + 1;
        let room = row_width.saturating_sub(3 + key_width);
        let label = truncate(label, room).into_owned();
        let gap = row_width.saturating_sub(3 + width(&label) + key_width);
        let style = if chosen {
            theme.selected().fg(theme.text).add_modifier(Modifier::BOLD)
        } else {
            Style::new().fg(theme.text)
        };
        lines.push(Line::from(vec![
            Span::styled(marker, style.fg(theme.primary)),
            Span::styled(label, style),
            Span::styled(" ".repeat(gap), style),
            Span::styled(pad_left(key, key_width), style.patch(theme.muted())),
        ]));
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

fn confirm_delete(frame: &mut Frame<'_>, anchor: Anchor, view: &View<'_>) {
    let prompt = " Delete this message for everyone? ";
    let keys = " Enter / y confirm · Esc / n cancel ";
    let max_inner = usize::from(anchor.area.width.saturating_sub(4));
    let inner_width = width(prompt).max(width(keys)).clamp(24, max_inner.max(24));
    let rect = place(anchor, inner_width, 2);
    let inner = frame_box(frame, rect, "Delete message?", view);
    let row_width = usize::from(inner.width);
    let lines = vec![
        Line::from(Span::styled(
            truncate(prompt, row_width).into_owned(),
            Style::new().fg(view.theme.text),
        )),
        Line::from(Span::styled(
            truncate(keys, row_width).into_owned(),
            view.theme.muted(),
        )),
    ];
    frame.render_widget(Paragraph::new(lines), inner);
}
