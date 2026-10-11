//! Rounded bubbles drawn on a grid of sub-cells (D98). Each cell is split
//! into 2 × 4 octants; a bubble is a rectangle of octants with rounded
//! corners, and the cells it covers are drawn with whichever glyph fills
//! exactly those octants. Rectangles that overlap merge, which is how a
//! compact run's bubbles share their aligned edge and how a wider bubble's
//! corner lands in its neighbour's row.
//!
//! The glyphs: Unicode 16's octants (U+1CD00–1CDE5) for the patterns older
//! block characters don't cover (`▐`, `▂`, `🮂`, quadrants, …), and its
//! justified quarter circles (U+1FBEC–1FBEF, radius half a cell wide, centred
//! on a cell corner) for a rounded corner alone in its cell. kitty draws all of
//! them itself; each is one cell wide everywhere faro measures.

use std::ops::Range;

/// A corner of a rectangle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Corner {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

/// A rectangle of octants: `x` counts two per cell from the grid's left edge,
/// `y` four per row from the grid's first row (it may start above the grid or
/// end below it; what's outside is clipped). Corners on a rounded side are
/// drawn as quarter circles when nothing else shares their cell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Rect {
    pub x: Range<usize>,
    pub y: Range<isize>,
    pub round_left: bool,
    pub round_right: bool,
}

/// What a cell of the grid shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Cell {
    /// Nothing: the pane's background.
    Empty,
    /// Entirely inside a bubble: its fill.
    Full,
    /// Partly covered: this glyph in the bubble's colour on the pane's.
    Glyph(char),
}

/// Cells `cols` wide and `rows` high, with the octants each rectangle covers.
pub(super) struct Grid {
    cols: usize,
    rows: usize,
    masks: Vec<u8>,
    corners: Vec<(usize, Corner, u8)>,
}

impl Grid {
    pub fn new(cols: usize, rows: usize) -> Self {
        Self {
            cols,
            rows,
            masks: vec![0; cols * rows],
            corners: Vec::new(),
        }
    }

    pub fn add(&mut self, rect: &Rect) {
        for sy in rect.y.clone() {
            for sx in rect.x.clone() {
                if let Some((index, bit)) = self.octant(sx, sy)
                    && let Some(mask) = self.masks.get_mut(index)
                {
                    *mask |= bit;
                }
            }
        }
        let (Some(right), Some(bottom)) = (rect.x.end.checked_sub(1), rect.y.end.checked_sub(1))
        else {
            return;
        };
        if rect.x.is_empty() || rect.y.is_empty() {
            return;
        }
        let (left, top) = (rect.x.start, rect.y.start);
        if rect.round_left {
            self.corner(Corner::TopLeft, left, top);
            self.corner(Corner::BottomLeft, left, bottom);
        }
        if rect.round_right {
            self.corner(Corner::TopRight, right, top);
            self.corner(Corner::BottomRight, right, bottom);
        }
    }

    /// The cell at `col`, `row`.
    pub fn cell(&self, col: usize, row: usize) -> Cell {
        let index = row * self.cols + col;
        match self.masks.get(index).copied().unwrap_or(0) {
            0 => Cell::Empty,
            0xFF => Cell::Full,
            mask => Cell::Glyph(
                self.corners
                    .iter()
                    .filter(|(at, _, bit)| *at == index && *bit == mask)
                    .find_map(|(_, corner, bit)| disk(*corner, *bit))
                    .unwrap_or_else(|| octant(mask)),
            ),
        }
    }

    fn corner(&mut self, corner: Corner, sx: usize, sy: isize) {
        if let Some((index, bit)) = self.octant(sx, sy) {
            self.corners.push((index, corner, bit));
        }
    }

    /// The cell index and bit of octant `sx`, `sy`, if it's on the grid.
    fn octant(&self, sx: usize, sy: isize) -> Option<(usize, u8)> {
        let sy = usize::try_from(sy).ok()?;
        let (col, row) = (sx / 2, sy / 4);
        (col < self.cols && row < self.rows).then(|| (row * self.cols + col, bit(sx, sy)))
    }
}

/// The bit for octant `sx`, `sy` within its cell: `y * 2 + x`, so bit 0 is
/// the top left octant (Unicode's octant 1) and bit 7 the bottom right (8).
const fn bit(sx: usize, sy: usize) -> u8 {
    1 << ((sy % 4) * 2 + sx % 2)
}

/// A quarter circle for `corner`, when `bit` is the cell's corner octant on
/// that side (its centre is then on the cell's corner).
fn disk(corner: Corner, bit: u8) -> Option<char> {
    match corner {
        Corner::TopLeft if bit == self::bit(1, 3) => Some('\u{1FBEE}'),
        Corner::TopRight if bit == self::bit(0, 3) => Some('\u{1FBED}'),
        Corner::BottomLeft if bit == self::bit(1, 0) => Some('\u{1FBEC}'),
        Corner::BottomRight if bit == self::bit(0, 0) => Some('\u{1FBEF}'),
        Corner::TopLeft | Corner::TopRight | Corner::BottomLeft | Corner::BottomRight => None,
    }
}

/// Patterns older characters already draw, by the octants they fill (Unicode
/// numbers octants 1–8 left to right, top to bottom). Unicode 16's octants
/// cover the rest, in ascending order of the pattern.
const OLDER: [(char, u8); 24] = [
    ('\u{2580}', 0b0000_1111),  // ▀ 1234
    ('\u{2584}', 0b1111_0000),  // ▄ 5678
    ('\u{258C}', 0b0101_0101),  // ▌ 1357
    ('\u{2590}', 0b1010_1010),  // ▐ 2468
    ('\u{2598}', 0b0000_0101),  // ▘ 13
    ('\u{259D}', 0b0000_1010),  // ▝ 24
    ('\u{2596}', 0b0101_0000),  // ▖ 57
    ('\u{2597}', 0b1010_0000),  // ▗ 68
    ('\u{259A}', 0b1010_0101),  // ▚ 13 68
    ('\u{259E}', 0b0101_1010),  // ▞ 24 57
    ('\u{2599}', 0b1111_0101),  // ▙ 13 5678
    ('\u{259B}', 0b0101_1111),  // ▛ 1234 57
    ('\u{259C}', 0b1010_1111),  // ▜ 1234 68
    ('\u{259F}', 0b1111_1010),  // ▟ 24 5678
    ('\u{1FB82}', 0b0000_0011), // 🮂 12
    ('\u{2582}', 0b1100_0000),  // ▂ 78
    ('\u{2586}', 0b1111_1100),  // ▆ 345678
    ('\u{1FB85}', 0b0011_1111), // 🮅 123456
    ('\u{1CEA8}', 0b0000_0001), // 1
    ('\u{1CEAB}', 0b0000_0010), // 2
    ('\u{1CEA3}', 0b0100_0000), // 7
    ('\u{1CEA0}', 0b1000_0000), // 8
    ('\u{1FBE6}', 0b0001_0100), // 35
    ('\u{1FBE7}', 0b0010_1000), // 46
];

/// The glyph filling exactly the octants in `mask` (not 0 or 0xFF).
pub(super) fn octant(mask: u8) -> char {
    if let Some((glyph, _)) = OLDER.iter().find(|(_, m)| *m == mask) {
        return *glyph;
    }
    // Patterns below this one that aren't octant characters: the empty one
    // and the older characters'.
    let taken = 1 + OLDER.iter().filter(|(_, m)| *m < mask).count();
    u32::try_from(usize::from(mask) - taken)
        .ok()
        .and_then(|offset| char::from_u32(0x1CD00 + offset))
        .unwrap_or(' ')
}

/// Whether `symbol` is a glyph drawn on the pane's background around a
/// rounded bubble (an octant pattern, a quarter circle or a Powerline cap).
pub(in crate::ui) fn is_edge(symbol: &str) -> bool {
    let mut chars = symbol.chars();
    let (Some(c), None) = (chars.next(), chars.next()) else {
        return false;
    };
    OLDER.iter().any(|(glyph, _)| *glyph == c)
        || matches!(c, '\u{1CD00}'..='\u{1CDE5}' | '\u{1FBEC}'..='\u{1FBEF}' | '\u{E0B4}' | '\u{E0B6}')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_pattern_has_its_own_glyph() {
        let glyphs: std::collections::HashSet<char> = (1..=254).map(octant).collect();
        assert_eq!(glyphs.len(), 254);
        assert_eq!(octant(0b0000_0100), '\u{1CD00}', "BLOCK OCTANT-3");
        assert_eq!(octant(0b0000_0110), '\u{1CD01}', "BLOCK OCTANT-23");
        assert_eq!(octant(0b1111_1110), '\u{1CDE5}', "BLOCK OCTANT-2345678");
        assert!((1..=254).all(|m| is_edge(&octant(m).to_string())));
    }

    #[test]
    fn a_lone_bubble_has_round_corners_and_thin_edges() {
        // Text on row 1, cells 1..3; side cells 0 and 4; edges on rows 0 and 2.
        let mut grid = Grid::new(5, 3);
        grid.add(&Rect {
            x: 1..9,
            y: 3..9,
            round_left: true,
            round_right: true,
        });
        let row = |r| (0..5).map(|c| grid.cell(c, r)).collect::<Vec<_>>();
        let g = Cell::Glyph;
        assert_eq!(
            row(0),
            vec![g('\u{1FBEE}'), g('▂'), g('▂'), g('▂'), g('\u{1FBED}')]
        );
        assert_eq!(
            row(1),
            vec![g('▐'), Cell::Full, Cell::Full, Cell::Full, g('▌')]
        );
        assert_eq!(
            row(2),
            vec![
                g('\u{1FBEC}'),
                g('\u{1FB82}'),
                g('\u{1FB82}'),
                g('\u{1FB82}'),
                g('\u{1FBEF}')
            ]
        );
    }

    #[test]
    fn a_wider_bubble_rounds_its_corner_in_the_next_row() {
        // A three-cell-wide bubble on row 0 above a one-cell one on row 1.
        let mut grid = Grid::new(6, 2);
        grid.add(&Rect {
            x: 1..11,
            y: -1..5,
            round_left: true,
            round_right: true,
        });
        grid.add(&Rect {
            x: 1..5,
            y: 3..9,
            round_left: true,
            round_right: true,
        });
        let g = Cell::Glyph;
        assert_eq!(grid.cell(0, 1), g('▐'), "the aligned side is one edge");
        assert_eq!(grid.cell(3, 1), g('\u{1FB82}'));
        assert_eq!(grid.cell(5, 1), g('\u{1FBEF}'), "a quarter circle");
    }
}
