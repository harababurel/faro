//! The favicon (WEBSITE.md §5.4): the app's small lighthouse, pixel for
//! pixel. It's drawn with half-blocks, so its 9 × 8 cells are 9 × 16
//! pixels: a 16-pixel-tall icon with nothing scaled. It sits on a dark,
//! rounded tile in faro-dark's colours, so it reads on light and dark tab
//! bars alike (white stripes would vanish on a light one).
//!
//! Three files, all from the same pixels: `favicon.svg` (modern browsers),
//! `favicon.ico` (16 and 32 px PNGs, for everything else and for the
//! `/favicon.ico` browsers ask for unprompted) and `apple-touch-icon.png`
//! (180 px, full-bleed: iOS rounds the corners itself).

use std::fmt::Write as _;

use faro_app::theme::{Theme, UserThemes};
use faro_app::ui::logo::{SMALL_SIZE, draw_small_lighthouse};
use faro_web::color::{self, Rgb};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use resvg::tiny_skia;

/// The icon grid: 16 × 16 pixels.
const GRID: u32 = 16;
/// The lighthouse's left edge on the grid (9 wide: 3 left, 4 right).
const LEFT: u32 = 3;
/// The tile's corner radius on the grid.
const RADIUS: f32 = 3.0;
/// The home-screen icon's size and the size of one lighthouse pixel on it
/// (9 × 16 → 90 × 160, centred).
const APPLE: u32 = 180;
const APPLE_PIXEL: u32 = 10;

pub struct Icons {
    pub svg: String,
    pub ico: Vec<u8>,
    pub apple_touch: Vec<u8>,
}

/// The lighthouse as pixels: `(x, y, colour)` on its own 9 × 16 grid, and
/// the tile's colour.
struct Pixels {
    tile: Rgb,
    lit: Vec<(u32, u32, Rgb)>,
}

pub fn render() -> Result<Icons, String> {
    let pixels = pixels()?;
    Ok(Icons {
        svg: svg(&pixels),
        ico: ico(&[
            (16, png(&pixels, 16, 1, true)?),
            (32, png(&pixels, 32, 2, true)?),
        ]),
        apple_touch: png(&pixels, APPLE, APPLE_PIXEL, false)?,
    })
}

/// Draw the small lighthouse the way the app does (About, the narrow logo)
/// and read its half-blocks back as pixels.
fn pixels() -> Result<Pixels, String> {
    let user = UserThemes::default();
    let theme = Theme::find("faro-dark", &user)
        .ok_or_else(|| "the built-in theme faro-dark is missing".to_owned())?;
    let (width, height) = SMALL_SIZE;
    let area = Rect::new(0, 0, width, height);
    let mut buffer = Buffer::empty(area);
    draw_small_lighthouse(&mut buffer, area, 0, 0, theme);
    let tile = color::rgb(theme.background, Rgb(0x10, 0x12, 0x16));
    let mut lit = Vec::new();
    for y in 0..height {
        for x in 0..width {
            let Some(cell) = buffer.cell((x, y)) else {
                continue;
            };
            let (top, bottom) = match cell.symbol() {
                "█" => (true, true),
                "▀" => (true, false),
                "▄" => (false, true),
                _ => (false, false),
            };
            let fg = color::rgb(cell.fg, tile);
            let (x, y) = (u32::from(x), u32::from(y) * 2);
            if top {
                lit.push((x, y, fg));
            }
            if bottom {
                lit.push((x, y + 1, fg));
            }
        }
    }
    if lit.is_empty() {
        return Err("the small lighthouse drew nothing".to_owned());
    }
    Ok(Pixels { tile, lit })
}

/// One `<rect>` per pixel on the 16 × 16 grid, edges crisp.
fn svg(pixels: &Pixels) -> String {
    let mut out = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {GRID} {GRID}" shape-rendering="crispEdges"><rect width="{GRID}" height="{GRID}" rx="{RADIUS}" fill="{}"/>"#,
        pixels.tile.hex()
    );
    for &(x, y, colour) in &pixels.lit {
        let _ = write!(
            out,
            r#"<rect x="{}" y="{y}" width="1" height="1" fill="{}"/>"#,
            x + LEFT,
            colour.hex()
        );
    }
    out.push_str("</svg>\n");
    out
}

/// A `size` × `size` PNG with each lighthouse pixel `scale` × `scale`,
/// centred; on a rounded tile (`rounded`) or a full-bleed one.
fn png(pixels: &Pixels, size: u32, scale: u32, rounded: bool) -> Result<Vec<u8>, String> {
    let mut pixmap = tiny_skia::Pixmap::new(size, size)
        .ok_or_else(|| "the icon pixmap won't start".to_owned())?;
    let paint = |colour: Rgb| {
        let mut paint = tiny_skia::Paint::default();
        paint.set_color_rgba8(colour.0, colour.1, colour.2, 255);
        paint.anti_alias = false;
        paint
    };
    let edge = to_f32(size);
    let tile = if rounded {
        rounded_rect(edge, RADIUS * edge / to_f32(GRID))
    } else {
        tiny_skia::Rect::from_xywh(0.0, 0.0, edge, edge).map(tiny_skia::PathBuilder::from_rect)
    }
    .ok_or_else(|| "the icon tile won't build".to_owned())?;
    let mut tile_paint = paint(pixels.tile);
    tile_paint.anti_alias = true;
    pixmap.fill_path(
        &tile,
        &tile_paint,
        tiny_skia::FillRule::Winding,
        tiny_skia::Transform::identity(),
        None,
    );
    let (width, height) = (u32::from(SMALL_SIZE.0), u32::from(SMALL_SIZE.1) * 2);
    let left = (size - width * scale) / 2;
    let top = (size - height * scale) / 2;
    for &(x, y, colour) in &pixels.lit {
        let rect = tiny_skia::Rect::from_xywh(
            to_f32(left + x * scale),
            to_f32(top + y * scale),
            to_f32(scale),
            to_f32(scale),
        )
        .ok_or_else(|| "an icon pixel won't build".to_owned())?;
        pixmap.fill_rect(rect, &paint(colour), tiny_skia::Transform::identity(), None);
    }
    pixmap
        .encode_png()
        .map_err(|error| format!("the icon PNG won't encode: {error}"))
}

fn rounded_rect(side: f32, radius: f32) -> Option<tiny_skia::Path> {
    let (r, s) = (radius, side);
    let mut path = tiny_skia::PathBuilder::new();
    path.move_to(r, 0.0);
    path.line_to(s - r, 0.0);
    path.quad_to(s, 0.0, s, r);
    path.line_to(s, s - r);
    path.quad_to(s, s, s - r, s);
    path.line_to(r, s);
    path.quad_to(0.0, s, 0.0, s - r);
    path.line_to(0.0, r);
    path.quad_to(0.0, 0.0, r, 0.0);
    path.close();
    path.finish()
}

/// An ICO file holding PNGs (`(size, png)`), as every browser reads them.
fn ico(images: &[(u8, Vec<u8>)]) -> Vec<u8> {
    let count = u16::try_from(images.len()).unwrap_or(u16::MAX);
    let mut out = Vec::new();
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&count.to_le_bytes());
    let mut offset = 6 + 16 * u32::from(count);
    for (size, png) in images {
        let length = u32::try_from(png.len()).unwrap_or(u32::MAX);
        out.extend_from_slice(&[*size, *size, 0, 0]);
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&32u16.to_le_bytes());
        out.extend_from_slice(&length.to_le_bytes());
        out.extend_from_slice(&offset.to_le_bytes());
        offset = offset.saturating_add(length);
    }
    for (_, png) in images {
        out.extend_from_slice(png);
    }
    out
}

#[expect(
    clippy::cast_precision_loss,
    reason = "icon coordinates are a few hundred pixels at most"
)]
const fn to_f32(value: u32) -> f32 {
    value as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png_size(png: &[u8]) -> (u32, u32) {
        let word = |at: usize| u32::from_be_bytes(png.get(at..at + 4).unwrap().try_into().unwrap());
        (word(16), word(20))
    }

    #[test]
    fn the_icons_are_the_small_lighthouse_at_their_sizes() {
        let icons = render().unwrap();
        // The lighthouse fills the grid's height: its cap's tip on the top
        // row and its stone base on the bottom one.
        let pixels = pixels().unwrap();
        let rows: Vec<u32> = pixels.lit.iter().map(|&(_, y, _)| y).collect();
        assert_eq!(rows.iter().min(), Some(&0), "the cap's tip");
        assert_eq!(rows.iter().max(), Some(&15), "the stone base");
        assert!(pixels.lit.iter().all(|&(x, _, _)| x < 9), "9 pixels wide");

        assert!(icons.svg.starts_with("<svg"));
        assert!(icons.svg.contains(r#"viewBox="0 0 16 16""#));
        assert_eq!(icons.svg.matches("<rect").count(), pixels.lit.len() + 1);

        assert_eq!(
            &icons.ico[..6],
            &[0, 0, 1, 0, 2, 0],
            "an icon file with two images"
        );
        assert_eq!(&icons.ico[6..8], &[16, 16]);
        assert_eq!(&icons.ico[22..24], &[32, 32]);
        let first = u32::from_le_bytes(icons.ico[18..22].try_into().unwrap());
        assert_eq!(first, 6 + 2 * 16, "the first PNG follows the directory");
        let start = usize::try_from(first).unwrap();
        assert_eq!(png_size(&icons.ico[start..]), (16, 16));

        assert_eq!(png_size(&icons.apple_touch), (APPLE, APPLE));
    }
}
