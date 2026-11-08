//! Popups (D40, D41): the message action menu and the reaction picker over
//! the message area; the conversation menu and the mute picker next to the
//! sidebar. Drawn last, over everything else.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Paragraph};

use super::View;
use super::messages::{format_time as format_hhmm, reactions, suffix, thread_body, to_jiff, zoned};
use super::shortcodes::{REACT_SEARCH_LIMIT, REACT_SEARCH_ROWS, search_shortcodes};
use super::text::{one_line, pad_left, sanitize, truncate, width};
use faro_core::domain::{
    ConversationId, DeliveryStatus, Direction, MessageRef, ServiceId, Timestamp, VerificationState,
    format_byte_size, format_disappearing_remaining, format_disappearing_timer,
    format_safety_number,
};
use faro_core::store::RecipientReceipt;

use crate::app::state::{
    DISAPPEARING_CHOICES, Geometry, MUTE_CHOICES, MessageAction, Popup, REMOTE_CONSENT_CHOICES,
    SidebarRow, State, same_emoji,
};

/// Shortcut keys of `MUTE_CHOICES`.
const MUTE_KEYS: [&str; 5] = ["1", "2", "3", "4", "5"];
/// Shortcut keys of `DISAPPEARING_CHOICES`.
const DISAPPEARING_KEYS: [&str; 7] = ["1", "2", "3", "4", "5", "6", "7"];

#[expect(
    clippy::too_many_lines,
    reason = "flat dispatch table over every Popup variant"
)]
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
        Popup::DisappearingTimer { target, selected } => {
            let anchor =
                beside_sidebar(frame.area(), geometry, *target).unwrap_or_else(|| Anchor {
                    area: geometry.history.unwrap_or_else(|| frame.area()),
                    around: None,
                });
            let labels: Vec<(String, &str)> = DISAPPEARING_CHOICES
                .iter()
                .zip(DISAPPEARING_KEYS)
                .map(|((label, _), key)| ((*label).to_owned(), key))
                .collect();
            menu(
                frame,
                anchor,
                "Disappearing messages",
                &labels,
                *selected,
                view,
            );
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
        Popup::ConfirmUnlinkDevice { name, .. } => {
            confirm_unlink_device(frame, name, geometry, view);
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
        Popup::SafetyNumber(peer) => safety_number(frame, state, *peer, view),
        Popup::Thread {
            messages, selected, ..
        } => thread_viewer(frame, state, messages, *selected, geometry, view),
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
    if let Some(event) = &message.system_event {
        rows.push((
            "Event",
            sanitize(&state.system_event_text(message, event)).into_owned(),
            text_style,
        ));
    }
    if let Some(edited_at) = message.edited_at {
        rows.push(("Edited", format_datetime(edited_at, view), text_style));
    }
    if let Some(value) = disappears_value(message, view) {
        rows.push(("Disappears", value, text_style));
    }
    if !message.is_system()
        && let Direction::Outgoing(status) = message.direction
    {
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
        push_attachment_rows(&mut rows, att, text_style, theme.secondary);
    }
    rows
}

fn push_attachment_rows(
    rows: &mut Vec<(&'static str, String, Style)>,
    att: &faro_core::domain::AttachmentRef,
    text_style: Style,
    secondary: ratatui::style::Color,
) {
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
            Style::new().fg(secondary),
        ));
    }
}

fn disappears_value(message: &faro_core::domain::Message, view: &View<'_>) -> Option<String> {
    let secs = message.expires_in_secs?;
    Some(match message.expires_at() {
        Some(expires_at) => format!(
            "in {} ({})",
            format_disappearing_remaining(expires_at, view.now_timestamp()),
            format_datetime(expires_at, view),
        ),
        None => format!("{} after read", format_disappearing_timer(secs)),
    })
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
    ];
    if let Some(release) = state.update_available() {
        rows.push((
            format!("faro {} is available", release.version),
            Style::new().fg(theme.accent),
        ));
    }
    rows.extend([
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
    ]);
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

fn confirm_unlink_device(frame: &mut Frame<'_>, name: &str, geometry: &Geometry, view: &View<'_>) {
    let clean = sanitize(name);
    let prompt = format!(" Unlink \"{clean}\" from your Signal account? ");
    let keys = " Enter / y confirm · Esc / n cancel ";
    let area = geometry.history.unwrap_or_else(|| frame.area());
    let max_inner = usize::from(area.width.saturating_sub(4));
    let inner_width = width(&prompt).max(width(keys)).clamp(24, max_inner.max(24));
    let box_w = u16::try_from(inner_width + 2)
        .unwrap_or(u16::MAX)
        .min(area.width);
    let box_h = 4u16.min(area.height);
    let rect = Rect {
        x: area.x + area.width.saturating_sub(box_w) / 2,
        y: area.y + area.height.saturating_sub(box_h) / 2,
        width: box_w,
        height: box_h,
    };
    let inner = frame_box(frame, rect, "Unlink device?", view);
    let row_width = usize::from(inner.width);
    let lines = vec![
        Line::from(Span::styled(
            truncate(&prompt, row_width).into_owned(),
            Style::new().fg(view.theme.text),
        )),
        Line::from(Span::styled(
            truncate(keys, row_width).into_owned(),
            view.theme.muted(),
        )),
    ];
    frame.render_widget(Paragraph::new(lines), inner);
}

fn safety_number(frame: &mut Frame<'_>, state: &State, peer: ServiceId, view: &View<'_>) {
    let theme = view.theme;
    let name = sanitize(&state.person_name(peer)).into_owned();
    let record = state.identities.get(&peer);
    let key_changed = record.is_some_and(|r| r.key_changed);
    let verified = record.map_or(VerificationState::Default, |r| r.verified);
    let (status_text, status_style) = if key_changed {
        (
            "⚠ Safety number changed — verify or approve before sending",
            Style::new().fg(theme.warning).add_modifier(Modifier::BOLD),
        )
    } else {
        match verified {
            VerificationState::Verified => (
                "✓ Marked as verified",
                Style::new().fg(theme.success).add_modifier(Modifier::BOLD),
            ),
            VerificationState::Unverified => (
                "⚠ Marked as unverified",
                Style::new().fg(theme.warning).add_modifier(Modifier::BOLD),
            ),
            VerificationState::Default => ("Not verified yet", theme.muted()),
        }
    };
    let mut rows: Vec<(String, Style)> = vec![
        (
            format!(" {name}"),
            Style::new().fg(theme.primary).add_modifier(Modifier::BOLD),
        ),
        (format!(" {status_text}"), status_style),
        (String::new(), theme.muted()),
    ];
    if let Some(sn) = record.and_then(|r| r.safety_number.as_deref()) {
        for line in format_safety_number(sn) {
            rows.push((
                format!("   {line}   "),
                Style::new().fg(theme.text).add_modifier(Modifier::BOLD),
            ));
        }
    } else {
        rows.push((" No safety number available yet".to_owned(), theme.muted()));
    }
    rows.push((String::new(), theme.muted()));
    let keys = if key_changed {
        " v verify · a approve new key · y copy · Esc close "
    } else if verified == VerificationState::Verified {
        " v clear · u mark unverified · y copy · Esc close "
    } else {
        " v mark verified · u mark unverified · y copy · Esc close "
    };
    rows.push((keys.to_owned(), theme.muted()));

    let area = frame.area();
    let wanted_width = rows.iter().map(|(t, _)| width(t)).max().unwrap_or(32) + 2;
    let inner_width = u16::try_from(wanted_width)
        .unwrap_or(u16::MAX)
        .min(area.width.saturating_sub(4));
    let inner_height = u16::try_from(rows.len())
        .unwrap_or(u16::MAX)
        .min(area.height.saturating_sub(2));
    let rect = Rect {
        x: area.x + area.width.saturating_sub(inner_width + 2) / 2,
        y: area.y + area.height.saturating_sub(inner_height + 2) / 2,
        width: (inner_width + 2).min(area.width),
        height: (inner_height + 2).min(area.height),
    };
    let inner = frame_box(frame, rect, "Safety number", view);
    let room = usize::from(inner.width);
    let lines: Vec<Line<'static>> = rows
        .into_iter()
        .map(|(text, style)| Line::from(Span::styled(truncate(&text, room).into_owned(), style)))
        .collect();
    frame.render_widget(Paragraph::new(lines), inner);
}

/// `T` (or `t` on a text message): the chronological quote thread lens (D166).
fn thread_viewer(
    frame: &mut Frame<'_>,
    state: &State,
    messages: &[faro_core::domain::Message],
    selected: usize,
    geometry: &Geometry,
    view: &View<'_>,
) {
    let theme = view.theme;
    let area = geometry.history.unwrap_or_else(|| frame.area());
    let box_w = area.width.saturating_sub(4).clamp(24, 78).min(area.width);
    let inner_w = usize::from(box_w.saturating_sub(2)).max(10);
    let content_w = inner_w.saturating_sub(4).max(6);

    let mut all_lines: Vec<Line<'static>> = Vec::new();
    if let Some(first_quote) = messages.first().and_then(|m| m.quote.as_ref())
        && !messages.iter().any(|m| m.reference() == first_quote.target)
    {
        let who = sanitize(&state.person_name(first_quote.target.sender)).into_owned();
        let preview = first_quote
            .preview
            .as_ref()
            .map_or_else(|| "…".to_owned(), |p| one_line(p.expose()));
        let stub = format!(" ▎ {who}: {preview} (original message not in history)");
        all_lines.push(Line::from(Span::styled(
            truncate(&stub, inner_w).into_owned(),
            theme.muted().add_modifier(Modifier::ITALIC),
        )));
        all_lines.push(Line::from(Span::styled(" │", theme.muted())));
    }

    let clamped = selected.min(messages.len().saturating_sub(1));
    let mut sel_range = (0usize, 0usize);
    for (i, message) in messages.iter().enumerate() {
        if i > 0 {
            all_lines.push(Line::from(Span::styled(" │", theme.muted())));
        }
        let start = if i == 0 { 0 } else { all_lines.len() };
        let prev_ref = i
            .checked_sub(1)
            .and_then(|p| messages.get(p))
            .map(faro_core::domain::Message::reference);
        all_lines.extend(thread_message_lines(
            state,
            message,
            prev_ref,
            i == clamped,
            inner_w,
            content_w,
            view,
        ));
        if i == clamped {
            sel_range = (start, all_lines.len());
        }
    }

    let max_inner_h = usize::from(area.height.saturating_sub(4)).max(3);
    let inner_h = all_lines.len().clamp(3, max_inner_h);
    let top = if all_lines.len() <= inner_h {
        0
    } else {
        sel_range
            .1
            .saturating_sub(inner_h)
            .min(sel_range.0)
            .min(all_lines.len() - inner_h)
    };
    let count = messages.len();
    let noun = if count == 1 { "message" } else { "messages" };
    let title = if all_lines.len() > inner_h {
        format!("Thread · {}/{} {noun}", clamped + 1, count)
    } else {
        format!("Thread · {count} {noun}")
    };
    let box_h = u16::try_from(inner_h + 2)
        .unwrap_or(u16::MAX)
        .min(area.height);
    let rect = Rect {
        x: area.x + area.width.saturating_sub(box_w) / 2,
        y: area.y + area.height.saturating_sub(box_h) / 2,
        width: box_w,
        height: box_h,
    };
    let inner = frame_box(frame, rect, &title, view);
    let visible: Vec<Line<'static>> = all_lines.into_iter().skip(top).take(inner_h).collect();
    frame.render_widget(Paragraph::new(visible), inner);
}

fn thread_message_lines(
    state: &State,
    message: &faro_core::domain::Message,
    prev_ref: Option<MessageRef>,
    chosen: bool,
    inner_w: usize,
    content_w: usize,
    view: &View<'_>,
) -> Vec<Line<'static>> {
    let theme = view.theme;
    let row_bg = if chosen {
        theme.selected()
    } else {
        Style::new()
    };
    let spine = if chosen {
        Span::styled(" ▌ ", row_bg.fg(theme.primary).add_modifier(Modifier::BOLD))
    } else {
        Span::styled(" │ ", theme.muted())
    };
    let finish_row = |spans: Vec<Span<'static>>| -> Line<'static> {
        let mut line = Vec::with_capacity(spans.len() + 2);
        line.push(spine.clone());
        let mut used = 3usize;
        for mut span in spans {
            used += width(&span.content);
            span.style = row_bg.patch(span.style);
            line.push(span);
        }
        if chosen && used < inner_w {
            line.push(Span::styled(" ".repeat(inner_w - used), row_bg));
        }
        Line::from(line)
    };

    let sender = sanitize(&state.sender_name(message)).into_owned();
    let sender_style = if message.direction == Direction::Incoming {
        theme.sender(message.sender).add_modifier(Modifier::BOLD)
    } else {
        Style::new().fg(theme.primary).add_modifier(Modifier::BOLD)
    };
    let mut header = vec![
        Span::styled(
            truncate(&sender, content_w.saturating_sub(10)).into_owned(),
            sender_style,
        ),
        Span::styled(
            format!(" · {}", format_hhmm(message.sent_at, view)),
            theme.muted(),
        ),
    ];
    header.extend(suffix(state, message, view));
    let mut rows = vec![finish_row(header)];

    if prev_ref.is_some()
        && let Some(quote) = &message.quote
        && Some(quote.target) != prev_ref
    {
        let who = sanitize(&state.person_name(quote.target.sender)).into_owned();
        let preview = quote
            .preview
            .as_ref()
            .map_or_else(|| "…".to_owned(), |p| one_line(p.expose()));
        let text = truncate(&format!("↳ {who}: {preview}"), content_w).into_owned();
        rows.push(finish_row(vec![Span::styled(
            text,
            theme.muted().add_modifier(Modifier::ITALIC),
        )]));
    }

    for body_line in thread_body(state, message, content_w, view) {
        rows.push(finish_row(body_line));
    }
    if !message.reactions.is_empty() {
        let react_line = truncate(&reactions(message), content_w).into_owned();
        rows.push(finish_row(vec![Span::styled(react_line, theme.muted())]));
    }
    rows
}
