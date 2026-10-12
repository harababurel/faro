//! Colours and cell widths, kept free of canvas types (as `glyphs` is) so the
//! Canvas painter (`paint.rs`) and a later SVG exporter (WEBSITE.md §6.4) can
//! share them. Builds for native targets too, so a poster generator can use it.

use std::fmt::Write as _;

use ratatui::buffer::{Cell, CellDiffOption};
use ratatui::style::{Color, Modifier};
use unicode_width::UnicodeWidthStr as _;

/// How far `DIM` text fades towards its background.
pub const DIM: f64 = 0.45;

/// A colour the renderer can paint, before it becomes CSS.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    /// `self` moved `t` of the way towards `to` (0 keeps `self`, 1 gives `to`).
    #[must_use]
    pub fn blend(self, to: Self, t: f64) -> Self {
        let mix = |a: u8, b: u8| {
            let v = f64::from(a).mul_add(1.0 - t, f64::from(b) * t);
            u8::try_from(clamp_u32(v, 255)).unwrap_or(u8::MAX)
        };
        Self(mix(self.0, to.0), mix(self.1, to.1), mix(self.2, to.2))
    }

    /// What the painter compares to skip redundant state changes.
    pub const fn key(self) -> u32 {
        (self.0 as u32) << 16 | (self.1 as u32) << 8 | self.2 as u32
    }

    /// `#rrggbb`, as the canvas's CSS and an SVG attribute want it.
    pub fn hex(self) -> String {
        let mut css = String::with_capacity(7);
        self.write_hex(&mut css);
        css
    }

    /// Clears `css` and writes `#rrggbb` into it, so a render loop can reuse
    /// one buffer instead of allocating per colour change.
    pub fn write_hex(self, css: &mut String) {
        css.clear();
        let _ = write!(css, "#{:02x}{:02x}{:02x}", self.0, self.1, self.2);
    }
}

/// The 16 ANSI colours (xterm's defaults), for themes that use them (`system`).
const ANSI: [Rgb; 16] = [
    Rgb(0x00, 0x00, 0x00),
    Rgb(0xcd, 0x00, 0x00),
    Rgb(0x00, 0xcd, 0x00),
    Rgb(0xcd, 0xcd, 0x00),
    Rgb(0x00, 0x00, 0xee),
    Rgb(0xcd, 0x00, 0xcd),
    Rgb(0x00, 0xcd, 0xcd),
    Rgb(0xe5, 0xe5, 0xe5),
    Rgb(0x7f, 0x7f, 0x7f),
    Rgb(0xff, 0x00, 0x00),
    Rgb(0x00, 0xff, 0x00),
    Rgb(0xff, 0xff, 0x00),
    Rgb(0x5c, 0x5c, 0xff),
    Rgb(0xff, 0x00, 0xff),
    Rgb(0x00, 0xff, 0xff),
    Rgb(0xff, 0xff, 0xff),
];

fn ansi(index: usize) -> Rgb {
    ANSI.get(index).copied().unwrap_or(Rgb(0x80, 0x80, 0x80))
}

/// A cell colour as RGB; `Reset` is the theme's default.
pub fn rgb(color: Color, default: Rgb) -> Rgb {
    match color {
        Color::Reset => default,
        Color::Rgb(r, g, b) => Rgb(r, g, b),
        Color::Indexed(i) => match faro_app::theme::indexed_rgb(i) {
            Some((r, g, b)) => Rgb(r, g, b),
            None => ansi(usize::from(i)),
        },
        Color::Black => ansi(0),
        Color::Red => ansi(1),
        Color::Green => ansi(2),
        Color::Yellow => ansi(3),
        Color::Blue => ansi(4),
        Color::Magenta => ansi(5),
        Color::Cyan => ansi(6),
        Color::Gray => ansi(7),
        Color::DarkGray => ansi(8),
        Color::LightRed => ansi(9),
        Color::LightGreen => ansi(10),
        Color::LightYellow => ansi(11),
        Color::LightBlue => ansi(12),
        Color::LightMagenta => ansi(13),
        Color::LightCyan => ansi(14),
        Color::White => ansi(15),
    }
}

/// A cell's foreground and background after `REVERSED` and `DIM`.
/// `foreground` and `background` are the theme's defaults, what `Color::Reset`
/// means.
pub fn cell_colors(cell: &Cell, foreground: Rgb, background: Rgb) -> (Rgb, Rgb) {
    let (mut fg, mut bg) = (rgb(cell.fg, foreground), rgb(cell.bg, background));
    if cell.modifier.contains(Modifier::REVERSED) {
        std::mem::swap(&mut fg, &mut bg);
    }
    if cell.modifier.contains(Modifier::DIM) {
        fg = fg.blend(bg, DIM);
    }
    (fg, bg)
}

/// How many cells a cell's symbol covers: faro's forced width, else Unicode's.
pub fn cell_width(cell: &Cell) -> u16 {
    let width = match cell.diff_option {
        CellDiffOption::ForcedWidth(w) => usize::from(w.get()),
        CellDiffOption::None | CellDiffOption::Skip | CellDiffOption::AlwaysUpdate => {
            cell.symbol().width()
        }
    };
    u16::try_from(width.max(1)).unwrap_or(1)
}

/// `v` rounded and clamped to `0..=max` (NaN is 0). The painter sizes the
/// canvas with it as well as blending colours.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "clamped to 0..=max first"
)]
pub(crate) fn clamp_u32(v: f64, max: u32) -> u32 {
    let v = v.round();
    if v.is_nan() || v <= 0.0 {
        0
    } else if v >= f64::from(max) {
        max
    } else {
        v as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::num::NonZeroU16;

    const FG: Rgb = Rgb(0xe6, 0xe6, 0xe6);
    const BG: Rgb = Rgb(0x10, 0x12, 0x16);

    #[test]
    fn reset_is_the_theme_default() {
        assert_eq!(rgb(Color::Reset, FG), FG);
        assert_eq!(rgb(Color::Reset, BG), BG);
    }

    #[test]
    fn rgb_and_indexed_come_through_as_them() {
        assert_eq!(rgb(Color::Rgb(1, 2, 3), FG), Rgb(1, 2, 3));
        // 16..=255 have a fixed value in xterm-256; 196 is full red.
        assert_eq!(rgb(Color::Indexed(196), FG), Rgb(0xff, 0x00, 0x00));
        assert_eq!(rgb(Color::Indexed(232), FG), Rgb(8, 8, 8));
        // 0..=15 depend on the user's palette, so they fall back to xterm's.
        assert_eq!(rgb(Color::Indexed(12), FG), ansi(12));
    }

    #[test]
    fn named_colours_are_the_ansi_table() {
        assert_eq!(rgb(Color::Black, FG), Rgb(0x00, 0x00, 0x00));
        assert_eq!(rgb(Color::Red, FG), Rgb(0xcd, 0x00, 0x00));
        assert_eq!(rgb(Color::LightBlue, FG), Rgb(0x5c, 0x5c, 0xff));
        assert_eq!(rgb(Color::White, FG), Rgb(0xff, 0xff, 0xff));
    }

    #[test]
    fn reverse_swaps_and_dim_blends_towards_the_background() {
        let plain = Cell::default();
        assert_eq!(cell_colors(&plain, FG, BG), (FG, BG));

        let mut reversed = Cell::default();
        reversed.fg = Color::Red;
        reversed.bg = Color::Indexed(196);
        reversed.modifier = Modifier::REVERSED;
        assert_eq!(
            cell_colors(&reversed, FG, BG),
            (Rgb(0xff, 0x00, 0x00), ansi(1))
        );

        let mut dim = Cell::default();
        dim.fg = Color::White;
        dim.bg = Color::Black;
        dim.modifier = Modifier::DIM;
        assert_eq!(
            cell_colors(&dim, FG, BG),
            (Rgb(0x8c, 0x8c, 0x8c), Rgb(0, 0, 0))
        );
        // The same blend `Rgb::blend` promises on its own.
        assert_eq!(
            cell_colors(&dim, FG, BG).0,
            Rgb(0xff, 0xff, 0xff).blend(Rgb(0, 0, 0), DIM)
        );
    }

    #[test]
    fn hex_is_lowercase_hash_rrggbb() {
        assert_eq!(Rgb(0x00, 0x00, 0x00).hex(), "#000000");
        assert_eq!(BG.hex(), "#101216");
        assert_eq!(Rgb(0xff, 0x0a, 0x0f).hex(), "#ff0a0f");
    }

    #[test]
    fn key_matches_the_colour() {
        assert_eq!(Rgb(0x12, 0x34, 0x56).key(), 0x12_34_56);
        assert_eq!(Rgb(1, 2, 3).key(), Rgb(1, 2, 3).key());
        assert_ne!(FG.key(), BG.key());
    }

    #[test]
    fn cell_width_prefers_the_forced_width() {
        let mut cell = Cell::default();
        assert_eq!(cell_width(&cell), 1, "a blank cell is still one cell");

        cell.set_symbol("🧗");
        assert_eq!(cell_width(&cell), 2, "Unicode's width otherwise");

        cell.diff_option = CellDiffOption::ForcedWidth(NonZeroU16::new(3).unwrap());
        assert_eq!(cell_width(&cell), 3, "faro's forced width wins");

        let mut skipped = Cell::default();
        skipped.set_symbol("a");
        skipped.diff_option = CellDiffOption::Skip;
        assert_eq!(cell_width(&skipped), 1);
    }
}
