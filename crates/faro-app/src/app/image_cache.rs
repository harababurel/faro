//! Inline images as drawing sees them (D78): which protocol is active, how big
//! an image box is, and the cache to draw decoded images from. The terminal
//! implements [`ImageCache`] with ratatui-image (`app::images`); renderers
//! without images use [`NoImages`].

use std::path::PathBuf;

use faro_core::config::InlineImages;
use ratatui::buffer::Buffer;
use ratatui::layout::{Rect, Size};

use super::state::GraphicsSupport;
use crate::theme::ColorDepth;

/// Maximum width of an inline image box in the message pane, in cells (D78).
pub const MAX_IMAGE_COLS: u16 = 40;
/// Maximum height of an inline photo box in the message pane, in cells (D78).
pub const MAX_IMAGE_ROWS: u16 = 14;
/// Maximum height of a sticker box in the message pane, in cells (D79).
pub const MAX_STICKER_ROWS: u16 = 8;

/// Assumed cell pixel size for Unicode half-blocks (`1:2` aspect ratio, D78).
pub const HALFBLOCK_FONT: CellSize = CellSize {
    width: 10,
    height: 20,
};

/// How an image is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ImageProtocol {
    /// Unicode half-blocks, two pixels per cell.
    Halfblocks,
    Sixel,
    Kitty,
    Iterm2,
}

impl ImageProtocol {
    /// A terminal graphics protocol (`Kitty`, `Sixel` or `Iterm2`) rather
    /// than half-blocks.
    pub const fn is_graphics(self) -> bool {
        match self {
            Self::Kitty | Self::Sixel | Self::Iterm2 => true,
            Self::Halfblocks => false,
        }
    }
}

/// A cell's size in pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CellSize {
    pub width: u16,
    pub height: u16,
}

/// Identifies a decoded, resized image slice in the cache (D78).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ImageKey {
    pub path: PathBuf,
    pub width: u16,
    pub height: u16,
    pub protocol: ImageProtocol,
    pub shape: ImageShape,
}

/// How a picture fills its box.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ImageShape {
    /// The whole picture, stretched to the box (whose shape already
    /// matches it): photos and stickers in the chat.
    #[default]
    Whole,
    /// A profile picture: cropped to fill the box, corners cut round where
    /// the protocol keeps transparency.
    Avatar,
}

impl ImageShape {
    /// How many decoded pictures of this shape the cache keeps: profile
    /// pictures are tiny and a tall list shows dozens at once.
    pub const fn cache_limit(self) -> usize {
        match self {
            Self::Whole => 32,
            Self::Avatar => 96,
        }
    }
}

/// The decoded images a frame draws from.
pub trait ImageCache {
    /// The protocol to draw with on this frame (`None` when inline images are
    /// off, or Auto/Halfblocks is selected at 16 colours).
    fn active_protocol(&self) -> Option<ImageProtocol>;

    /// Whether the active protocol is a terminal graphics protocol.
    fn is_graphics_protocol(&self) -> bool {
        self.active_protocol()
            .is_some_and(ImageProtocol::is_graphics)
    }

    /// Cell pixel size used to size image boxes for the active protocol.
    fn font_size(&self) -> CellSize;

    /// Look up `key`, marking it recently used. `None`: not decoded yet.
    fn get(&self, key: &ImageKey) -> Option<CachedImage<'_>>;
}

/// What [`ImageCache::get`] returns for a decoded [`ImageKey`].
pub enum CachedImage<'a> {
    Ready(&'a dyn DrawImage),
    Failed,
}

/// A decoded image, ready to draw.
pub trait DrawImage {
    /// Draw at `(x, y)` relative to `clip_area` (`y` may be negative: the top
    /// is scrolled off), clipped to it, with half-block colours mapped to the
    /// 256-colour palette when `colors` is `Indexed256`.
    fn draw(&self, x: i16, y: i16, clip_area: Rect, buf: &mut Buffer, colors: ColorDepth);

    /// Draw like [`Self::draw`], leaving any screen rectangles in `occluders`
    /// untouched so floating popups and the palette sit cleanly over the image.
    ///
    /// Returns `true` if the image was drawn (or was completely hidden under
    /// `occluders`), or `false` if this protocol cannot clip around `occluders`
    /// so the caller should leave its fallback badge or placeholder in the
    /// uncovered cells.
    fn draw_occluded(
        &self,
        x: i16,
        y: i16,
        clip_area: Rect,
        occluders: &[Rect],
        buf: &mut Buffer,
        colors: ColorDepth,
    ) -> bool {
        if occluders.is_empty() {
            self.draw(x, y, clip_area, buf, colors);
            return true;
        }
        false
    }
}

/// No inline images at all (the web demo, for now).
pub struct NoImages;

impl ImageCache for NoImages {
    fn active_protocol(&self) -> Option<ImageProtocol> {
        None
    }

    fn font_size(&self) -> CellSize {
        HALFBLOCK_FONT
    }

    fn get(&self, _key: &ImageKey) -> Option<CachedImage<'_>> {
        None
    }
}

/// The visible intersection of an image box `(x, y, width, height)` (relative
/// to `clip_area`) with `clip_area` in screen coordinates.
pub fn visible_rect(x: u16, y: i16, width: u16, height: u16, clip_area: Rect) -> Option<Rect> {
    if clip_area.width == 0 || clip_area.height == 0 || width == 0 || height == 0 {
        return None;
    }
    let top_skip = (-y).max(0).cast_unsigned();
    let dst_y = y.max(0).cast_unsigned();
    if x >= clip_area.width || dst_y >= clip_area.height || top_skip >= height {
        return None;
    }
    let vis_w = width.min(clip_area.width - x);
    let vis_h = (height - top_skip).min(clip_area.height - dst_y);
    (vis_w > 0 && vis_h > 0).then(|| Rect::new(clip_area.x + x, clip_area.y + dst_y, vis_w, vis_h))
}

/// Subtract every rectangle in `occluders` from `base`, returning disjoint
/// non-empty rectangles whose union is `base \ ⋃occluders`.
pub fn subtract_rects(base: Rect, occluders: &[Rect]) -> Vec<Rect> {
    if base.width == 0 || base.height == 0 {
        return Vec::new();
    }
    let mut parts = vec![base];
    for &occ in occluders {
        if occ.width == 0 || occ.height == 0 {
            continue;
        }
        let mut next = Vec::with_capacity(parts.len() + 3);
        for r in parts {
            if !r.intersects(occ) {
                next.push(r);
                continue;
            }
            let ix0 = r.left().max(occ.left());
            let ix1 = r.right().min(occ.right());
            let iy0 = r.top().max(occ.top());
            let iy1 = r.bottom().min(occ.bottom());
            if iy0 > r.top() {
                next.push(Rect::new(r.left(), r.top(), r.width, iy0 - r.top()));
            }
            if r.bottom() > iy1 {
                next.push(Rect::new(r.left(), iy1, r.width, r.bottom() - iy1));
            }
            if ix0 > r.left() {
                next.push(Rect::new(r.left(), iy0, ix0 - r.left(), iy1 - iy0));
            }
            if r.right() > ix1 {
                next.push(Rect::new(ix1, iy0, r.right() - ix1, iy1 - iy0));
            }
        }
        parts = next;
    }
    parts
}

/// Active protocol for the current settings and terminal environment (D78).
pub fn resolve_protocol(
    setting: InlineImages,
    colors: ColorDepth,
    graphics: GraphicsSupport,
    detected: ImageProtocol,
) -> Option<ImageProtocol> {
    match setting {
        InlineImages::Kitty => return Some(ImageProtocol::Kitty),
        InlineImages::Sixel => return Some(ImageProtocol::Sixel),
        InlineImages::Iterm2 => return Some(ImageProtocol::Iterm2),
        InlineImages::Auto | InlineImages::Halfblocks | InlineImages::Off => {}
    }
    if setting == InlineImages::Off || colors == ColorDepth::Ansi16 {
        return None;
    }
    if setting == InlineImages::Halfblocks
        || colors == ColorDepth::Indexed256
        || graphics == GraphicsSupport::HalfblocksOnly
    {
        return Some(ImageProtocol::Halfblocks);
    }
    Some(detected)
}

/// Compute the cell `(width, height)` box for an image of `img_width × img_height`
/// pixels, given the cell pixel size `font_size` and maximum cell bounds
/// `max_cols × max_rows` (D78).
pub fn box_size(
    img_width: u32,
    img_height: u32,
    font_size: CellSize,
    max_cols: u16,
    max_rows: u16,
) -> Size {
    let max_cols = max_cols.max(1);
    let max_rows = max_rows.max(1);
    let fw = u64::from(font_size.width.max(1));
    let fh = u64::from(font_size.height.max(1));
    let avail_w = u64::from(max_cols) * fw;
    let avail_h = u64::from(max_rows) * fh;
    let iw = u64::from(img_width.max(1));
    let ih = u64::from(img_height.max(1));
    let (scaled_w, scaled_h) = if iw * avail_h >= ih * avail_w {
        (avail_w, (ih * avail_w + iw / 2) / iw)
    } else {
        ((iw * avail_h + ih / 2) / ih, avail_h)
    };
    let cols = scaled_w.div_ceil(fw).clamp(1, u64::from(max_cols));
    let rows = scaled_h.div_ceil(fh).clamp(1, u64::from(max_rows));
    Size::new(
        u16::try_from(cols).unwrap_or(max_cols),
        u16::try_from(rows).unwrap_or(max_rows),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn box_size_preserves_aspect_ratio_within_bounds() {
        // 16:9 image in 40x14 box with 1:2 cells (400x280 px): width-limited to 40 cols, 12 rows.
        assert_eq!(
            box_size(1600, 900, HALFBLOCK_FONT, MAX_IMAGE_COLS, MAX_IMAGE_ROWS),
            Size::new(40, 12)
        );
        // 1:1 image in 40x14 box with 1:2 cells: height-limited to 14 rows = 28 cols.
        assert_eq!(
            box_size(500, 500, HALFBLOCK_FONT, MAX_IMAGE_COLS, MAX_IMAGE_ROWS),
            Size::new(28, 14)
        );
        // 1:1 sticker in 40x8 box with 1:2 cells: 16x8.
        assert_eq!(
            box_size(512, 512, HALFBLOCK_FONT, MAX_IMAGE_COLS, MAX_STICKER_ROWS),
            Size::new(16, 8)
        );
        // Narrow pane clamps width.
        assert_eq!(
            box_size(1600, 900, HALFBLOCK_FONT, 16, MAX_IMAGE_ROWS),
            Size::new(16, 5)
        );
    }

    #[test]
    fn protocol_resolution_respects_colour_depth_and_terminal_hops() {
        assert_eq!(
            resolve_protocol(
                InlineImages::Auto,
                ColorDepth::Truecolor,
                GraphicsSupport::Direct,
                ImageProtocol::Kitty,
            ),
            Some(ImageProtocol::Kitty)
        );
        // mosh or mixed tmux clients fall back to half-blocks.
        assert_eq!(
            resolve_protocol(
                InlineImages::Auto,
                ColorDepth::Truecolor,
                GraphicsSupport::HalfblocksOnly,
                ImageProtocol::Kitty,
            ),
            Some(ImageProtocol::Halfblocks)
        );
        // 256 colours forces half-blocks.
        assert_eq!(
            resolve_protocol(
                InlineImages::Auto,
                ColorDepth::Indexed256,
                GraphicsSupport::Direct,
                ImageProtocol::Kitty,
            ),
            Some(ImageProtocol::Halfblocks)
        );
        // 16 colours disables inline images.
        assert_eq!(
            resolve_protocol(
                InlineImages::Auto,
                ColorDepth::Ansi16,
                GraphicsSupport::Direct,
                ImageProtocol::Kitty,
            ),
            None
        );
        // Explicit off disables inline images.
        assert_eq!(
            resolve_protocol(
                InlineImages::Off,
                ColorDepth::Truecolor,
                GraphicsSupport::Direct,
                ImageProtocol::Kitty,
            ),
            None
        );
    }

    #[test]
    fn forced_protocols_override_detection_colour_depth_and_terminal_hops() {
        for (mode, protocol) in [
            (InlineImages::Kitty, ImageProtocol::Kitty),
            (InlineImages::Sixel, ImageProtocol::Sixel),
            (InlineImages::Iterm2, ImageProtocol::Iterm2),
        ] {
            for colors in [
                ColorDepth::Truecolor,
                ColorDepth::Indexed256,
                ColorDepth::Ansi16,
            ] {
                for graphics in [GraphicsSupport::Direct, GraphicsSupport::HalfblocksOnly] {
                    assert_eq!(
                        resolve_protocol(mode, colors, graphics, ImageProtocol::Halfblocks),
                        Some(protocol)
                    );
                }
            }
        }
    }

    #[test]
    fn subtract_rects_splits_around_occluders_without_overlap() {
        let base = Rect::new(10, 10, 10, 6);
        assert_eq!(
            subtract_rects(base, &[Rect::new(0, 0, 5, 5)]),
            vec![base],
            "disjoint occluder leaves base whole"
        );
        assert!(
            subtract_rects(base, &[Rect::new(8, 8, 20, 20)]).is_empty(),
            "enclosing occluder leaves nothing"
        );
        let hole = Rect::new(13, 12, 4, 2);
        let parts = subtract_rects(base, &[hole]);
        assert_eq!(parts.len(), 4);
        let mut covered = 0usize;
        for y in base.top()..base.bottom() {
            for x in base.left()..base.right() {
                let p = ratatui::layout::Position::new(x, y);
                let hits = parts.iter().filter(|r| r.contains(p)).count();
                if hole.contains(p) {
                    assert_eq!(hits, 0, "({x}, {y}) inside hole");
                } else {
                    assert_eq!(hits, 1, "({x}, {y}) outside hole");
                    covered += 1;
                }
            }
        }
        assert_eq!(covered, 60 - 8);
    }
}
