//! Inline images in the demo (D117, D135). The browser decodes and scales
//! (`createImageBitmap`, a canvas), so the wasm module carries no image
//! codecs. Like faro in kitty, the demo draws real pictures into the cells
//! faro reserves for them ([`Picture`]: the cells are blanked and the painter
//! draws the bitmap over them); with half-blocks chosen in the settings it
//! turns pixels into cells ([`Halfblocks`]), as faro does in other terminals.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

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

/// Where a frame put a [`Picture`]: its whole box in cells (the top may be
/// above the screen when it's scrolled), and the part of it that shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placement {
    /// The picture's bitmap, as the shell keeps it.
    pub id: u64,
    pub x: i32,
    pub y: i32,
    pub width: u16,
    pub height: u16,
    pub visible: Rect,
}

impl Placement {
    /// The screen rows the picture shows on.
    pub fn rows(&self) -> std::ops::Range<u16> {
        self.visible.top()..self.visible.bottom()
    }
}

/// A picture drawn as pixels, like faro's kitty mode (D135): drawing blanks
/// its cells (so nothing faro put there shows through a transparent image)
/// and notes where it went; the painter draws the bitmap there, stretched to
/// the box as faro's kitty images are.
#[derive(Debug)]
pub struct Picture {
    id: u64,
    width: u16,
    height: u16,
    placed: Rc<RefCell<Vec<Placement>>>,
}

impl DrawImage for Picture {
    fn draw(&self, x: i16, y: i16, clip_area: Rect, buf: &mut Buffer, _colors: ColorDepth) {
        let left = u16::try_from(x.max(0)).unwrap_or(0);
        let Some(visible) = visible_rect(left, y, self.width, self.height, clip_area) else {
            return;
        };
        for sy in visible.top()..visible.bottom() {
            for sx in visible.left()..visible.right() {
                if let Some(cell) = buf.cell_mut((sx, sy)) {
                    cell.reset();
                }
            }
        }
        self.placed.borrow_mut().push(Placement {
            id: self.id,
            x: i32::from(clip_area.x) + i32::from(left),
            y: i32::from(clip_area.y) + i32::from(y),
            width: self.width,
            height: self.height,
            visible,
        });
    }
}

/// A finished decode.
#[derive(Debug)]
pub enum Decoded {
    Halfblocks(Halfblocks),
    /// A bitmap the shell keeps under this id.
    Picture(u64),
}

#[derive(Debug)]
enum Entry {
    Halfblocks(Halfblocks),
    Picture(Picture),
}

/// The decoded images, and which are being decoded.
#[derive(Debug, Default)]
pub struct Images {
    active: Option<ImageProtocol>,
    entries: HashMap<ImageKey, Option<Entry>>,
    /// Insertion order, for dropping the oldest.
    order: Vec<ImageKey>,
    in_flight: HashSet<ImageKey>,
    /// Where the frame being drawn put its pictures.
    placed: Rc<RefCell<Vec<Placement>>>,
}

impl Images {
    /// Before a frame: pictures, as in kitty (D135), unless the settings
    /// choose half-blocks or turn images off. 256 colours mean half-blocks
    /// and 16 none, as in the terminal.
    pub fn prepare_frame(&mut self, state: &State) {
        self.active = resolve_protocol(
            state.config.media.inline_images,
            state.colors(),
            GraphicsSupport::Direct,
            ImageProtocol::Kitty,
        );
        self.placed.borrow_mut().clear();
    }

    /// After a frame: where it put its pictures.
    pub fn placements(&self) -> Vec<Placement> {
        self.placed.borrow().clone()
    }

    /// After a frame: of the images it wanted, those to decode now (each
    /// only once).
    pub fn to_decode(&mut self, wanted: &[ImageKey]) -> Vec<ImageKey> {
        wanted
            .iter()
            .filter(|key| !self.entries.contains_key(key))
            .filter(|key| self.in_flight.insert((*key).clone()))
            .cloned()
            .collect()
    }

    /// A decode finished (`None`: it failed and is drawn as such). Returns
    /// the bitmap of a picture dropped to make room, for the shell to free.
    pub fn insert(&mut self, key: ImageKey, image: Option<Decoded>) -> Option<u64> {
        self.in_flight.remove(&key);
        let mut dropped = None;
        if self.entries.len() >= LIMIT && !self.order.is_empty() {
            let oldest = self.order.remove(0);
            if let Some(Some(Entry::Picture(picture))) = self.entries.remove(&oldest) {
                dropped = Some(picture.id);
            }
        }
        let entry = image.map(|image| match image {
            Decoded::Halfblocks(cells) => Entry::Halfblocks(cells),
            Decoded::Picture(id) => Entry::Picture(Picture {
                id,
                width: key.width,
                height: key.height,
                placed: Rc::clone(&self.placed),
            }),
        });
        self.order.push(key.clone());
        self.entries.insert(key, entry);
        dropped
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
            Some(Entry::Halfblocks(cells)) => CachedImage::Ready(cells),
            Some(Entry::Picture(picture)) => CachedImage::Ready(picture),
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

    fn picture_key(name: &str) -> ImageKey {
        ImageKey {
            protocol: ImageProtocol::Kitty,
            width: 3,
            height: 4,
            ..key(name)
        }
    }

    /// Pictures are drawn like faro's kitty images (D135): the cells are
    /// blanked and the frame notes where the picture went, scrolled-off top
    /// included, for the painter to draw it there.
    #[test]
    fn a_picture_blanks_its_cells_and_notes_where_it_went() {
        let mut images = Images::default();
        assert_eq!(
            images.insert(picture_key("p"), Some(Decoded::Picture(7))),
            None
        );
        let Some(CachedImage::Ready(picture)) = images.get(&picture_key("p")) else {
            panic!("the picture is ready");
        };
        let mut buf = Buffer::empty(Rect::new(0, 0, 10, 6));
        for cell in &mut buf.content {
            cell.set_symbol("x").set_bg(Color::Red);
        }
        let pane = Rect::new(2, 1, 8, 5);
        picture.draw(1, -1, pane, &mut buf, ColorDepth::Truecolor);
        assert_eq!(
            images.placements(),
            [Placement {
                id: 7,
                x: 3,
                y: 0,
                width: 3,
                height: 4,
                visible: Rect::new(3, 1, 3, 3),
            }]
        );
        assert_eq!(buf[(3, 1)].symbol(), " ", "blanked");
        assert_eq!(buf[(3, 1)].bg, Color::Reset, "the theme's background");
        assert_eq!(buf[(5, 3)].symbol(), " ", "blanked");
        assert_eq!(buf[(6, 1)].symbol(), "x", "right of the picture");
        assert_eq!(buf[(3, 0)].symbol(), "x", "above the pane");
        assert_eq!(buf[(3, 4)].symbol(), "x", "below the picture");
    }

    #[test]
    fn pictures_follow_the_settings_and_free_their_bitmaps() {
        let me = "00000000-0000-0000-0000-000000000001".parse().unwrap();
        let mut state = State::new(me, Vec::new(), Vec::new());
        let mut images = Images::default();
        images.prepare_frame(&state);
        assert_eq!(images.active_protocol(), Some(ImageProtocol::Kitty), "auto");
        state.config.media.inline_images = faro_core::config::InlineImages::Halfblocks;
        images.prepare_frame(&state);
        assert_eq!(images.active_protocol(), Some(ImageProtocol::Halfblocks));

        images.insert(picture_key("first"), Some(Decoded::Picture(1)));
        for n in 1..LIMIT {
            images.insert(key(&n.to_string()), None);
        }
        assert_eq!(
            images.insert(key("one more"), None),
            Some(1),
            "the oldest picture's bitmap is freed"
        );
    }
}
