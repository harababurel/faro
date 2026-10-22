//! The logo screen (D61): what the message pane shows while no chat is
//! open. The lighthouse from the naming phase (D12) beside or above the
//! wordmark, its beam turning; smaller forms when the pane is small or the
//! terminal's own colours are in use; a few key hints under each.

use std::time::Duration;

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use super::View;
use super::text::{sanitize, width};
use crate::app::keymap::Action;
use crate::app::state::{Focus, Geometry, SidebarView, State};
use crate::theme::Theme;

mod art;
#[cfg(test)]
mod tests;

use art::{Art, LIGHTHOUSE_HEIGHT, LIGHTHOUSE_WIDTH, Picture, SMALL_HEIGHT, SMALL_WIDTH};

/// Seconds into the animation the still picture shows (the prototype's
/// static frame: the beam pointing right, half lit).
pub const STILL: f64 = 0.35;
/// How often the logo is redrawn while it moves (10 fps).
pub const FRAME: Duration = Duration::from_millis(100);

pub const TAGLINE: &str = "unofficial Signal client for the terminal";
const WORD: &str = "faro";
/// Columns between the big lighthouse and the wordmark beside it.
const GUTTER: u16 = 4;
/// Columns between the small lighthouse and the word beside it.
const SMALL_GUTTER: u16 = 3;

/// The logo's forms, largest first (D61).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    /// The lighthouse with the wordmark (scale 2) and tagline beside it.
    Full,
    /// The lighthouse above the wordmark at this scale, then the tagline
    /// if it fits.
    Stacked(u16),
    /// The small lighthouse in the theme's colours, the word beside it
    /// (or under it in very narrow panes).
    Small,
    /// Just the word.
    Word,
}

const TIERS: [Tier; 5] = [
    Tier::Full,
    Tier::Stacked(2),
    Tier::Stacked(1),
    Tier::Small,
    Tier::Word,
];

impl Tier {
    /// Whether it's drawn in the logo's own colours (and so can move).
    const fn is_art(self) -> bool {
        match self {
            Self::Full | Self::Stacked(_) => true,
            Self::Small | Self::Word => false,
        }
    }

    /// Its size in a pane `room` columns wide, the tagline `tagline` wide.
    fn size(self, room: u16, tagline: u16) -> (u16, u16) {
        let fits = tagline <= room;
        match self {
            Self::Full => {
                let (word, _) = art::wordmark_size(2);
                (
                    LIGHTHOUSE_WIDTH + GUTTER + word.max(tagline),
                    LIGHTHOUSE_HEIGHT,
                )
            }
            Self::Stacked(scale) => {
                let (word_width, word_height) = art::wordmark_size(scale);
                // A blank line, the tagline and the version under it (D97).
                let tag = if fits { 3 } else { 0 };
                (
                    LIGHTHOUSE_WIDTH.max(word_width),
                    LIGHTHOUSE_HEIGHT + 1 + word_height + tag,
                )
            }
            Self::Small => {
                let word = width_of(WORD);
                if SMALL_WIDTH + SMALL_GUTTER + word <= room {
                    let text = if SMALL_WIDTH + SMALL_GUTTER + tagline <= room {
                        tagline
                    } else {
                        word
                    };
                    (SMALL_WIDTH + SMALL_GUTTER + text, SMALL_HEIGHT)
                } else {
                    (SMALL_WIDTH, SMALL_HEIGHT + 2)
                }
            }
            Self::Word => (width_of(WORD), 1),
        }
    }
}

/// A line under the logo.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Note {
    /// What's going on: loading, or nothing to open.
    Status(&'static str),
    /// A newer faro is out (D148).
    Update(String),
    /// A key and what it does.
    Hint(String, &'static str),
}

/// Hints dropped last when the pane is short: the status and two hints.
const MIN_NOTES: usize = 3;

/// The first status line in `faro demo` (D87).
const DEMO_STATUS: &str = "demo · fake contacts, nothing is sent";

/// Draw the logo screen in `area` (inside the pane's border).
pub fn render(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &State,
    view: &View<'_>,
    geometry: &mut Geometry,
) {
    let theme = view.theme;
    let art = Art::new(theme, state.colors());
    let tagline = width_of(TAGLINE);
    let mut notes = notes(state, geometry);
    let tier = choose(area, art.is_some(), tagline, &mut notes);
    let (w, h) = tier.size(area.width, tagline);
    let below = rows_below(notes.len());
    let top = area.y + area.height.saturating_sub(h + below) / 2;
    let block = Rect::new(centred(area, w), top, w, h).intersection(area);
    let t = view.logo_time.unwrap_or(STILL);
    let version = version_text();
    let buffer = frame.buffer_mut();
    match (tier, art) {
        (Tier::Full, Some(art)) => draw_full(buffer, area, block, art, t, theme, &version),
        (Tier::Stacked(scale), Some(art)) => {
            draw_stacked(buffer, area, block, (art, scale, t), theme, &version);
        }
        (Tier::Full | Tier::Stacked(_), None) | (Tier::Small, _) => {
            draw_small(buffer, area, block, theme, &version);
        }
        (Tier::Word, _) => draw_text(buffer, area, centred(area, w), top, WORD, brand(theme)),
    }
    draw_notes(buffer, area, top + h + 1, &notes, theme);
    geometry.logo_animated = view.logo_time.is_some() && tier.is_art() && art.is_some();
}

/// The largest form that fits with its notes, the last hint or two
/// dropped if that's what it takes; the word alone drops notes from the end
/// until it fits.
fn choose(area: Rect, art: bool, tagline: u16, notes: &mut Vec<Note>) -> Tier {
    let fewest = notes.len().min(MIN_NOTES);
    for tier in TIERS {
        if tier.is_art() && !art {
            continue;
        }
        let (w, h) = tier.size(area.width, tagline);
        if w > area.width {
            continue;
        }
        let fits = (fewest..=notes.len())
            .rev()
            .find(|&keep| h + rows_below(keep) <= area.height);
        if let Some(keep) = fits {
            notes.truncate(keep);
            return tier;
        }
    }
    while !notes.is_empty() && 1 + rows_below(notes.len()) > area.height {
        notes.pop();
    }
    Tier::Word
}

/// Rows the notes take under the logo, a blank line first.
fn rows_below(notes: usize) -> u16 {
    match notes {
        0 => 0,
        n => u16::try_from(n + 1).unwrap_or(u16::MAX),
    }
}

/// The lines under the logo: what's going on (in the demo, first that it
/// is one, D87), then key hints, most useful
/// first. With nothing open the list has the keys (D60): `alt+j/k` (from
/// anywhere) moves its highlight and `Enter` opens it.
fn notes(state: &State, geometry: &Geometry) -> Vec<Note> {
    let empty = state.listed_in(SidebarView::Chats).next().is_none()
        && state.listed_in(SidebarView::Archive).next().is_none();
    let mut notes = Vec::new();
    if state.is_demo() {
        notes.push(Note::Status(DEMO_STATUS));
    }
    if state.loading {
        notes.push(Note::Status("Loading…"));
    } else if empty {
        notes.push(Note::Status("No conversations yet"));
    }
    if let Some(release) = state.update_available() {
        notes.push(Note::Update(format!(
            "faro {} is available",
            release.version
        )));
    }
    let keymap = &state.keymap;
    let mut hint = |actions: &[Action], focus, label| {
        if let Some(key) = keymap.hint(actions, focus) {
            notes.push(Note::Hint(key, label));
        }
    };
    if !state.loading && !empty {
        let choose = [Action::NextConversation, Action::PreviousConversation];
        hint(&choose, Focus::Input, "choose a chat");
        if geometry.sidebar.is_some() {
            let open = if state.config.mouse.enabled {
                "open it, or click one"
            } else {
                "open it"
            };
            hint(&[Action::FocusInput], Focus::Conversations, open);
        }
    }
    hint(&[Action::OpenPalette], state.focus, "command palette");
    hint(&[Action::Help], state.focus, "keybindings");
    notes
}

/// The version line under the tagline (D97).
fn version_text() -> String {
    format!("v{}", crate::version::shown())
}

/// The version under the tagline at `y`, if it fits there.
fn draw_version(buffer: &mut Buffer, area: Rect, x: u16, y: u16, version: &str, theme: &Theme) {
    if x + width_of(version) <= area.right() {
        draw_text(buffer, area, x, y, version, theme.muted());
    }
}

fn draw_full(
    buffer: &mut Buffer,
    area: Rect,
    block: Rect,
    art: Art,
    t: f64,
    theme: &Theme,
    version: &str,
) {
    draw_picture(buffer, area, block.x, block.y, &art.lighthouse(t), theme);
    let (_, word_height) = art::wordmark_size(2);
    let column = word_height + 3;
    let x = block.x + LIGHTHOUSE_WIDTH + GUTTER;
    let y = block.y + LIGHTHOUSE_HEIGHT.saturating_sub(column) / 2;
    draw_picture(buffer, area, x, y, &art.wordmark(2), theme);
    draw_text(buffer, area, x, y + word_height + 1, TAGLINE, theme.muted());
    draw_version(buffer, area, x, y + word_height + 2, version, theme);
}

fn draw_stacked(
    buffer: &mut Buffer,
    area: Rect,
    block: Rect,
    (art, scale, t): (Art, u16, f64),
    theme: &Theme,
    version: &str,
) {
    let lighthouse = art.lighthouse(t);
    let x = centred(area, lighthouse.width);
    draw_picture(buffer, area, x, block.y, &lighthouse, theme);
    let word = art.wordmark(scale);
    let y = block.y + LIGHTHOUSE_HEIGHT + 1;
    draw_picture(buffer, area, centred(area, word.width), y, &word, theme);
    let tagline = width_of(TAGLINE);
    if tagline <= area.width {
        let y = y + word.height + 1;
        draw_text(
            buffer,
            area,
            centred(area, tagline),
            y,
            TAGLINE,
            theme.muted(),
        );
        let x = centred(area, width_of(version));
        draw_version(buffer, area, x, y + 1, version, theme);
    }
}

fn draw_small(buffer: &mut Buffer, area: Rect, block: Rect, theme: &Theme, version: &str) {
    let picture = art::small(theme);
    if block.width > SMALL_WIDTH {
        // Beside: the word level with the tower, the tagline under it.
        draw_picture(buffer, area, block.x, block.y, &picture, theme);
        let x = block.x + SMALL_WIDTH + SMALL_GUTTER;
        let y = block.y + SMALL_HEIGHT / 2 - 1;
        draw_text(buffer, area, x, y, WORD, brand(theme));
        if x + width_of(TAGLINE) <= area.right() {
            draw_text(buffer, area, x, y + 1, TAGLINE, theme.muted());
            draw_version(buffer, area, x, y + 2, version, theme);
        }
    } else {
        let x = centred(area, SMALL_WIDTH);
        draw_picture(buffer, area, x, block.y, &picture, theme);
        let y = block.y + SMALL_HEIGHT + 1;
        draw_text(
            buffer,
            area,
            centred(area, width_of(WORD)),
            y,
            WORD,
            brand(theme),
        );
    }
}

/// The status line centred, then the hints as a block centred under it,
/// keys right-aligned in a column.
fn draw_notes(buffer: &mut Buffer, area: Rect, top: u16, notes: &[Note], theme: &Theme) {
    let hints: Vec<(&str, &str)> = notes
        .iter()
        .filter_map(|note| match note {
            Note::Hint(key, label) => Some((key.as_str(), *label)),
            Note::Status(_) | Note::Update(_) => None,
        })
        .collect();
    let keys = hints.iter().map(|(k, _)| width_of(k)).max().unwrap_or(0);
    let labels = hints.iter().map(|(_, l)| width_of(l)).max().unwrap_or(0);
    let left = centred(area, keys + 2 + labels);
    for (y, note) in (top..).zip(notes) {
        match note {
            Note::Status(text) => {
                draw_text(
                    buffer,
                    area,
                    centred(area, width_of(text)),
                    y,
                    text,
                    theme.muted(),
                );
            }
            Note::Update(text) => {
                draw_text(
                    buffer,
                    area,
                    centred(area, width_of(text)),
                    y,
                    text,
                    Style::new().fg(theme.accent),
                );
            }
            Note::Hint(key, label) => {
                let pad = usize::from(keys - width_of(key));
                let line = Line::from(vec![
                    Span::styled(
                        format!("{}{key}", " ".repeat(pad)),
                        Style::new().fg(theme.text),
                    ),
                    Span::styled(format!("  {label}"), theme.muted()),
                ]);
                draw_line(buffer, area, left, y, &line);
            }
        }
    }
}

/// How the word is written outside the wordmark.
fn brand(theme: &Theme) -> Style {
    Style::new().fg(theme.primary).add_modifier(Modifier::BOLD)
}

/// The column that centres something `width` wide in `area`.
fn centred(area: Rect, width: u16) -> u16 {
    area.x + area.width.saturating_sub(width) / 2
}

/// `text`'s width in cells, as drawn (D47: measured after sanitising).
fn width_of(text: &str) -> u16 {
    u16::try_from(width(&sanitize(text))).unwrap_or(u16::MAX)
}

fn draw_text(buffer: &mut Buffer, area: Rect, x: u16, y: u16, text: &str, style: Style) {
    let line = Line::from(Span::styled(sanitize(text).into_owned(), style));
    draw_line(buffer, area, x, y, &line);
}

/// `line` at (`x`, `y`), cut at the edge of `area`.
fn draw_line(buffer: &mut Buffer, area: Rect, x: u16, y: u16, line: &Line<'_>) {
    if y < area.y || y >= area.bottom() || x >= area.right() {
        return;
    }
    buffer.set_line(x, y, line, area.right() - x);
}

/// `picture` with its top left corner at (`x`, `y`); what falls outside
/// `area` isn't drawn.
/// The small theme-coloured lighthouse (for the About popup, D94).
pub const SMALL_SIZE: (u16, u16) = (art::SMALL_WIDTH, art::SMALL_HEIGHT);

/// Draw the small lighthouse with its top-left corner at `(x, y)`, clipped to
/// `area`.
pub fn draw_small_lighthouse(buffer: &mut Buffer, area: Rect, x: u16, y: u16, theme: &Theme) {
    draw_picture(buffer, area, x, y, &art::small(theme), theme);
}

fn draw_picture(buffer: &mut Buffer, area: Rect, x: u16, y: u16, picture: &Picture, theme: &Theme) {
    for (dx, dy, glyph) in picture.glyphs() {
        let position = Position::new(x + dx, y + dy);
        if !area.contains(position) {
            continue;
        }
        if let Some(cell) = buffer.cell_mut(position) {
            cell.set_char(glyph.symbol)
                .set_fg(glyph.fg)
                .set_bg(glyph.bg.unwrap_or(theme.background));
        }
    }
}
