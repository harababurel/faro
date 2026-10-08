//! Emoji that terminals measure differently (D47).
//!
//! faro measures text per grapheme (`unicode-width`), as kitty and recent tmux
//! do. Terminals that add up the widths of the code points instead (mosh, older
//! tmux and VTE-style emulators) disagree on emoji with a variation selector
//! (❤️), skin tones (👍🏽), keycaps (1️⃣) and joined sequences (🧗‍♀️, families):
//! everything after one on the same line lands in the wrong cells.
//!
//! In [`EmojiMode::Compatible`], [`display`] (used by `text::sanitize`) draws
//! those graphemes as their parts without the joiners and modifiers (❤️ → ❤,
//! 👨‍👩‍👧‍👦 → 👨👩👧👦), whose width every terminal agrees on. [`finish`] then goes
//! over the drawn frame: anything that still slipped through (text that wasn't
//! sanitised, like the message input) gets a stand-in of the same width, so no
//! such grapheme ever reaches the terminal. Half a flag (a lone regional
//! indicator) becomes its letter in every mode: kitty draws it two cells wide,
//! tmux one.
//!
//! [`finish`] also repairs wide characters cut by something drawn over them (a
//! popup's edge over the right half of an emoji), in every mode: ratatui never
//! draws a cell hidden behind a wide character, so the popup's edge would be
//! missing there.
//!
//! tmux joins what it receives onto the cell before the cursor, wherever the
//! cursor came from (D55): a regional indicator onto a cell that starts with
//! one (so a flag drawn right after another flag becomes one cell holding
//! both, and a redraw shifts the rest of the row), anything onto a cell that
//! ends with a zero-width joiner. So in every mode: two flags in a row get a
//! space between them ([`display`]) or the second becomes its letters
//! ([`finish`]), and graphemes that start with a joiner or modifier, or end
//! with a joiner, lose them. So do skin tones on a base tmux doesn't join them
//! to (🫶🏽, 🏃🏽: tmux 3.6 has a fixed list of bases), which tmux draws two
//! cells wider. Emoji tmux always draws two cells wide ([`TMUX_WIDE`]) get
//! VS16, which makes them two cells wide everywhere else too. In compatible
//! mode those become a near equivalent (✌ → ☮), and flags become their letters
//! (🇷🇴 → RO): mosh keeps the two halves of a flag in separate cells and can
//! redraw one without the other.

use std::borrow::Cow;
use std::cell::Cell as ModeCell;

use std::num::NonZeroU16;

use ratatui::buffer::{Buffer, CellDiffOption};
use unicode_segmentation::UnicodeSegmentation as _;
use unicode_width::{UnicodeWidthChar as _, UnicodeWidthStr as _};

use crate::app::state::EmojiMode;

thread_local! {
    /// The mode of the frame being drawn; `ui::render` sets it from the state.
    /// Thread-local so parallel tests can't see each other's mode.
    static MODE: ModeCell<EmojiMode> = const { ModeCell::new(EmojiMode::Full) };
}

#[cfg(test)]
thread_local! {
    /// How many stand-ins the last [`finish`] on this thread placed.
    static STAND_INS: ModeCell<usize> = const { ModeCell::new(0) };
}

/// How many stand-ins the last frame drawn on this thread needed: text that
/// skipped `text::sanitize`.
#[cfg(test)]
pub fn last_stand_ins() -> usize {
    STAND_INS.with(ModeCell::get)
}

/// Whether every terminal draws `symbol` (a cell's grapheme) as faro measures
/// it, in the current mode.
#[cfg(test)]
pub fn drawable(symbol: &str) -> bool {
    !needs_rewrite(symbol, mode())
}

pub fn set_mode(mode: EmojiMode) {
    MODE.with(|m| m.set(mode));
}

fn mode() -> EmojiMode {
    MODE.with(ModeCell::get)
}

/// `text` as it should be drawn in the current mode.
pub fn display(text: &str) -> Cow<'_, str> {
    if text.is_ascii() {
        return Cow::Borrowed(text);
    }
    let mode = mode();
    let mut previous: Option<&str> = None;
    let clean = text.graphemes(true).all(|g| {
        let ok = !needs_rewrite(g, mode) && !previous.is_some_and(|p| joins(p, g));
        previous = Some(g);
        ok
    });
    if clean {
        return Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len() + 8);
    // The last grapheme written, to keep tmux from joining the next onto it.
    let mut last = String::new();
    for g in text.graphemes(true) {
        let drawn: Cow<'_, str> = if needs_rewrite(g, mode) {
            Cow::Owned(rewrite(g, mode))
        } else {
            Cow::Borrowed(g)
        };
        for part in drawn.graphemes(true) {
            if joins(&last, part) {
                out.push(' ');
            }
            out.push_str(part);
            part.clone_into(&mut last);
        }
    }
    Cow::Owned(out)
}

/// Make the drawn frame safe for the terminal: stand-ins for graphemes the
/// current mode doesn't draw (see the module docs), then wide characters cut
/// by an overlay are blanked. Returns how many stand-ins were needed.
pub fn finish(buffer: &mut Buffer) -> usize {
    let mode = mode();
    let area = buffer.area;
    let mut replaced = 0;
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            let Some(cell) = buffer.cell((x, y)) else {
                continue;
            };
            if !needs_rewrite(cell.symbol(), mode) {
                continue;
            }
            let cells = cell.symbol().width().max(1);
            let parts = stand_in(cell.symbol(), cells, mode);
            let style = cell.style();
            let mut at = x;
            for part in parts {
                let width = u16::try_from(part.width().max(1)).unwrap_or(1);
                if let Some(cell) = buffer.cell_mut((at, y)) {
                    cell.set_symbol(&part).set_style(style);
                }
                for k in 1..width {
                    if let Some(hidden) = at.checked_add(k).and_then(|x| buffer.cell_mut((x, y))) {
                        hidden.reset();
                    }
                }
                at = at.saturating_add(width);
            }
            replaced += 1;
        }
    }
    replaced += separate_joining_cells(buffer);
    repair_cut_wide(buffer);
    pin_wide_cells(buffer);
    #[cfg(test)]
    STAND_INS.with(|s| s.set(replaced));
    replaced
}

/// Whether `g` is drawn differently in `mode`.
fn needs_rewrite(g: &str, mode: EmojiMode) -> bool {
    lone_indicator(g).is_some()
        || loose_decoration(g)
        || unjoined_skin_tone(g)
        || match mode {
            EmojiMode::Full => missing_presentation(g),
            EmojiMode::Compatible => {
                !width_agrees(g) || flag_letters(g).is_some() || g.chars().any(is_tmux_wide)
            }
        }
}

/// Emoji tmux (3.6) always draws two cells wide (its default
/// `codepoint-widths`) although on their own, without VS16, they're one cell
/// wide everywhere else (☝ ✌ ✍ 🖐…). Each with what compatible mode draws
/// instead: mosh measures them one cell wide whatever follows, so no form of
/// them is safe there.
const TMUX_WIDE: [(char, &str); 9] = [
    ('\u{261D}', "👆"),
    ('\u{26F9}', "🏀"),
    ('\u{270C}', "☮"),
    ('\u{270D}', "📝"),
    ('\u{1F3CB}', "💪"),
    ('\u{1F3CC}', "⛳"),
    ('\u{1F574}', "🕺"),
    ('\u{1F575}', "🔍"),
    ('\u{1F590}', "✋"),
];

fn is_tmux_wide(c: char) -> bool {
    TMUX_WIDE.iter().any(|(wide, _)| *wide == c)
}

/// An emoji tmux draws two cells wide not followed by VS16, which makes it
/// two cells wide for faro and other terminals too.
fn missing_presentation(g: &str) -> bool {
    let mut chars = g.chars().peekable();
    while let Some(c) = chars.next() {
        if is_tmux_wide(c) && chars.peek() != Some(&'\u{FE0F}') {
            return true;
        }
    }
    false
}

/// `g` with VS16 after each emoji tmux draws two cells wide (replacing a
/// text presentation selector).
fn with_presentation(g: &str) -> String {
    let mut out = String::with_capacity(g.len() + 3);
    let mut chars = g.chars().peekable();
    while let Some(c) = chars.next() {
        out.push(c);
        if is_tmux_wide(c) {
            if chars.peek() == Some(&'\u{FE0E}') {
                chars.next();
            }
            if chars.peek() != Some(&'\u{FE0F}') {
                out.push('\u{FE0F}');
            }
        }
    }
    out
}

/// What compatible mode draws for `c` (see [`TMUX_WIDE`]).
fn compatible_char(c: char) -> Cow<'static, str> {
    TMUX_WIDE.iter().find(|(wide, _)| *wide == c).map_or_else(
        || Cow::Owned(c.to_string()),
        |(_, instead)| Cow::Borrowed(*instead),
    )
}

/// Terminals that add up code points measure `g` as faro does.
fn width_agrees(g: &str) -> bool {
    g.width() == g.chars().map(|c| c.width().unwrap_or(0)).sum::<usize>()
}

fn is_indicator(c: char) -> bool {
    ('\u{1F1E6}'..='\u{1F1FF}').contains(&c)
}

/// The letter of a regional indicator.
fn indicator_letter(c: char) -> Option<char> {
    is_indicator(c)
        .then(|| char::from_u32(u32::from('A') + (u32::from(c) - 0x1F1E6)))
        .flatten()
}

/// The letter of a lone regional indicator (half a flag).
fn lone_indicator(g: &str) -> Option<char> {
    let mut chars = g.chars();
    let c = chars.next()?;
    if chars.next().is_some() {
        return None;
    }
    indicator_letter(c)
}

/// A flag's two letters (🇷🇴 → RO), as wide as the flag.
fn flag_letters(g: &str) -> Option<String> {
    let mut chars = g.chars();
    let (a, b) = (chars.next()?, chars.next()?);
    if chars.next().is_some() {
        return None;
    }
    Some(
        [indicator_letter(a)?, indicator_letter(b)?]
            .into_iter()
            .collect(),
    )
}

/// A grapheme that starts with a joiner or modifier (nothing to attach to)
/// or ends with a joiner: tmux would join it onto its neighbours.
fn loose_decoration(g: &str) -> bool {
    g.chars().next().is_some_and(is_decoration)
        || (g.chars().count() > 1 && g.ends_with('\u{200D}'))
}

fn is_skin_tone(c: char) -> bool {
    ('\u{1F3FB}'..='\u{1F3FF}').contains(&c)
}

/// The emoji tmux (3.6) joins a skin tone onto (`utf8_should_combine`); after
/// any other emoji it draws the skin tone as an emoji of its own, two cells
/// wider than faro measures (🫶🏽, 🏃🏽, ☝🏽).
const TMUX_SKIN_TONE_BASES: &[u32] = &[
    0x1F44B, 0x1F44C, 0x1F44D, 0x1F44E, 0x1F44F, 0x1F450, 0x1F466, 0x1F467, 0x1F468, 0x1F469,
    0x1F46E, 0x1F470, 0x1F471, 0x1F472, 0x1F473, 0x1F474, 0x1F475, 0x1F476, 0x1F477, 0x1F478,
    0x1F47C, 0x1F481, 0x1F482, 0x1F483, 0x1F485, 0x1F486, 0x1F487, 0x1F4AA, 0x1F575, 0x1F57A,
    0x1F590, 0x1F595, 0x1F596, 0x1F645, 0x1F646, 0x1F647, 0x1F64B, 0x1F64C, 0x1F64D, 0x1F64E,
    0x1F64F, 0x1F6B4, 0x1F6B5, 0x1F6B6, 0x1F926, 0x1F937, 0x1F938, 0x1F939, 0x1F93D, 0x1F93E,
    0x1F9B5, 0x1F9B6, 0x1F9B8, 0x1F9B9, 0x1F9CD, 0x1F9CE, 0x1F9CF, 0x1F9D1, 0x1F9D2, 0x1F9D3,
    0x1F9D4, 0x1F9D5, 0x1F9D6, 0x1F9D7, 0x1F9D8, 0x1F9D9, 0x1F9DA, 0x1F9DB, 0x1F9DC, 0x1F9DD,
    0x1F9DE, 0x1F9DF,
];

/// A skin tone after an emoji tmux doesn't join it to.
fn unjoined_skin_tone(g: &str) -> bool {
    let mut previous: Option<char> = None;
    g.chars().any(|c| {
        let unjoined = is_skin_tone(c)
            && previous.is_some_and(|p| TMUX_SKIN_TONE_BASES.binary_search(&u32::from(p)).is_err());
        previous = Some(c);
        unjoined
    })
}

/// `g` without the skin tones tmux wouldn't join.
fn without_unjoined_skin_tones(g: &str) -> String {
    let mut out = String::with_capacity(g.len());
    let mut previous: Option<char> = None;
    for c in g.chars() {
        let unjoined = is_skin_tone(c)
            && previous.is_some_and(|p| TMUX_SKIN_TONE_BASES.binary_search(&u32::from(p)).is_err());
        if !unjoined {
            out.push(c);
        }
        previous = Some(c);
    }
    out
}

/// Whether tmux would join `right`, drawn just after `left`, onto `left`'s
/// cell: a regional indicator after a cell that starts with one (tmux checks
/// only the first code point, so a whole flag counts).
fn joins(left: &str, right: &str) -> bool {
    left.chars().next().is_some_and(is_indicator) && right.chars().next().is_some_and(is_indicator)
}

/// Joiners and modifiers that make a grapheme's width terminal-dependent:
/// variation selector 16, zero-width joiner, keycap, skin tones, tags.
fn is_decoration(c: char) -> bool {
    matches!(
        c,
        '\u{FE0F}' | '\u{200D}' | '\u{20E3}' | '\u{1F3FB}'..='\u{1F3FF}' | '\u{E0020}'..='\u{E007F}'
    )
}

/// `g` as graphemes every terminal measures alike and tmux keeps apart.
fn rewrite(g: &str, mode: EmojiMode) -> String {
    if let Some(letter) = lone_indicator(g) {
        return letter.to_string();
    }
    if mode == EmojiMode::Compatible
        && let Some(letters) = flag_letters(g)
    {
        return letters;
    }
    if mode == EmojiMode::Full {
        // Only skin tones tmux won't join, loose joiners or modifiers at the
        // ends, or a missing VS16: fix just those.
        let kept = without_unjoined_skin_tones(g);
        let trimmed = kept
            .trim_start_matches(is_decoration)
            .trim_end_matches('\u{200D}');
        if !trimmed.is_empty() && !loose_decoration(trimmed) {
            return with_presentation(trimmed);
        }
    }
    let stripped: String = g
        .chars()
        .filter(|c| !is_decoration(*c))
        .map(|c| match mode {
            EmojiMode::Compatible => compatible_char(c),
            EmojiMode::Full => Cow::Owned(c.to_string()),
        })
        .collect();
    let safe: String = stripped
        .graphemes(true)
        .map(|part| {
            if width_agrees(part) && lone_indicator(part).is_none() {
                Cow::Borrowed(part)
            } else {
                // Shouldn't happen once decorations are gone; keep the base.
                Cow::Owned(part.chars().take(1).collect())
            }
        })
        .collect();
    if safe.is_empty() {
        "\u{FFFD}".to_owned()
    } else if mode == EmojiMode::Full {
        with_presentation(&safe)
    } else {
        safe
    }
}

/// `g`'s rewrite cut or padded to exactly `cells` cells, as one string per cell
/// that starts a grapheme.
fn stand_in(g: &str, cells: usize, mode: EmojiMode) -> Vec<String> {
    let mut parts = Vec::new();
    let mut used = 0;
    for part in rewrite(g, mode).graphemes(true) {
        let width = part.width();
        if width == 0 {
            continue;
        }
        if used + width > cells {
            break;
        }
        used += width;
        parts.push(part.to_owned());
    }
    parts.extend(std::iter::repeat_n(" ".to_owned(), cells - used));
    parts
}

/// Cells tmux would join onto the cell before them (see [`joins`]): a flag
/// right after another flag becomes its letters, which are as wide. Text
/// that went through `display` never needs this; the input and symbols put
/// next to each other by different widgets can. Returns how many changed.
fn separate_joining_cells(buffer: &mut Buffer) -> usize {
    let area = buffer.area;
    let mut changed = 0;
    for y in area.top()..area.bottom() {
        // The cell that covers the column before `x`.
        let mut left: Option<u16> = None;
        let mut x = area.left();
        while x < area.right() {
            let width = buffer
                .cell((x, y))
                .map_or(1, |cell| u16::try_from(cell.symbol().width()).unwrap_or(1));
            let joined = left
                .and_then(|l| buffer.cell((l, y)))
                .zip(buffer.cell((x, y)))
                .is_some_and(|(l, cell)| joins(l.symbol(), cell.symbol()));
            if joined
                && let Some(cell) = buffer.cell_mut((x, y))
                && let Some(letters) = flag_letters(cell.symbol())
            {
                let style = cell.style();
                cell.set_symbol(&letters[..1]);
                if let Some(next) = x.checked_add(1).and_then(|x| buffer.cell_mut((x, y))) {
                    next.set_symbol(&letters[1..]).set_style(style);
                }
                changed += 1;
            }
            left = Some(x);
            x = x.saturating_add(width.max(1));
        }
    }
    changed
}

/// Blank wide characters whose hidden cells something else was drawn into.
/// Every overlay is bordered, so its edge over a hidden cell is a visible
/// symbol; a hidden cell that's merely recoloured (a row tint, a base style
/// patched over the area) is still intact.
fn repair_cut_wide(buffer: &mut Buffer) {
    let area = buffer.area;
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            let Some(cell) = buffer.cell((x, y)) else {
                continue;
            };
            let width = u16::try_from(cell.symbol().width()).unwrap_or(1);
            if width < 2 {
                continue;
            }
            let cut = (1..width).any(|k| {
                x.checked_add(k)
                    .and_then(|x| buffer.cell((x, y)))
                    .is_none_or(|hidden| hidden.symbol() != " ")
            });
            if cut && let Some(cell) = buffer.cell_mut((x, y)) {
                cell.set_symbol(" ");
            }
        }
    }
}

/// Make ratatui skip the hidden cells of every wide character when it sends
/// the frame. Left alone, ratatui 0.30 re-sends the hidden cell after an emoji
/// with VS16 (❤️) without moving the cursor there, but the terminal's cursor is
/// already past it, so the rest of the run lands one cell to the right: borders
/// and counts shift, and the row spills into the next one.
fn pin_wide_cells(buffer: &mut Buffer) {
    for cell in &mut buffer.content {
        let width = u16::try_from(cell.symbol().width()).unwrap_or(1);
        if width > 1
            && let Some(width) = NonZeroU16::new(width)
        {
            cell.diff_option = CellDiffOption::ForcedWidth(width);
        }
    }
}

#[cfg(test)]
mod tests {
    use ratatui::layout::Rect;
    use ratatui::style::{Color, Style};

    use super::*;

    fn compatible<T>(f: impl FnOnce() -> T) -> T {
        set_mode(EmojiMode::Compatible);
        let result = f();
        set_mode(EmojiMode::Full);
        result
    }

    #[test]
    fn full_mode_keeps_emoji_but_not_half_flags() {
        let text = "❤️ 👍🏽 🧗‍♀️ 🇩🇪 1️⃣";
        assert_eq!(display(text), text);
        assert_eq!(display("a 🇩 b"), "a D b");
        assert!(matches!(display("plain ascii"), Cow::Borrowed(_)));
    }

    #[test]
    fn compatible_mode_drops_joiners_and_modifiers() {
        compatible(|| {
            assert_eq!(display("❤️"), "❤");
            // ✌ is two cells in tmux and one under mosh: something else.
            assert_eq!(display("☺️ ✌️"), "☺ ☮");
            assert_eq!(display("👍🏽"), "👍");
            assert_eq!(display("1️⃣"), "1");
            assert_eq!(display("👨‍👩‍👧‍👦"), "👨👩👧👦");
            assert_eq!(display("🧗‍♀️ up"), "🧗♀ up");
            assert_eq!(display("🏳️‍🌈"), "🏳🌈");
            // Already the same everywhere: left alone.
            assert_eq!(display("😂 陈伟 é"), "😂 陈伟 é");
            // Flags become their letters: mosh keeps a flag's halves apart.
            assert_eq!(display("🇩🇪"), "DE");
        });
    }

    #[test]
    fn every_rewritten_grapheme_is_measured_alike() {
        compatible(|| {
            for sample in [
                "❤️",
                "👍🏽",
                "1️⃣",
                "👨‍👩‍👧‍👦",
                "🚴‍♂️",
                "🏳️‍🌈",
                "🇩",
                "🏴‍☠️",
                "✌️",
                "☝🏽",
                "🖐",
                "⛹️‍♀️",
            ] {
                for g in display(sample).graphemes(true) {
                    assert!(width_agrees(g), "{sample:?} → {g:?}");
                    assert!(!needs_rewrite(g, EmojiMode::Compatible), "{g:?}");
                }
            }
        });
    }

    #[test]
    fn stand_ins_keep_the_width() {
        let c = EmojiMode::Compatible;
        assert_eq!(stand_in("❤️", 2, c), ["❤", " "]);
        assert_eq!(stand_in("👨‍👩‍👧‍👦", 2, c), ["👨"]);
        assert_eq!(stand_in("1️⃣", 2, c), ["1", " "]);
        assert_eq!(stand_in("🇩", 1, c), ["D"]);
        assert_eq!(stand_in("🇷🇴", 2, c), ["R", "O"]);
    }

    #[test]
    fn flags_next_to_each_other_are_kept_apart() {
        // tmux would join the second flag onto the first one's cell (D55).
        assert_eq!(display("🇷🇴🇩🇪🇯🇵 ok"), "🇷🇴 🇩🇪 🇯🇵 ok");
        assert_eq!(display("🇷🇴 🇩🇪"), "🇷🇴 🇩🇪");
        compatible(|| assert_eq!(display("🇷🇴🇩🇪 🇯🇵"), "RODE JP"));
    }

    #[test]
    fn skin_tones_tmux_cant_join_are_dropped() {
        // tmux 3.6 draws these two cells wider (D55).
        assert_eq!(display("🫶🏽 🏃🏽 ☝🏽"), "🫶 🏃 ☝\u{FE0F}");
        assert_eq!(display("👍🏽 👋🏻 🧑🏽‍💻"), "👍🏽 👋🏻 🧑🏽‍💻");
        assert!(TMUX_SKIN_TONE_BASES.is_sorted());
    }

    #[test]
    fn emoji_tmux_always_draws_wide_get_vs16() {
        // tmux 3.6 draws ☝ ✌ ✍ 🖐… two cells wide even without VS16 (D55).
        assert_eq!(display("✌ ok ☝🏽 ✍\u{FE0E}"), "✌️ ok ☝️ ✍️");
        assert_eq!(display("⛹\u{200D}♀️"), "⛹️\u{200D}♀️");
        let fine = "✌️ 🖐️ 🕵️‍♀️";
        assert_eq!(display(fine), fine);
        for (wide, instead) in TMUX_WIDE {
            let full = with_presentation(&wide.to_string());
            assert_eq!(full.width(), 2, "{wide:?}");
            assert!(!needs_rewrite(&full, EmojiMode::Full));
            assert!(width_agrees(instead), "{instead:?}");
            assert!(
                !needs_rewrite(instead, EmojiMode::Compatible),
                "{instead:?}"
            );
        }
        compatible(|| assert_eq!(display("✌️ ☝🏽 🖐"), "☮ 👆 ✋"));
    }

    #[test]
    fn loose_joiners_and_modifiers_are_dropped() {
        assert_eq!(display("a\u{200D} b"), "a b");
        assert_eq!(display("\u{1F3FD}x"), "\u{FFFD}x");
        assert_eq!(display("\u{FE0F}"), "\u{FFFD}");
        // Complete sequences stay.
        assert_eq!(display("🧗‍♀️ 👍🏽"), "🧗‍♀️ 👍🏽");
    }

    #[test]
    fn flags_drawn_side_by_side_become_letters() {
        // Two widgets (or the input, which isn't sanitised) put flags next
        // to each other: the second becomes its letters, as wide.
        let mut buffer = Buffer::empty(Rect::new(0, 0, 8, 1));
        buffer.set_string(0, 0, "🇷🇴", Style::new());
        buffer.set_string(2, 0, "🇩🇪", Style::new().bg(Color::Red));
        buffer.set_string(4, 0, " 🇯🇵", Style::new());
        assert_eq!(finish(&mut buffer), 1);
        // (`row` prints the hidden cells too.)
        assert_eq!(row(&buffer, 0), "🇷🇴 DE 🇯🇵  ");
        assert_eq!(buffer[(3, 0)].bg, Color::Red);
    }

    fn row(buffer: &Buffer, y: u16) -> String {
        (0..buffer.area.width)
            .map(|x| buffer[(x, y)].symbol().to_owned())
            .collect()
    }

    #[test]
    fn finish_replaces_what_slipped_through() {
        compatible(|| {
            let mut buffer = Buffer::empty(Rect::new(0, 0, 8, 1));
            buffer.set_string(0, 0, "a❤️b👨‍👩‍👧‍👦", Style::new());
            assert_eq!(finish(&mut buffer), 2);
            // Same cells as before: a, ❤ + pad, b, 👨 (+ hidden cell).
            assert_eq!(row(&buffer, 0), "a❤ b👨   ");
            for x in 0..8 {
                assert!(!needs_rewrite(
                    buffer[(x, 0)].symbol(),
                    EmojiMode::Compatible
                ));
            }
        });
    }

    #[test]
    fn wide_characters_cut_by_an_overlay_edge_are_blanked() {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 6, 3));
        // Row 0: an emoji whose right half gets a popup border.
        buffer.set_string(0, 0, "a😂bc", Style::new());
        buffer.set_string(2, 0, "│", Style::new());
        // Row 1: only recoloured over the right half (a base style patched
        // over an area): still intact.
        buffer.set_string(0, 1, "a😂bc", Style::new());
        buffer.set_style(Rect::new(2, 1, 4, 1), Style::new().bg(Color::Blue));
        // Row 2: a coloured bubble and a selection tint over the whole row are fine.
        buffer.set_string(0, 2, "a😂bc", Style::new().bg(Color::Red));
        buffer.set_style(Rect::new(0, 2, 6, 1), Style::new().bg(Color::Green));
        finish(&mut buffer);
        assert_eq!(buffer[(1, 0)].symbol(), " ");
        assert_eq!(buffer[(1, 1)].symbol(), "😂");
        assert_eq!(buffer[(1, 2)].symbol(), "😂");
    }

    #[test]
    fn hidden_cells_are_never_sent() {
        // The previous frame had text where the emoji's hidden cells are now.
        let mut prev = Buffer::empty(Rect::new(0, 0, 8, 1));
        prev.set_string(0, 0, "xxxxxxxx", Style::new());
        let mut next = Buffer::empty(Rect::new(0, 0, 8, 1));
        next.set_string(0, 0, "a❤️b😂c", Style::new());
        finish(&mut next);
        let sent: Vec<u16> = prev.diff(&next).iter().map(|(x, _, _)| *x).collect();
        // a, ❤️, b, 😂, c and the blank end: never the cells right after an
        // emoji (2 and 5).
        assert_eq!(sent, vec![0, 1, 3, 4, 6, 7]);
    }

    #[test]
    fn styled_emoji_on_an_untouched_row_stay() {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 4, 1));
        buffer.set_string(0, 0, "😂", Style::new().bg(Color::Red));
        finish(&mut buffer);
        assert_eq!(buffer[(0, 0)].symbol(), "😂");
    }
}
