//! The command palette (D44): a box near the top, the query on the first
//! row, results below with matched characters highlighted and keys on the
//! right; settings under section headers, and a note about the level
//! (the colour depth at Themes, D57) above them. Drawn last, over
//! everything.

use faro_core::domain::ConversationId;
use ratatui::Frame;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Paragraph};
use unicode_segmentation::UnicodeSegmentation as _;

use super::View;
use super::text::{sanitize, truncate, width};
use crate::app::palette::{Item, ItemKind, Level, Match, Palette};
use crate::app::state::{Geometry, PaletteGeometry, State};

/// Widest the box gets.
const MAX_WIDTH: u16 = 76;
/// Shown on the query row at the root while nothing is typed.
const PREFIX_HINT: &str = "@ chats  : settings  > commands  ? keys";
/// Shown on the query row at Settings while nothing is typed.
const SETTINGS_HINT: &str = "←/→ cycle · enter list";
/// Shown on the query row below the root while nothing is typed.
const BACK_HINT: &str = "← back";
/// After a category's title.
const OPENS: &str = " ›";

pub fn render(frame: &mut Frame<'_>, state: &State, view: &View<'_>, geometry: &mut Geometry) {
    let Some(palette) = &state.palette else {
        return;
    };
    let screen = frame.area();
    // Leave the footer visible; give up on absurdly small terminals.
    let usable = screen.height.saturating_sub(1);
    if screen.width < 20 || usable < 5 {
        return;
    }
    let box_width = screen.width.saturating_sub(4).min(MAX_WIDTH);
    // Recompute the image note: config reloads and terminal checks can change
    // the active protocol while this level stays open.
    let image_note = (palette.level()
        == Level::Setting(crate::app::palette::Setting::InlineImages))
    .then(|| crate::app::palette::note(state, palette.level()))
    .flatten();
    let note = image_note.as_ref().or(palette.note.as_ref());
    // Borders, the query and the rule, and the note if there is one.
    let chrome = 4 + u16::from(note.is_some());
    let most_rows = (usable * 3 / 5).saturating_sub(chrome).max(1);
    let wanted = u16::try_from(palette.matches.len().max(1)).unwrap_or(u16::MAX);
    let rows = wanted.min(most_rows);
    let box_height = (rows + chrome).min(usable);
    let top = (usable / 6).min(usable - box_height);
    let area = Rect::new((screen.width - box_width) / 2, top, box_width, box_height);

    let theme = view.theme;
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(theme.border(true))
        .style(theme.panel())
        .title(Span::styled(
            format!(" {} ", sanitize(&palette.breadcrumb())),
            Style::new().fg(theme.primary).add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(area);
    frame.render_widget(Clear, area);
    frame.render_widget(block, area);
    if inner.height < 3 {
        return;
    }
    let query = Rect { height: 1, ..inner };
    draw_query(frame, query, palette, view);
    let rule = "─".repeat(usize::from(inner.width));
    frame.render_widget(
        Paragraph::new(Span::styled(rule, theme.muted())),
        Rect {
            y: inner.y + 1,
            height: 1,
            ..inner
        },
    );
    let mut top = inner.y + 2;
    if let Some(note) = note
        && inner.height > 3
    {
        let room = usize::from(inner.width).saturating_sub(2);
        let text = format!(" {}", truncate(&sanitize(note), room));
        frame.render_widget(
            Paragraph::new(Span::styled(text, theme.muted())),
            Rect {
                y: top,
                height: 1,
                ..inner
            },
        );
        top += 1;
    }
    let list = Rect {
        y: top,
        height: inner.bottom() - top,
        ..inner
    };
    let offset = visible_offset(palette, usize::from(list.height));
    draw_results(frame, list, palette, offset, view);
    geometry.palette = Some(PaletteGeometry { area, list, offset });
}

/// `palette.offset`, moved if needed so the selection is on screen.
fn visible_offset(palette: &Palette, rows: usize) -> usize {
    let rows = rows.max(1);
    let last_start = palette.matches.len().saturating_sub(rows);
    let offset = palette.offset.min(last_start);
    if palette.selected < offset {
        palette.selected
    } else if palette.selected >= offset + rows {
        palette.selected + 1 - rows
    } else {
        offset
    }
}

fn draw_query(frame: &mut Frame<'_>, area: Rect, palette: &Palette, view: &View<'_>) {
    let theme = view.theme;
    let text = sanitize(palette.input.text());
    let room = usize::from(area.width).saturating_sub(3);
    // Keep the end of a long query (where the cursor usually is) in view.
    let before_cursor = sanitize(
        palette
            .input
            .text()
            .get(..palette.input.cursor())
            .unwrap_or(""),
    );
    let skip = width(&before_cursor).saturating_sub(room);
    let shown: String = skip_cells(&text, skip);
    let mut spans = vec![
        Span::styled(" › ", Style::new().fg(theme.primary)),
        Span::styled(
            truncate(&shown, room).into_owned(),
            Style::new().fg(theme.text),
        ),
    ];
    let hint = match palette.level() {
        Level::Root => PREFIX_HINT,
        Level::Settings => SETTINGS_HINT,
        Level::Chats
        | Level::Search(_)
        | Level::Themes
        | Level::Setting(_)
        | Level::Commands
        | Level::Keys
        | Level::People
        | Level::Nickname(_)
        | Level::AttachFile
        | Level::ChatLinks
        | Level::ForwardTo(_) => BACK_HINT,
    };
    if text.is_empty() && usize::from(area.width) > width(hint) + 20 {
        let gap = usize::from(area.width) - 3 - width(hint) - 1;
        spans.push(Span::raw(" ".repeat(gap)));
        spans.push(Span::styled(hint, theme.muted()));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
    let cursor = width(&before_cursor).saturating_sub(skip);
    let x = area.x
        + 3
        + u16::try_from(cursor)
            .unwrap_or(0)
            .min(area.width.saturating_sub(4));
    frame.set_cursor_position(Position::new(x, area.y));
}

/// `text` without its first `cells` columns.
fn skip_cells(text: &str, cells: usize) -> String {
    let mut used = 0;
    text.graphemes(true)
        .skip_while(|g| {
            let skip = used < cells;
            used += width(g);
            skip
        })
        .collect()
}

fn draw_results(
    frame: &mut Frame<'_>,
    area: Rect,
    palette: &Palette,
    offset: usize,
    view: &View<'_>,
) {
    let theme = view.theme;
    if palette.matches.is_empty() {
        let text = if palette.level() == Level::Root {
            "  No matches (@ chats, : settings, > commands, ? keys)"
        } else {
            "  No matches"
        };
        frame.render_widget(Paragraph::new(Span::styled(text, theme.muted())), area);
        return;
    }
    let lines: Vec<Line<'static>> = palette
        .matches
        .iter()
        .enumerate()
        .skip(offset)
        .take(usize::from(area.height))
        .filter_map(|(i, found)| {
            let item = palette.items.get(found.item)?;
            Some(result_line(
                item,
                found,
                i == palette.selected,
                area.width,
                view,
            ))
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), area);
}

/// `▌ @ path › title ›        detail`
fn result_line(
    item: &Item,
    found: &Match,
    selected: bool,
    row_width: u16,
    view: &View<'_>,
) -> Line<'static> {
    let theme = view.theme;
    if item.kind == ItemKind::Header {
        return header_line(item, found, row_width, view);
    }
    let base = if selected {
        theme.selected().fg(theme.text)
    } else {
        Style::new().fg(theme.text)
    };
    let row_width = usize::from(row_width);
    let marker = if selected { "▌" } else { " " };
    let glyph = match (&item.kind, item.current) {
        (_, Some(true)) => "● ",
        (_, Some(false)) => "○ ",
        (
            ItemKind::Conversation(ConversationId::Group(_))
            | ItemKind::ForwardTo {
                destination: ConversationId::Group(_),
                ..
            },
            None,
        ) => "# ",
        // Direct chats: blank, as in the sidebar (D96).
        (
            ItemKind::Conversation(ConversationId::Direct(_))
            | ItemKind::ForwardTo {
                destination: ConversationId::Direct(_),
                ..
            }
            | ItemKind::Level(_)
            | ItemKind::EnterDir(_)
            | ItemKind::AttachFile(_)
            | ItemKind::SearchHit { .. }
            | ItemKind::ChatLink { .. }
            | ItemKind::ChatAttachment { .. }
            | ItemKind::Command(_)
            | ItemKind::Setting(_)
            | ItemKind::Theme(_)
            | ItemKind::Switch(_)
            | ItemKind::Header
            | ItemKind::Key
            | ItemKind::SetNickname(_)
            | ItemKind::ClearNickname(_),
            None,
        ) => "  ",
    };
    let opens = matches!(item.kind, ItemKind::Level(_) | ItemKind::EnterDir(_)) && !item.disabled;
    let suffix = if opens { width(OPENS) } else { 0 };
    let detail = sanitize(&item.detail).into_owned();
    let detail_width = if detail.is_empty() {
        0
    } else {
        width(&detail) + 2
    };
    let mut room = row_width.saturating_sub(1 + width(glyph) + detail_width + suffix);
    let mut used = 1 + width(glyph);
    let mut spans = vec![
        Span::styled(marker, base.fg(theme.primary)),
        Span::styled(glyph, base.patch(theme.muted())),
    ];
    if let Some(path) = &item.path {
        let path = truncate(&sanitize(&format!("{path} › ")), room / 2).into_owned();
        room = room.saturating_sub(width(&path));
        used += width(&path);
        spans.push(Span::styled(path, base.patch(theme.muted())));
    }
    let title_style = if selected {
        base.add_modifier(Modifier::BOLD)
    } else if item.disabled {
        base.patch(theme.muted())
    } else {
        base
    };
    let hit = base.fg(theme.primary).add_modifier(Modifier::BOLD);
    let (title_spans, title_width) =
        highlighted(&item.title, &found.highlight, room, title_style, hit);
    spans.extend(title_spans);
    used += title_width;
    if opens {
        spans.push(Span::styled(OPENS, base.patch(theme.muted())));
        used += suffix;
    }
    let gap = row_width.saturating_sub(used + detail_width);
    spans.push(Span::styled(" ".repeat(gap), base));
    if !detail.is_empty() {
        spans.push(Span::styled(
            format!(" {detail} "),
            base.patch(theme.muted()),
        ));
    }
    Line::from(spans)
}

/// A section's title (`Appearance`) above its items, which are indented
/// below it; never selected.
fn header_line(item: &Item, found: &Match, row_width: u16, view: &View<'_>) -> Line<'static> {
    let theme = view.theme;
    let style = theme.muted().add_modifier(Modifier::BOLD);
    let hit = Style::new().fg(theme.primary).add_modifier(Modifier::BOLD);
    let room = usize::from(row_width).saturating_sub(1);
    let (title, _) = highlighted(&item.title, &found.highlight, room, style, hit);
    let mut spans = vec![Span::raw(" ")];
    spans.extend(title);
    Line::from(spans)
}

/// `title` cut to `room` cells, the graphemes at `highlight` in `hit`; and
/// the cells used.
fn highlighted(
    title: &str,
    highlight: &[u32],
    room: usize,
    style: Style,
    hit: Style,
) -> (Vec<Span<'static>>, usize) {
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut run = String::new();
    let mut run_hit = false;
    let mut used = 0;
    let total = width(&sanitize(title));
    let budget = if total > room {
        room.saturating_sub(1)
    } else {
        room
    };
    for (i, grapheme) in title.graphemes(true).enumerate() {
        let grapheme = sanitize(grapheme);
        let w = width(&grapheme);
        if used + w > budget {
            break;
        }
        used += w;
        let is_hit = u32::try_from(i).is_ok_and(|i| highlight.binary_search(&i).is_ok());
        if is_hit != run_hit && !run.is_empty() {
            spans.push(Span::styled(
                std::mem::take(&mut run),
                if run_hit { hit } else { style },
            ));
        }
        run_hit = is_hit;
        run.push_str(&grapheme);
    }
    if !run.is_empty() {
        spans.push(Span::styled(run, if run_hit { hit } else { style }));
    }
    if total > room && room > 0 {
        spans.push(Span::styled("…", style));
        used += 1;
    }
    (spans, used)
}
