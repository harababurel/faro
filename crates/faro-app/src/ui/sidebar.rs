//! Left sidebar: the conversation list (DESIGN.md §7.1, §7.7).

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Paragraph};

use faro_core::config::{Avatars, TimestampFormat};
use faro_core::domain::{ConversationId, Direction, Message, Timestamp};
use faro_core::store::{ConversationSummary, NotifyLevel};

use super::text::{self, sanitize, truncate};
use super::{View, messages};
use crate::app::state::{Focus, Geometry, Section, SidebarRow, SidebarView, State};

pub fn render(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &State,
    view: &View<'_>,
    geometry: &mut Geometry,
) {
    let theme = view.theme;
    let focused = state.focus == Focus::Conversations;
    let title_style = if focused {
        Style::new().fg(theme.primary).add_modifier(Modifier::BOLD)
    } else {
        theme.muted()
    };
    let title = match (state.list.view, state.list.allowlist) {
        (SidebarView::Chats, false) => " Conversations ",
        (SidebarView::Chats, true) => " Conversations · allowlist ",
        (SidebarView::Archive, _) => " Archive ",
    };
    // Lit while the pointer is on its edge or dragging it (D156).
    let border = if state.resize.hover || state.resize.dragging.is_some() {
        Style::new().fg(theme.accent)
    } else {
        theme.border(focused)
    };
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(border)
        .title(Span::styled(title, title_style));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    geometry.sidebar = Some(inner);

    if state.loading {
        frame.render_widget(
            Paragraph::new(Span::styled(" Loading…", theme.muted())),
            inner,
        );
        return;
    }
    let height = usize::from(inner.height);
    // Screen rows: a conversation takes two with previews (D154), so the
    // offset, clicks and the wheel all count lines, not entries.
    let mut lines = Vec::new();
    let mut kinds = Vec::new();
    let mut selected = None;
    // Where each two-row entry starts, for its picture.
    let mut tall = Vec::new();
    for row in rows(state, view, usize::from(inner.width)) {
        let first = lines.len();
        if row.selected {
            selected = Some((first, first + row.lines.len().saturating_sub(1)));
        }
        if let (SidebarRow::Conversation(id), 2) = (row.kind, row.lines.len()) {
            tall.push((first, id));
        }
        kinds.extend(std::iter::repeat_n(row.kind, row.lines.len()));
        lines.extend(row.lines);
    }
    let offset = visible_offset(
        state.sidebar.offset,
        lines.len(),
        height,
        selected,
        state.sidebar.reveal,
    );
    geometry.sidebar_offset = offset;
    geometry
        .sidebar_rows
        .extend(kinds.into_iter().skip(offset).take(height));
    let shown: Vec<Line<'static>> = lines.into_iter().skip(offset).take(height).collect();
    frame.render_widget(Paragraph::new(shown), inner);
    pictures(frame, inner, state, view, geometry, &tall, offset);
}

/// A picture's box: the badge's four cells, one in from the edge, two rows.
const PICTURE: Rect = Rect::new(1, 0, 4, 2);

/// Draw profile pictures over the badges of two-row entries (`avatars =
/// "pictures"` or `"auto"`): only with a graphics protocol (half-blocks are too coarse at
/// four cells) and while no popup or palette is open (D78). Initials stay
/// until a picture is decoded, and wherever there's none.
fn pictures(
    frame: &mut Frame<'_>,
    inner: Rect,
    state: &State,
    view: &View<'_>,
    geometry: &mut Geometry,
    tall: &[(usize, ConversationId)],
    offset: usize,
) {
    use crate::app::image_cache::{CachedImage, ImageKey, ImageShape};

    let overlay = state.popup.is_some() || state.palette.is_some();
    let protocol = match view.images.active_protocol() {
        Some(protocol) if protocol.is_graphics() => protocol,
        Some(_) | None => return,
    };
    let wanted = match state.config.conversations.avatars {
        Avatars::Pictures | Avatars::Auto => true,
        Avatars::Off | Avatars::Initials => false,
    };
    if !wanted || overlay {
        return;
    }
    for &(first, id) in tall {
        let Some(path) = state.pictures.files.get(&id) else {
            continue;
        };
        let (Ok(first), Ok(offset)) = (i32::try_from(first), i32::try_from(offset)) else {
            continue;
        };
        let y = first - offset;
        if y + i32::from(PICTURE.height) <= 0 || y >= i32::from(inner.height) {
            continue;
        }
        let Ok(y) = i16::try_from(y) else {
            continue;
        };
        let key = ImageKey {
            path: path.clone(),
            width: PICTURE.width,
            height: PICTURE.height,
            protocol,
            shape: ImageShape::Avatar,
        };
        match view.images.get(&key) {
            Some(CachedImage::Ready(picture)) => {
                clear_badge(frame.buffer_mut(), inner, y);
                let x = i16::try_from(PICTURE.x).unwrap_or(0);
                picture.draw(x, y, inner, frame.buffer_mut(), state.colors());
            }
            Some(CachedImage::Failed) => {}
            None => {
                if !geometry.images_wanted.contains(&key) {
                    geometry.images_wanted.push(key);
                }
            }
        }
    }
}

/// Blank the badge under a picture in the row's own colours (taken from the
/// cell before it), so a round picture's corners show the row, not the
/// badge.
fn clear_badge(buf: &mut ratatui::buffer::Buffer, inner: Rect, y: i16) {
    for dy in 0..PICTURE.height {
        let Ok(row) = u16::try_from(i32::from(y) + i32::from(dy)) else {
            continue;
        };
        if row >= inner.height {
            continue;
        }
        let row = inner.y + row;
        let style = buf
            .cell((inner.x, row))
            .map(ratatui::buffer::Cell::style)
            .unwrap_or_default();
        for dx in 0..PICTURE.width {
            if let Some(cell) = buf.cell_mut((inner.x + PICTURE.x + dx, row)) {
                cell.set_symbol(" ").set_style(style);
            }
        }
    }
}

/// The first row to show: the requested offset, clamped so the list doesn't
/// scroll past its end, and moved just enough to show the selection (its
/// first and last rows) when asked; its first row if both don't fit.
fn visible_offset(
    requested: usize,
    rows: usize,
    height: usize,
    selected: Option<(usize, usize)>,
    reveal: bool,
) -> usize {
    let mut offset = requested.min(rows.saturating_sub(height));
    if reveal && let Some((first, last)) = selected {
        if first < offset {
            offset = first;
        } else if last >= offset + height {
            offset = (last + 1).saturating_sub(height).min(first);
        }
    }
    offset
}

struct Row {
    lines: Vec<Line<'static>>,
    kind: SidebarRow,
    selected: bool,
}

impl Row {
    fn label(text: impl Into<String>, style: Style) -> Self {
        Self::one(
            Line::from(Span::styled(text.into(), style)),
            SidebarRow::Label,
        )
    }

    fn one(line: Line<'static>, kind: SidebarRow) -> Self {
        Self {
            lines: vec![line],
            kind,
            selected: false,
        }
    }
}

/// The current view's rows (D41). Chats: pinned and recent (headed only when
/// something is pinned), then a row into the archive. Archive: a row back,
/// then archived, hidden and not-allowed conversations under headers.
fn rows(state: &State, view: &View<'_>, width: usize) -> Vec<Row> {
    let highlighted = state.highlighted();
    let theme = view.theme;
    let listed: Vec<&ConversationSummary> = state.visible_conversations().collect();
    let archive = state.list.view == SidebarView::Archive;
    let headed = archive || listed.iter().any(|c| state.section(c) == Section::Pinned);
    let mut rows = Vec::with_capacity(listed.len() + 6);
    if archive {
        rows.push(Row::one(
            Line::from(Span::styled(" ◂ Chats", Style::new().fg(theme.accent))),
            SidebarRow::SwitchView,
        ));
    }
    let mut current = None;
    for summary in &listed {
        let section = state.section(summary);
        if headed && current != Some(section) {
            current = Some(section);
            rows.push(Row::label(section_label(section), theme.muted()));
        }
        rows.push(row(state, view, summary, highlighted, width));
    }
    if listed.is_empty() {
        let note = match (archive, state.list.allowlist) {
            (true, _) => " Nothing archived or hidden",
            (false, true) => " Allowlist is empty",
            (false, false) => " No conversations yet",
        };
        rows.push(Row::label(note, theme.muted()));
    }
    let archive_items: Vec<&ConversationSummary> = state.listed_in(SidebarView::Archive).collect();
    let elsewhere = archive_items.len();
    let requests = archive_items
        .iter()
        .filter(|c| state.section(c) == Section::Request)
        .count();
    if !archive && elsewhere > 0 {
        let label = if requests == 0 {
            " ▸ Archive"
        } else if requests == elsewhere {
            " ▸ Requests"
        } else {
            " ▸ Requests & archive"
        };
        rows.push(Row::label("", theme.muted()));
        rows.push(Row::one(
            Line::from(vec![
                Span::styled(label, Style::new().fg(theme.accent)),
                Span::styled(format!(" ({elsewhere})"), theme.muted()),
            ]),
            SidebarRow::SwitchView,
        ));
    }
    rows
}

fn section_label(section: Section) -> &'static str {
    match section {
        Section::Pinned => " ▾ Pinned",
        Section::Recent => " ▾ Recent",
        Section::Request => " ▾ Message requests",
        Section::Archived => " ▾ Archived",
        Section::Hidden => " ▾ Hidden until a message",
        Section::NotAllowed => " ▾ Not on the allowlist",
        Section::Blocked => " ▾ Blocked",
    }
}

/// Marks a group, beside the unread dot (D96). Direct chats get blanks, so
/// every name starts in the same column.
fn kind_glyph(id: ConversationId) -> &'static str {
    match id {
        ConversationId::Group(_) => "# ",
        ConversationId::Direct(_) => "  ",
    }
}

/// The right-edge marker for a chat that won't notify as usual (D96, D176):
/// blocked or muted (`⊘`), message request (`?`), or mentions only (`@`).
fn notify_glyph(summary: &ConversationSummary, view: &View<'_>) -> Option<&'static str> {
    if summary.state.blocked || summary.state.muted_at(view.now_timestamp()) {
        return Some("⊘");
    }
    if summary.state.request {
        return Some("?");
    }
    match summary.state.notify {
        NotifyLevel::Mentions => Some("@"),
        NotifyLevel::All => None,
    }
}

fn row(
    state: &State,
    view: &View<'_>,
    summary: &ConversationSummary,
    highlighted: Option<ConversationId>,
    width: usize,
) -> Row {
    let theme = view.theme;
    let selected = highlighted == Some(summary.id);
    // With nothing open the highlight is only a cursor (D60): no open style.
    let open = selected && state.open.is_some();
    let unread = summary.unread > 0;
    let typing = !state.typists(summary.id).is_empty();
    let quiet = notify_glyph(summary, view);
    // Muted or mentions only: drawn quietly, as it won't notify (D49).
    let muted = quiet.is_some();
    let settings = state.config.conversations;

    let marker = if unread {
        let style = if muted {
            theme.muted()
        } else {
            Style::new().fg(theme.unread)
        };
        Span::styled(" ● ", style)
    } else {
        Span::raw("   ")
    };
    let kind = kind_glyph(summary.id);
    let count = if unread {
        format!(" {}", summary.unread)
    } else {
        String::new()
    };
    // The notification marker sits at the edge, after the count, so markers
    // line up whatever the counts; its room is kept before the name is cut.
    let quiet = quiet.map(|glyph| format!(" {glyph}")).unwrap_or_default();
    let end = if count.is_empty() && quiet.is_empty() {
        ""
    } else {
        " "
    };
    // With previews, typing shows on the second row instead (D154).
    let typing_suffix = typing && !(settings.previews && settings.preview_activity);
    let suffix = if typing_suffix { " …" } else { "" };

    let mut name_style = if muted {
        theme.muted()
    } else {
        Style::new().fg(theme.text)
    };
    if unread && !muted {
        name_style = name_style.add_modifier(Modifier::BOLD);
    }
    if open {
        name_style = name_style
            .fg(theme.primary)
            .remove_modifier(Modifier::DIM)
            .add_modifier(Modifier::BOLD);
    }
    let count_style = if muted {
        theme.muted()
    } else {
        Style::new().fg(theme.unread).add_modifier(Modifier::BOLD)
    };
    let title = state.conversation_title(summary.id);
    // `auto` (D162): a badge only where pictures can be drawn (initials
    // until each arrives, and for chats without one); the dot elsewhere.
    let badges = match settings.avatars {
        Avatars::Off => false,
        Avatars::Initials | Avatars::Pictures => true,
        Avatars::Auto => settings.previews && view.images.is_graphics_protocol(),
    };
    let badge = badges.then(|| Badge {
        initials: initials(&sanitize(&title)),
        colour: theme.avatar(summary.id),
    });
    let parts = Parts {
        marker,
        badge,
        kind,
        title,
        name_style,
        suffix,
        count,
        count_style,
        quiet,
        end,
        bright: unread && !muted,
    };
    let mut lines = if settings.previews {
        two_rows(state, view, summary, parts, width)
    } else {
        vec![one_row(parts, view, width)]
    };
    if selected {
        lines = lines
            .into_iter()
            .map(|line| line.style(theme.selected()))
            .collect();
    }
    Row {
        lines,
        kind: SidebarRow::Conversation(summary.id),
        selected,
    }
}

/// Before a preview, under the first letter of the name (D154).
const BRANCH: &str = "╰─ ";

/// What both forms of a conversation's entry are made of.
struct Parts {
    /// The unread dot (or blanks), when there's no badge.
    marker: Span<'static>,
    /// The initials badge, in place of the dot (D158).
    badge: Option<Badge>,
    kind: &'static str,
    title: String,
    name_style: Style,
    /// ` …` while someone types (unless the preview says so).
    suffix: &'static str,
    count: String,
    count_style: Style,
    /// The mute or mentions-only marker.
    quiet: String,
    end: &'static str,
    /// Unread and not muted: the time and preview are drawn brighter.
    bright: bool,
}

/// An initials avatar (D158).
struct Badge {
    /// Two cells' worth.
    initials: String,
    colour: Color,
}

/// Which row of an entry [`lead`] draws.
#[derive(Clone, Copy)]
enum Lead {
    /// The only row (no previews).
    Only,
    /// The name's row, with previews.
    Top,
    /// The preview's row.
    Bottom,
}

/// Before the group marker: the unread dot in 3 cells, or a badge in 6. A
/// badge is a pill on a row of its own (`▐AB▌`) and a rounded square over
/// two (`▟AB▙` over `▜██▛`), in the conversation's colour. With a badge the
/// dot goes (the count at the edge and the bold name still say unread).
fn lead(parts: &Parts, row: Lead, view: &View<'_>) -> (Vec<Span<'static>>, usize) {
    let Some(badge) = &parts.badge else {
        return match row {
            Lead::Only | Lead::Top => (vec![parts.marker.clone()], 3),
            Lead::Bottom => (vec![Span::raw("   ")], 3),
        };
    };
    let edge = Style::new().fg(badge.colour);
    // Dark on the colour; the terminal's own background has no colour value.
    let ink = if view.theme.background == Color::Reset {
        Color::Black
    } else {
        view.theme.background
    };
    let letters = Span::styled(
        badge.initials.clone(),
        Style::new()
            .bg(badge.colour)
            .fg(ink)
            .add_modifier(Modifier::BOLD),
    );
    let spans = match row {
        Lead::Only => vec![Span::styled("▐", edge), letters, Span::styled("▌", edge)],
        Lead::Top => vec![Span::styled("▟", edge), letters, Span::styled("▙", edge)],
        Lead::Bottom => vec![Span::styled("▜██▛", edge)],
    };
    let mut lead = vec![Span::raw(" ")];
    lead.extend(spans);
    lead.push(Span::raw(" "));
    (lead, 6)
}

/// Up to two cells of initials for `title` (D158): the first letters of its
/// first and last words (`Alice Smith` → `AS`), or the first two of a single
/// word (`Bob` → `Bo`); digits if there are no letters (a phone number), `?`
/// if there's nothing. Always exactly two cells, padded on the right.
fn initials(title: &str) -> String {
    use unicode_segmentation::UnicodeSegmentation as _;

    let alphanumeric = |g: &&str| g.chars().next().is_some_and(char::is_alphanumeric);
    let firsts: Vec<&str> = title
        .split_whitespace()
        .filter_map(|word| word.graphemes(true).find(alphanumeric))
        .collect();
    let picked: String = match firsts.as_slice() {
        [] => "?".to_owned(),
        [only] => {
            let word = title
                .split_whitespace()
                .find(|word| word.graphemes(true).any(|g| alphanumeric(&g)))
                .unwrap_or(only);
            let mut letters = word.graphemes(true).filter(alphanumeric);
            let first = letters.next().unwrap_or(only).to_uppercase();
            let second = letters.next().map(str::to_lowercase).unwrap_or_default();
            first + &second
        }
        [first, .., last] => first.to_uppercase() + &last.to_uppercase(),
    };
    // Two cells at most (one wide character fills both), then padded.
    let mut out = String::new();
    for g in picked.graphemes(true) {
        if text::width(&out) + text::width(g) > 2 {
            break;
        }
        out.push_str(g);
    }
    let pad = 2_usize.saturating_sub(text::width(&out));
    out.push_str(&" ".repeat(pad));
    out
}

/// The entry without previews: everything on the name's row.
fn one_row(parts: Parts, view: &View<'_>, width: usize) -> Line<'static> {
    let theme = view.theme;
    let (lead, lead_width) = lead(&parts, Lead::Only, view);
    let fixed = lead_width
        + text::width(parts.kind)
        + text::width(&parts.count)
        + text::width(&parts.quiet)
        + parts.end.len();
    let room = width.saturating_sub(fixed + text::width(parts.suffix));
    let name = truncate(&sanitize(&parts.title), room).into_owned();
    let pad = room.saturating_sub(text::width(&name));
    let mut spans = lead;
    spans.extend([
        Span::styled(parts.kind, theme.muted()),
        Span::styled(name, parts.name_style),
        Span::styled(parts.suffix, Style::new().fg(theme.accent)),
        Span::raw(" ".repeat(pad)),
        Span::styled(parts.count, parts.count_style),
        Span::styled(parts.quiet, theme.muted()),
        Span::raw(parts.end),
    ]);
    Line::from(spans)
}

/// The entry with previews (D154): the name and the time, then the preview
/// under the name with the unread count and marker at the edge.
fn two_rows(
    state: &State,
    view: &View<'_>,
    summary: &ConversationSummary,
    parts: Parts,
    width: usize,
) -> Vec<Line<'static>> {
    let theme = view.theme;
    let time = summary
        .last_activity
        .filter(|_| state.config.conversations.preview_time)
        .map(|at| format!(" {} ", list_time(state, at, view)))
        .unwrap_or_default();
    let (top, lead_width) = lead(&parts, Lead::Top, view);
    let (bottom, _) = lead(&parts, Lead::Bottom, view);
    let fixed =
        lead_width + text::width(parts.kind) + text::width(parts.suffix) + text::width(&time);
    let room = width.saturating_sub(fixed);
    let name = truncate(&sanitize(&parts.title), room).into_owned();
    let pad = room.saturating_sub(text::width(&name));
    let time_style = if parts.bright {
        Style::new().fg(theme.unread)
    } else {
        theme.muted()
    };
    let mut first = top;
    first.extend([
        Span::styled(parts.kind, theme.muted()),
        Span::styled(name, parts.name_style),
        Span::styled(parts.suffix, Style::new().fg(theme.accent)),
        Span::raw(" ".repeat(pad)),
        Span::styled(time, time_style),
    ]);
    let first = Line::from(first);

    // The preview hangs off the name like a tree branch, so it can't be
    // mistaken for a name; one blank cell always separates it from the count.
    let indent = " ".repeat(text::width(parts.kind));
    let fixed = lead_width
        + indent.len()
        + text::width(BRANCH)
        + text::width(&parts.count)
        + text::width(&parts.quiet)
        + parts.end.len()
        + 1;
    let room = width.saturating_sub(fixed);
    let mut spans = bottom;
    spans.extend([Span::raw(indent), Span::styled(BRANCH, theme.muted())]);
    let used = preview_spans(state, summary, parts.bright, view, room, &mut spans);
    spans.push(Span::raw(" ".repeat(room.saturating_sub(used) + 1)));
    spans.push(Span::styled(parts.count, parts.count_style));
    spans.push(Span::styled(parts.quiet, theme.muted()));
    spans.push(Span::raw(parts.end));
    vec![first, Line::from(spans)]
}

/// The newest message's time for the list (D154): `10:42` today, `Mon`
/// within the week, `3 Oct` this year, `Oct 2025` before; `5m`/`3h`/`2d`
/// when message timestamps are relative.
fn list_time(state: &State, at: Timestamp, view: &View<'_>) -> String {
    if state.timestamp_format() == TimestampFormat::Relative {
        return messages::format_relative_time(at, view.now_timestamp());
    }
    let Some(at) = messages::to_jiff(at) else {
        return String::new();
    };
    let when = messages::zoned(at, view);
    let (date, today) = (when.date(), messages::zoned(view.now, view).date());
    let days = today.since(date).ok().map(|span| span.get_days());
    let format = if date == today {
        "%H:%M"
    } else if matches!(days, Some(1..=6)) {
        "%a"
    } else if date.year() == today.year() {
        "%-d %b"
    } else {
        "%b %Y"
    };
    when.strftime(format).to_string()
}

/// Push the second row's text (D154), cut to `room` cells; returns the
/// cells used. Typing beats a draft, which beats the newest message (as in
/// Signal's apps); the open conversation's draft is in the input box, so it
/// isn't repeated here. Nothing here marks anything read (D42).
fn preview_spans(
    state: &State,
    summary: &ConversationSummary,
    bright: bool,
    view: &View<'_>,
    room: usize,
    spans: &mut Vec<Span<'static>>,
) -> usize {
    let theme = view.theme;
    let settings = state.config.conversations;
    let is_open = state.open.as_ref().is_some_and(|o| o.id == summary.id);
    let mut push = |label: &str, text: &str, label_style: Style, style: Style| {
        let label = truncate(label, room).into_owned();
        let rest = room.saturating_sub(text::width(&label));
        let text = truncate(&text::one_line(text), rest).into_owned();
        let used = text::width(&label) + text::width(&text);
        spans.push(Span::styled(label, label_style));
        spans.push(Span::styled(text, style));
        used
    };
    let typists = state.typists(summary.id);
    if settings.preview_activity && !typists.is_empty() {
        let text = match (summary.id, typists) {
            (ConversationId::Group(_), [one]) => {
                format!("{} is typing…", state.person_name(one.who))
            }
            (ConversationId::Group(_), many) => format!("{} people are typing…", many.len()),
            (ConversationId::Direct(_), _) => "typing…".to_owned(),
        };
        let style = Style::new().fg(theme.accent).add_modifier(Modifier::ITALIC);
        return push("", &text, style, style);
    }
    if settings.preview_activity
        && !is_open
        && let Some(draft) = &summary.state.draft
    {
        return push(
            "Draft: ",
            draft.expose(),
            Style::new().fg(theme.warning),
            theme.muted(),
        );
    }
    let Some(message) = &summary.last_message else {
        return 0;
    };
    let style = if bright {
        Style::new().fg(theme.text)
    } else {
        theme.muted()
    };
    let who = if message.is_system() {
        String::new()
    } else if matches!(message.direction, Direction::Outgoing(_)) || message.sender == state.me {
        "You: ".to_owned()
    } else if matches!(summary.id, ConversationId::Group(_)) {
        format!("{}: ", state.person_name(message.sender))
    } else {
        String::new()
    };
    if message.deleted {
        return push(
            &who,
            "Message deleted",
            style,
            style.add_modifier(Modifier::ITALIC),
        );
    }
    push(&who, &message_preview(state, message), style, style)
}

/// One line for a message: its text (mentions as names) after a 📎 if it
/// carries a file, or what the file is when there's no text.
fn message_preview(state: &State, message: &Message) -> String {
    if let Some(event) = &message.system_event {
        return state.system_event_text(message, event);
    }
    let attachment = message.attachments.first();
    if let Some(voice) = attachment.filter(|a| a.is_voice_note()) {
        return match voice.duration_secs {
            Some(secs) => format!("▶ Voice message {}:{:02}", secs / 60, secs % 60),
            None => "▶ Voice message".to_owned(),
        };
    }
    let body = message
        .body
        .as_ref()
        .and_then(|_| state.readable_body(message))
        .filter(|text| !text.trim().is_empty());
    match (attachment, body) {
        (Some(_), Some(text)) => format!("📎 {text}"),
        (None, Some(text)) => text,
        (Some(file), None) => {
            let name = file
                .file_name
                .clone()
                .unwrap_or_else(|| if file.is_image() { "Photo" } else { "File" }.to_owned());
            format!("📎 {name}")
        }
        (None, None) => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::{initials, visible_offset};

    #[test]
    fn initials_take_first_and_last_words_and_always_fill_two_cells() {
        assert_eq!(initials("Alice Smith"), "AS");
        assert_eq!(initials("Mary Anne van Dyke"), "MD");
        assert_eq!(initials("Bob"), "Bo");
        assert_eq!(initials("Climbing 🧗"), "Cl", "emoji aren't letters");
        assert_eq!(initials("élodie roux"), "ÉR");
        assert_eq!(initials("+44 7700 900123"), "49", "a number");
        assert_eq!(initials("日本語"), "日", "one wide character fills both");
        assert_eq!(initials("X"), "X ");
        assert_eq!(initials("🧗 🧗"), "? ");
        assert_eq!(initials(""), "? ");
    }

    #[test]
    fn offset_is_clamped_and_reveals_the_selection() {
        // 20 rows, 5 visible.
        assert_eq!(visible_offset(0, 20, 5, Some((2, 2)), true), 0);
        assert_eq!(visible_offset(0, 20, 5, Some((9, 9)), true), 5);
        assert_eq!(visible_offset(8, 20, 5, Some((2, 2)), true), 2);
        // Scrolled away with the wheel: the selection may be off screen.
        assert_eq!(visible_offset(8, 20, 5, Some((2, 2)), false), 8);
        // Both rows of a two-row entry come into view…
        assert_eq!(visible_offset(0, 20, 5, Some((4, 5)), true), 1);
        // …or at least its first, when only one row fits.
        assert_eq!(visible_offset(0, 20, 1, Some((4, 5)), true), 4);
        // Never past the end.
        assert_eq!(visible_offset(99, 20, 5, None, false), 15);
        assert_eq!(visible_offset(3, 4, 5, None, false), 0);
    }
}
