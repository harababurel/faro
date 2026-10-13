//! Cell metrics (D122's amendment): the one rounding rule the canvas painter
//! and the site poster both use, so the poster's cells have exactly the shape
//! the live demo draws with when it takes the poster's place (§3.2).
//!
//! At a 16 px font and device pixel ratio 1 that's a 10 × 19 cell, not the
//! 9.6 × 19.2 the raw font ratios give: everything is rounded to whole device
//! pixels so neighbouring cells meet exactly and text stays crisp.

/// `JetBrains Mono`'s advance width in em (D107). The painter measures the
/// advance in the browser; the poster computes it from this, and for the
/// demo's font the two agree.
pub const ADVANCE: f32 = 0.6;

/// Line height as a multiple of the font size (as the Phase 0 prototype).
pub const LINE_HEIGHT: f32 = 1.2;

/// One cell in device pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Cell {
    /// The font size to draw at: `size × dpr`, rounded.
    pub font: f64,
    /// A cell's width: the font's advance at that size, rounded. The painter
    /// replaces it with what the browser measures for the loaded font.
    pub width: f64,
    /// A cell's height: [`LINE_HEIGHT`] times the font size, rounded.
    pub height: f64,
}

/// The cell `size` CSS pixels at `dpr` makes, in device pixels.
#[must_use]
pub fn cell(size: f64, dpr: f64) -> Cell {
    let font = (size * dpr).round();
    Cell {
        font,
        width: (font * f64::from(ADVANCE)).round().max(1.0),
        height: (font * f64::from(LINE_HEIGHT)).round(),
    }
}

/// What [`svg`](crate::svg) adds to its text group so a glyph drawn at
/// `font_size` lands on the next cell column instead of drifting inside a
/// `tspan`: the room between the font's own advance and the cell's width.
/// `None` when a cell is exactly one advance wide, and the attribute is then
/// left out.
#[must_use]
pub fn letter_spacing(cell_width: f32, font_size: f32) -> Option<f32> {
    let room = cell_width - font_size * ADVANCE;
    (room.abs() >= 0.005).then_some(room)
}

/// The type sizes the demo accepts (§4.4): finite, and 13 to 32 CSS
/// pixels — readable at the low end, sane at the high one. The page
/// chooses within this (16 max normally, 24 fullscreen); Rust keeps the
/// bound mechanical, it never auto-fits.
#[must_use]
pub fn valid_font_size(size: f64) -> bool {
    size.is_finite() && (13.0..=32.0).contains(&size)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Device pixels are whole numbers; compare with a tolerance rather than
    /// `==` (clippy's `float_cmp`).
    fn near(value: f64, want: f64) -> bool {
        (value - want).abs() < 0.001
    }

    #[test]
    fn a_16px_font_at_dpr_1_is_a_10_by_19_cell() {
        let cell = cell(16.0, 1.0);
        assert!(
            near(cell.font, 16.0) && near(cell.width, 10.0) && near(cell.height, 19.0),
            "{cell:?}"
        );
    }

    #[test]
    fn the_rounding_holds_at_other_ratios_and_sizes() {
        // What the demo's test page asks for: 14 CSS px at dpr 2.
        let big = cell(14.0, 2.0);
        assert!(
            near(big.font, 28.0) && near(big.width, 17.0) && near(big.height, 34.0),
            "{big:?}"
        );
        // A tiny font never loses its cell.
        assert!(near(cell(0.4, 1.0).width, 1.0));
    }

    #[test]
    fn letter_spacing_makes_up_what_the_cell_has_over_the_advance() {
        assert!(near(
            f64::from(letter_spacing(10.0, 16.0).unwrap_or_default()),
            0.4
        ));
        assert!(near(
            f64::from(letter_spacing(17.0, 28.0).unwrap_or_default()),
            0.2
        ));
        // A cell that is exactly one advance wide needs nothing added.
        assert!(letter_spacing(9.6, 16.0).is_none());
    }

    #[test]
    fn font_sizes_are_finite_and_between_13_and_32() {
        for ok in [13.0, 16.0, 24.0, 32.0] {
            assert!(valid_font_size(ok), "{ok} is allowed");
        }
        for no in [
            12.9,
            32.1,
            0.0,
            -16.0,
            f64::NAN,
            f64::INFINITY,
            f64::NEG_INFINITY,
        ] {
            assert!(!valid_font_size(no), "{no} is refused");
        }
    }
}
