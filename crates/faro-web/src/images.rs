//! Inline images in the demo (D117): half-blocks, two pixels per cell. The
//! browser decodes and scales (`createImageBitmap`, a canvas), so the wasm
//! module carries no image codecs; this module turns pixels into cells and
//! keeps them for drawing.

use std::collections::{HashMap, HashSet};

use faro_app::app::image_cache::{
    CachedImage, CellSize, DrawImage, HALFBLOCK_FONT, ImageCache, ImageKey, ImageProtocol,
    resolve_protocol, visible_rect,
};
use faro_app::app::state::{GraphicsSupport, State};
use faro_app::theme::{ColorDepth, nearest_256};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier};

/// At most this many decoded images are kept; the oldest go first.
const LIMIT: usize = 32;

/// An image as cells: for each, the top and bottom pixel (`None` where the
/// pixel is transparent).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Halfblocks {
    width: u16,
    height: u16,
    cells: Vec<[Option<(u8, u8, u8)>; 2]>,
}

impl Halfblocks {
    /// From RGBA pixels, `width` × `2 × height`, row by row (canvas
    /// `ImageData`). `None` if the size doesn't match.
    pub fn from_rgba(width: u16, height: u16, rgba: &[u8]) -> Option<Self> {
        let (columns, rows) = (usize::from(width), usize::from(height));
        if rgba.len() != columns * rows * 2 * 4 {
            return None;
        }
        let pixel = |column: usize, line: usize| {
            let at = (line * columns + column) * 4;
            match rgba.get(at..at + 4)? {
                [red, green, blue, alpha] if *alpha >= 128 => Some((*red, *green, *blue)),
                _ => None,
            }
        };
        let mut cells = Vec::with_capacity(columns * rows);
        for row in 0..rows {
            for column in 0..columns {
                cells.push([pixel(column, row * 2), pixel(column, row * 2 + 1)]);
            }
        }
        Some(Self {
            width,
            height,
            cells,
        })
    }
}

impl DrawImage for Halfblocks {
    fn draw(&self, x: i16, y: i16, clip_area: Rect, buf: &mut Buffer, colors: ColorDepth) {
        let left = u16::try_from(x.max(0)).unwrap_or(0);
        let Some(visible) = visible_rect(left, y, self.width, self.height, clip_area) else {
            return;
        };
        let top = i32::from(clip_area.y) + i32::from(y);
        let colour = |rgb: (u8, u8, u8)| {
            let c = Color::Rgb(rgb.0, rgb.1, rgb.2);
            if colors == ColorDepth::Indexed256 {
                nearest_256(c)
            } else {
                c
            }
        };
        for sy in visible.top()..visible.bottom() {
            let Ok(row) = usize::try_from(i32::from(sy) - top) else {
                continue;
            };
            for sx in visible.left()..visible.right() {
                let column = usize::from(sx - clip_area.x - left);
                let Some([upper, lower]) = self.cells.get(row * usize::from(self.width) + column)
                else {
                    continue;
                };
                let Some(cell) = buf.cell_mut((sx, sy)) else {
                    continue;
                };
                cell.modifier = Modifier::empty();
                let (symbol, fg, bg) = match (upper, lower) {
                    (Some(u), Some(l)) => ("▀", colour(*u), colour(*l)),
                    (Some(u), None) => ("▀", colour(*u), Color::Reset),
                    (None, Some(l)) => ("▄", colour(*l), Color::Reset),
                    (None, None) => (" ", Color::Reset, Color::Reset),
                };
                cell.set_symbol(symbol).set_fg(fg).set_bg(bg);
            }
        }
    }
}

/// The decoded images, and which are being decoded.
#[derive(Debug, Default)]
pub struct Images {
    active: Option<ImageProtocol>,
    entries: HashMap<ImageKey, Option<Halfblocks>>,
    /// Insertion order, for dropping the oldest.
    order: Vec<ImageKey>,
    in_flight: HashSet<ImageKey>,
}

impl Images {
    /// Before a frame: half-blocks, unless the settings turn images off.
    pub fn prepare_frame(&mut self, state: &State) {
        self.active = resolve_protocol(
            state.config.media.inline_images,
            state.colors(),
            GraphicsSupport::HalfblocksOnly,
            ImageProtocol::Halfblocks,
        );
    }

    /// After a frame: of the images it wanted, those to decode now (each
    /// only once).
    pub fn to_decode(&mut self, wanted: &[ImageKey]) -> Vec<ImageKey> {
        wanted
            .iter()
            .filter(|key| key.protocol == ImageProtocol::Halfblocks)
            .filter(|key| !self.entries.contains_key(key))
            .filter(|key| self.in_flight.insert((*key).clone()))
            .cloned()
            .collect()
    }

    /// A decode finished (`None`: it failed and is drawn as such).
    pub fn insert(&mut self, key: ImageKey, image: Option<Halfblocks>) {
        self.in_flight.remove(&key);
        if self.entries.len() >= LIMIT && !self.order.is_empty() {
            let oldest = self.order.remove(0);
            self.entries.remove(&oldest);
        }
        self.order.push(key.clone());
        self.entries.insert(key, image);
    }
}

impl ImageCache for Images {
    fn active_protocol(&self) -> Option<ImageProtocol> {
        self.active
    }

    fn font_size(&self) -> CellSize {
        HALFBLOCK_FONT
    }

    fn get(&self, key: &ImageKey) -> Option<CachedImage<'_>> {
        self.entries.get(key).map(|image| match image {
            Some(image) => CachedImage::Ready(image),
            None => CachedImage::Failed,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn key(name: &str) -> ImageKey {
        ImageKey {
            path: PathBuf::from(name),
            width: 2,
            height: 1,
            protocol: ImageProtocol::Halfblocks,
        }
    }

    #[test]
    fn pixels_become_half_blocks_with_transparency() {
        // 2 × 2 pixels: red over blue, then transparent over green.
        let rgba = [
            255, 0, 0, 255, 0, 0, 0, 0, //
            0, 0, 255, 255, 0, 255, 0, 255,
        ];
        let image = Halfblocks::from_rgba(2, 1, &rgba).unwrap();
        let mut buf = Buffer::empty(Rect::new(0, 0, 4, 2));
        image.draw(1, 1, buf.area, &mut buf, ColorDepth::Truecolor);
        let red_over_blue = &buf[(1, 1)];
        assert_eq!(red_over_blue.symbol(), "▀");
        assert_eq!(red_over_blue.fg, Color::Rgb(255, 0, 0));
        assert_eq!(red_over_blue.bg, Color::Rgb(0, 0, 255));
        let green_below = &buf[(2, 1)];
        assert_eq!(green_below.symbol(), "▄");
        assert_eq!(green_below.fg, Color::Rgb(0, 255, 0));
        assert_eq!(green_below.bg, Color::Reset);
        assert_eq!(buf[(1, 0)].symbol(), " ", "nothing above the image");
        assert!(Halfblocks::from_rgba(3, 1, &rgba).is_none(), "wrong size");
    }

    #[test]
    fn scrolled_off_rows_are_skipped() {
        let rgba = [200; 2 * 4 * 4];
        let image = Halfblocks::from_rgba(2, 2, &rgba).unwrap();
        let mut buf = Buffer::empty(Rect::new(0, 0, 2, 2));
        image.draw(0, -1, buf.area, &mut buf, ColorDepth::Truecolor);
        assert_eq!(buf[(0, 0)].symbol(), "▀", "the second row, at the top");
        assert_eq!(buf[(0, 1)].symbol(), " ", "nothing below the image");
    }

    #[test]
    fn each_image_is_decoded_once_and_the_cache_is_bounded() {
        let mut images = Images::default();
        assert_eq!(images.to_decode(&[key("a"), key("a")]), [key("a")]);
        assert!(images.to_decode(&[key("a")]).is_empty(), "in flight");
        images.insert(key("a"), None);
        assert!(matches!(images.get(&key("a")), Some(CachedImage::Failed)));
        assert!(images.to_decode(&[key("a")]).is_empty(), "done");
        for n in 0..LIMIT {
            images.insert(key(&n.to_string()), None);
        }
        assert!(images.get(&key("a")).is_none(), "the oldest went");
        assert_eq!(images.entries.len(), LIMIT);
    }
}
