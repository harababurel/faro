//! Glyphs the renderer draws as shapes instead of taking them from a font
//! (D106): block elements, octants, sextants, the Unicode 16 quarter circles,
//! Powerline caps, box drawing, Braille, and the symbols the webfont
//! (`JetBrains Mono`) lacks (`↳ ✎ ❝`, D107). Fonts draw these slightly differently on every OS
//! (or not at all); as shapes they line up exactly, as in kitty.
//!
//! Pure geometry in cell units, so the Canvas painter (`paint.rs`) and a later
//! SVG exporter (WEBSITE.md §6.4) can share it.

/// How one cell's glyph is drawn when it isn't text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    /// A 2×4 grid of filled squares: bit `i` is column `i % 2`, row `i / 2`,
    /// rows top to bottom (octants, and the older half and quarter blocks).
    Octant(u8),
    /// A 2×3 grid, the same way (Legacy Computing sextants).
    Sextant(u8),
    /// One rectangle, in eighths of the cell: `(x0, y0, x1, y1)`.
    Eighths(u8, u8, u8, u8),
    /// The whole cell at 1, 2 or 3 quarters' opacity (`░ ▒ ▓`).
    Shade(u8),
    /// A filled quarter of a circle, centred on a corner of the cell, with a
    /// radius of half the cell's width (Unicode 16 bubble corners).
    QuarterCircle(Corner),
    Powerline(Powerline),
    /// Box drawing: the line going up, right, down and left from the centre.
    Lines([Line; 4]),
    /// A rounded box corner (`╭ ╮ ╰ ╯`), named by where it sits in a box: an
    /// arc joining the middles of two edges (see [`Shape::arc_ends`]).
    Arc(Corner),
    /// Braille dots: bit `i` is dot `i + 1` in Unicode's numbering.
    Braille(u8),
    Symbol(Symbol),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Corner {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Powerline {
    /// U+E0B0: a solid triangle pointing right.
    RightTriangle,
    /// U+E0B2: pointing left.
    LeftTriangle,
    /// U+E0B4: a solid half-ellipse bulging right.
    RightRound,
    /// U+E0B6: bulging left.
    LeftRound,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Line {
    None,
    Light,
    Heavy,
    Double,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Symbol {
    /// `↳` a line down, then right, with an arrowhead.
    ReturnArrow,
    /// `✎` a pencil, pointing down-left.
    Pencil,
    /// `❝` two heavy turned commas.
    Quote,
    /// `❞` two heavy commas: `❝` turned half a circle (D145).
    CloseQuote,
}

impl Shape {
    /// For [`Shape::Arc`]: which edges the arc joins, as `(horizontal, vertical)`
    /// directions from the centre (`1` right/down, `-1` left/up).
    pub const fn arc_ends(corner: Corner) -> (i8, i8) {
        match corner {
            // `╭`: from the right edge round to the bottom edge.
            Corner::TopLeft => (1, 1),
            Corner::TopRight => (-1, 1),
            Corner::BottomLeft => (1, -1),
            Corner::BottomRight => (-1, -1),
        }
    }
}

/// The older characters that are octant patterns (the half and quarter blocks
/// and a few Unicode 16 additions), with their masks.
const OLDER: [(u32, u8); 24] = [
    (0x2580, 0b0000_1111),
    (0x2584, 0b1111_0000),
    (0x258c, 0b0101_0101),
    (0x2590, 0b1010_1010),
    (0x2598, 0b0000_0101),
    (0x259d, 0b0000_1010),
    (0x2596, 0b0101_0000),
    (0x2597, 0b1010_0000),
    (0x259a, 0b1010_0101),
    (0x259e, 0b0101_1010),
    (0x2599, 0b1111_0101),
    (0x259b, 0b0101_1111),
    (0x259c, 0b1010_1111),
    (0x259f, 0b1111_1010),
    (0x1fb82, 0b0000_0011),
    (0x2582, 0b1100_0000),
    (0x2586, 0b1111_1100),
    (0x1fb85, 0b0011_1111),
    (0x1cea8, 0b0000_0001),
    (0x1ceab, 0b0000_0010),
    (0x1cea3, 0b0100_0000),
    (0x1cea0, 0b1000_0000),
    (0x1fbe6, 0b0001_0100),
    (0x1fbe7, 0b0010_1000),
];

/// U+1CD00.. in order: every mask from 1 to 254 that isn't an older character.
const OCTANTS: [u8; 230] = octants();

#[expect(
    clippy::indexing_slicing,
    reason = "a const fn can't use `get`; every index is bounded by its loop"
)]
const fn octants() -> [u8; 230] {
    let mut table = [0; 230];
    let mut next = 0;
    let mut mask = 1;
    while mask < 255 {
        let mut older = false;
        let mut i = 0;
        while i < OLDER.len() {
            if OLDER[i].1 == mask {
                older = true;
            }
            i += 1;
        }
        if !older && next < table.len() {
            table[next] = mask;
            next += 1;
        }
        mask += 1;
    }
    table
}

const OCTANT_FIRST: u32 = 0x1cd00;
const SEXTANT_FIRST: u32 = 0x1fb00;
const BRAILLE_FIRST: u32 = 0x2800;

/// The shape for a cell's symbol, if it's drawn as one.
pub fn shape(symbol: &str) -> Option<Shape> {
    let mut chars = symbol.chars();
    let c = chars.next()?;
    // A variation selector is fine (`❝︎`); anything else makes it text.
    if chars.any(|next| !matches!(next, '\u{fe0e}' | '\u{fe0f}')) {
        return None;
    }
    let cp = u32::from(c);
    if cp == 0x2588 {
        return Some(Shape::Octant(0xff));
    }
    if let Some(&(_, mask)) = OLDER.iter().find(|(older, _)| *older == cp) {
        return Some(Shape::Octant(mask));
    }
    if let Some(&mask) = cp
        .checked_sub(OCTANT_FIRST)
        .and_then(|i| OCTANTS.get(usize::try_from(i).ok()?))
    {
        return Some(Shape::Octant(mask));
    }
    if let Some(i) = cp.checked_sub(SEXTANT_FIRST).filter(|i| *i < 60) {
        // Masks 1..=62 without the two half blocks (21 and 42).
        let mut mask = u8::try_from(i).ok()? + 1;
        if mask >= 21 {
            mask += 1;
        }
        if mask >= 42 {
            mask += 1;
        }
        return Some(Shape::Sextant(mask));
    }
    if let Some(dots) = cp
        .checked_sub(BRAILLE_FIRST)
        .and_then(|i| u8::try_from(i).ok())
    {
        return (dots != 0).then_some(Shape::Braille(dots));
    }
    eighths(cp)
        .or_else(|| curves(cp))
        .or_else(|| lines(cp))
        .or_else(|| symbol_shape(cp))
}

/// Partial blocks and shades.
fn eighths(cp: u32) -> Option<Shape> {
    let n = |v: u32| u8::try_from(v).ok();
    Some(match cp {
        // ▁..▇ lower eighths (▂ ▆ are octants, caught earlier).
        0x2581..=0x2587 => Shape::Eighths(0, 8 - n(cp - 0x2580)?, 8, 8),
        // ▉..▏ left eighths.
        0x2589..=0x258f => Shape::Eighths(0, 0, n(0x2590 - cp)?, 8),
        0x2594 => Shape::Eighths(0, 0, 8, 1),
        0x2595 => Shape::Eighths(7, 0, 8, 8),
        0x2591..=0x2593 => Shape::Shade(n(cp - 0x2590)?),
        // 🮂..🮆 upper eighths (🮂 🮅 are octants, caught earlier).
        0x1fb83..=0x1fb86 => Shape::Eighths(
            0,
            0,
            8,
            *[3, 5, 6, 7].get(usize::try_from(cp - 0x1fb83).ok()?)?,
        ),
        // 🮇..🮋 right eighths.
        0x1fb87..=0x1fb8b => Shape::Eighths(
            8 - *[2, 3, 5, 6, 7].get(usize::try_from(cp - 0x1fb87).ok()?)?,
            0,
            8,
            8,
        ),
        _ => return None,
    })
}

/// Quarter circles and Powerline caps.
fn curves(cp: u32) -> Option<Shape> {
    Some(match cp {
        0x1fbec => Shape::QuarterCircle(Corner::TopRight),
        0x1fbed => Shape::QuarterCircle(Corner::BottomLeft),
        0x1fbee => Shape::QuarterCircle(Corner::BottomRight),
        0x1fbef => Shape::QuarterCircle(Corner::TopLeft),
        0xe0b0 => Shape::Powerline(Powerline::RightTriangle),
        0xe0b2 => Shape::Powerline(Powerline::LeftTriangle),
        0xe0b4 => Shape::Powerline(Powerline::RightRound),
        0xe0b6 => Shape::Powerline(Powerline::LeftRound),
        _ => return None,
    })
}

/// Box drawing: `[up, right, down, left]`.
fn lines(cp: u32) -> Option<Shape> {
    use Line::{Double as D, Heavy as H, Light as L, None as N};
    let lines = match cp {
        0x2500 => [N, L, N, L],
        0x2501 => [N, H, N, H],
        0x2502 => [L, N, L, N],
        0x2503 => [H, N, H, N],
        0x250c => [N, L, L, N],
        0x250f => [N, H, H, N],
        0x2510 => [N, N, L, L],
        0x2513 => [N, N, H, H],
        0x2514 => [L, L, N, N],
        0x2517 => [H, H, N, N],
        0x2518 => [L, N, N, L],
        0x251b => [H, N, N, H],
        0x251c => [L, L, L, N],
        0x2524 => [L, N, L, L],
        0x252c => [N, L, L, L],
        0x2534 => [L, L, N, L],
        0x253c => [L, L, L, L],
        0x2550 => [N, D, N, D],
        0x2551 => [D, N, D, N],
        0x2574 => [N, N, N, L],
        0x2575 => [L, N, N, N],
        0x2576 => [N, L, N, N],
        0x2577 => [N, N, L, N],
        0x2578 => [N, N, N, H],
        0x2579 => [H, N, N, N],
        0x257a => [N, H, N, N],
        0x257b => [N, N, H, N],
        0x256d => return Some(Shape::Arc(Corner::TopLeft)),
        0x256e => return Some(Shape::Arc(Corner::TopRight)),
        0x2570 => return Some(Shape::Arc(Corner::BottomLeft)),
        0x256f => return Some(Shape::Arc(Corner::BottomRight)),
        _ => return None,
    };
    Some(Shape::Lines(lines))
}

fn symbol_shape(cp: u32) -> Option<Shape> {
    Some(Shape::Symbol(match cp {
        0x21b3 => Symbol::ReturnArrow,
        0x270e => Symbol::Pencil,
        0x275d => Symbol::Quote,
        0x275e => Symbol::CloseQuote,
        _ => return None,
    }))
}

/// Braille dot `i` (0-based bit) as `(column, row)` in a 2×4 grid.
pub const fn braille_dot(bit: u8) -> (u8, u8) {
    match bit {
        0 => (0, 0),
        1 => (0, 1),
        2 => (0, 2),
        3 => (1, 0),
        4 => (1, 1),
        5 => (1, 2),
        6 => (0, 3),
        _ => (1, 3),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(cp: u32) -> Option<Shape> {
        shape(&char::from_u32(cp).unwrap().to_string())
    }

    #[test]
    fn the_octant_block_is_every_remaining_mask_in_order() {
        assert_eq!(at(0x1cd00), Some(Shape::Octant(0b0000_0100)));
        assert_eq!(at(0x1cde5), Some(Shape::Octant(0b1111_1110)));
        assert_eq!(at(0x1cde6), None);
        let mut seen: Vec<u8> = OCTANTS.to_vec();
        seen.extend(OLDER.iter().map(|(_, m)| *m));
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), 254, "every mask but empty and full, once");
    }

    #[test]
    fn half_blocks_and_full_block_are_octant_patterns() {
        assert_eq!(shape("▀"), Some(Shape::Octant(0b0000_1111)));
        assert_eq!(shape("▄"), Some(Shape::Octant(0b1111_0000)));
        assert_eq!(shape("█"), Some(Shape::Octant(0xff)));
        assert_eq!(shape("▌"), Some(Shape::Octant(0b0101_0101)));
    }

    #[test]
    fn sextants_skip_the_half_blocks() {
        assert_eq!(at(0x1fb00), Some(Shape::Sextant(1)));
        assert_eq!(at(0x1fb13), Some(Shape::Sextant(20)));
        assert_eq!(at(0x1fb14), Some(Shape::Sextant(22)));
        assert_eq!(at(0x1fb3b), Some(Shape::Sextant(62)));
    }

    #[test]
    fn eighths_shades_curves_and_lines() {
        assert_eq!(shape("▁"), Some(Shape::Eighths(0, 7, 8, 8)));
        assert_eq!(shape("▏"), Some(Shape::Eighths(0, 0, 1, 8)));
        assert_eq!(shape("▒"), Some(Shape::Shade(2)));
        assert_eq!(at(0x1fbef), Some(Shape::QuarterCircle(Corner::TopLeft)));
        assert_eq!(at(0xe0b4), Some(Shape::Powerline(Powerline::RightRound)));
        assert_eq!(
            shape("─"),
            Some(Shape::Lines([
                Line::None,
                Line::Light,
                Line::None,
                Line::Light
            ]))
        );
        assert_eq!(shape("╭"), Some(Shape::Arc(Corner::TopLeft)));
    }

    #[test]
    fn braille_and_symbols() {
        assert_eq!(shape("⠀"), None, "blank braille is a space");
        assert_eq!(shape("⣿"), Some(Shape::Braille(0xff)));
        assert_eq!(shape("↳"), Some(Shape::Symbol(Symbol::ReturnArrow)));
        assert_eq!(shape("❝\u{fe0e}"), Some(Shape::Symbol(Symbol::Quote)));
        assert_eq!(shape("❞"), Some(Shape::Symbol(Symbol::CloseQuote)));
    }

    #[test]
    fn text_stays_text() {
        for text in ["a", "M", "🧗", "🇫🇷", "▀a", " ", ""] {
            assert_eq!(shape(text), None, "{text:?}");
        }
    }
}
