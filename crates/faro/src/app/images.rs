//! Inline image protocol selection, off-thread decoding and LRU cache (D78, D79).
//!
//! Decoded and encoded [`SlicedProtocol`]s are rendering resources, not app
//! state: they live in `app::run` (passed to [`crate::ui::render`] via
//! [`crate::ui::View`]), keyed by `(path, width, height, protocol)` and capped
//! at [`CACHE_LIMIT`] entries.

use std::cell::Cell;
use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Duration;

use faro_core::config::InlineImages;
use image::imageops::FilterType;
use ratatui::buffer::Buffer;
use ratatui::layout::{Rect, Size};
use ratatui::style::Modifier;
use ratatui::widgets::Widget as _;
use ratatui_image::picker::cap_parser::QueryStdioOptions;
use ratatui_image::picker::{Picker, ProtocolType};
use ratatui_image::sliced::{SignedPosition, SlicedImage, SlicedProtocol};
use ratatui_image::{FontSize, Resize};
use tokio::sync::mpsc;

use super::event::AppEvent;
use super::state::{GraphicsSupport, State};
use crate::theme::ColorDepth;

/// Maximum width of an inline image box in the message pane, in cells (D78).
pub const MAX_IMAGE_COLS: u16 = 40;
/// Maximum height of an inline photo box in the message pane, in cells (D78).
pub const MAX_IMAGE_ROWS: u16 = 14;
/// Maximum height of a sticker box in the message pane, in cells (D79).
pub const MAX_STICKER_ROWS: u16 = 8;
/// Maximum number of decoded image slices kept in memory (D78).
pub const CACHE_LIMIT: usize = 32;

/// Assumed cell pixel size for Unicode half-blocks (`1:2` aspect ratio, D78).
pub const HALFBLOCK_FONT: FontSize = FontSize::new(10, 20);
const QUERY_TIMEOUT: Duration = Duration::from_millis(500);

/// Identifies a decoded, resized image slice in [`Images`] (D78).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageKey {
    pub path: PathBuf,
    pub width: u16,
    pub height: u16,
    pub protocol: ProtocolType,
}

impl std::hash::Hash for ImageKey {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.path.hash(state);
        self.width.hash(state);
        self.height.hash(state);
        let tag: u8 = match self.protocol {
            ProtocolType::Halfblocks => 0,
            ProtocolType::Sixel => 1,
            ProtocolType::Kitty => 2,
            ProtocolType::Iterm2 => 3,
        };
        tag.hash(state);
    }
}

/// Result of off-thread decoding and encoding, sent back as
/// [`AppEvent::ImageDecoded`].
pub struct DecodedImage {
    pub key: ImageKey,
    pub generation: u64,
    pub protocol: Option<SlicedProtocol>,
}

impl std::fmt::Debug for DecodedImage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DecodedImage")
            .field("key", &self.key)
            .field("generation", &self.generation)
            .field("ok", &self.protocol.is_some())
            .finish()
    }
}

/// What [`Images::get`] returns for a requested [`ImageKey`].
pub enum CachedImage<'a> {
    Ready(&'a SlicedProtocol),
    Failed,
}

struct Entry {
    key: ImageKey,
    value: Option<SlicedProtocol>,
    last_used: Cell<u64>,
}

/// Holds the terminal's image [`Picker`], the active protocol for the current
/// frame, and the LRU cache of decoded [`SlicedProtocol`]s (D78).
pub struct Images {
    graphics_picker: Picker,
    halfblocks_picker: Picker,
    detected_protocol: ProtocolType,
    queried_font_size: FontSize,
    active_protocol: Option<ProtocolType>,
    generation: u64,
    clock: Cell<u64>,
    entries: Vec<Entry>,
    in_flight: HashSet<ImageKey>,
}

impl Default for Images {
    fn default() -> Self {
        Self::halfblocks()
    }
}

impl Images {
    /// Query the terminal once at startup (after entering the alternate screen
    /// and before `EventStream` reads stdin) when `mode` is `Auto` and nothing
    /// in between (`mosh`, mixed tmux clients, detached tmux) blocks graphics
    /// passthrough (D78).
    pub async fn detect(mode: InlineImages, graphics: GraphicsSupport) -> Self {
        if mode == InlineImages::Auto && graphics == GraphicsSupport::Direct {
            let queried = tokio::task::spawn_blocking(|| {
                let options = QueryStdioOptions {
                    timeout: QUERY_TIMEOUT,
                    ..QueryStdioOptions::default()
                };
                (
                    Picker::from_query_stdio_with_options(options),
                    Picker::halfblocks(),
                )
            })
            .await;
            match queried {
                Ok((Ok(picker), halfblocks)) => {
                    tracing::info!(
                        protocol = ?picker.protocol_type(),
                        font_size = ?picker.font_size(),
                        "terminal image protocol detected"
                    );
                    return Self::with_pickers(picker, halfblocks);
                }
                Ok((Err(error), halfblocks)) => {
                    tracing::info!(%error, "terminal image query unsupported; using half-blocks");
                    return Self::with_pickers(halfblocks.clone(), halfblocks);
                }
                Err(error) => {
                    tracing::warn!(%error, "terminal image query task failed; using half-blocks");
                }
            }
        }
        let halfblocks = tokio::task::spawn_blocking(Picker::halfblocks)
            .await
            .unwrap_or_else(|_| Picker::halfblocks());
        Self::with_pickers(halfblocks.clone(), halfblocks)
    }

    /// Half-blocks only, without querying stdio (for tests and `"halfblocks"`/`"off"`).
    pub fn halfblocks() -> Self {
        let picker = Picker::halfblocks();
        Self::with_pickers(picker.clone(), picker)
    }

    fn with_pickers(graphics_picker: Picker, halfblocks_picker: Picker) -> Self {
        let detected_protocol = graphics_picker.protocol_type();
        let queried_font_size = graphics_picker.font_size();
        Self {
            graphics_picker,
            halfblocks_picker,
            detected_protocol,
            queried_font_size,
            active_protocol: Some(detected_protocol),
            generation: 0,
            clock: Cell::new(0),
            entries: Vec::new(),
            in_flight: HashSet::new(),
        }
    }

    /// Update the active protocol from `state` before drawing a frame; empties
    /// the cache when the protocol changes or `repaint` (`Ctrl+L`) is set so
    /// kitty re-transmits after a clear or terminal change (D78).
    pub fn prepare_frame(&mut self, state: &State) {
        let active = resolve_protocol(
            state.config.media.inline_images,
            state.colors(),
            state.env.graphics,
            self.detected_protocol,
        );
        if active != self.active_protocol || state.env.repaint {
            self.clear();
            self.active_protocol = active;
        }
    }

    /// Clear all cached slices and bump the generation so in-flight decodes
    /// from before the clear are discarded.
    pub fn clear(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.entries.clear();
        self.in_flight.clear();
    }

    /// The protocol to draw with on this frame (`None` when inline images are
    /// off or at 16 colours).
    pub fn active_protocol(&self) -> Option<ProtocolType> {
        self.active_protocol
    }

    /// Whether the active protocol is a terminal graphics protocol (`Kitty`,
    /// `Sixel` or `Iterm2`) rather than `Halfblocks`.
    pub fn is_graphics_protocol(&self) -> bool {
        matches!(
            self.active_protocol,
            Some(ProtocolType::Kitty | ProtocolType::Sixel | ProtocolType::Iterm2)
        )
    }

    /// Cell pixel size used to size image boxes for the active protocol.
    pub fn font_size(&self) -> FontSize {
        match self.active_protocol {
            Some(ProtocolType::Kitty | ProtocolType::Sixel | ProtocolType::Iterm2) => {
                self.queried_font_size
            }
            Some(ProtocolType::Halfblocks) | None => HALFBLOCK_FONT,
        }
    }

    /// Look up `key` in the cache, marking it recently used.
    pub fn get(&self, key: &ImageKey) -> Option<CachedImage<'_>> {
        let entry = self.entries.iter().find(|e| e.key == *key)?;
        let tick = self.clock.get().wrapping_add(1);
        self.clock.set(tick);
        entry.last_used.set(tick);
        Some(match &entry.value {
            Some(protocol) => CachedImage::Ready(protocol),
            None => CachedImage::Failed,
        })
    }

    /// Store a finished decode in the cache, evicting the least recently used
    /// entry when at [`CACHE_LIMIT`].
    pub fn insert(&mut self, decoded: DecodedImage) -> bool {
        if decoded.generation != self.generation {
            return false;
        }
        self.in_flight.remove(&decoded.key);
        let tick = self.clock.get().wrapping_add(1);
        self.clock.set(tick);
        if let Some(existing) = self.entries.iter_mut().find(|e| e.key == decoded.key) {
            existing.value = decoded.protocol;
            existing.last_used.set(tick);
            return true;
        }
        if self.entries.len() >= CACHE_LIMIT
            && let Some((oldest_idx, _)) = self
                .entries
                .iter()
                .enumerate()
                .min_by_key(|(_, e)| e.last_used.get())
        {
            self.entries.remove(oldest_idx);
        }
        self.entries.push(Entry {
            key: decoded.key,
            value: decoded.protocol,
            last_used: Cell::new(tick),
        });
        true
    }

    /// Spawn off-thread decode + encode tasks for any keys in `wanted` that
    /// aren't cached or already in flight.
    pub fn spawn_wanted(&mut self, wanted: &[ImageKey], results: &mpsc::UnboundedSender<AppEvent>) {
        for key in wanted {
            if self.entries.iter().any(|e| e.key == *key) || !self.in_flight.insert(key.clone()) {
                continue;
            }
            let picker = match key.protocol {
                ProtocolType::Halfblocks => self.halfblocks_picker.clone(),
                ProtocolType::Kitty | ProtocolType::Sixel | ProtocolType::Iterm2 => {
                    let mut p = self.graphics_picker.clone();
                    p.set_protocol_type(key.protocol);
                    p
                }
            };
            let key = key.clone();
            let generation = self.generation;
            let tx = results.clone();
            tokio::spawn(async move {
                let k = key.clone();
                let protocol = tokio::task::spawn_blocking(move || decode_and_slice(&picker, &k))
                    .await
                    .ok()
                    .flatten();
                let _ = tx.send(AppEvent::ImageDecoded(DecodedImage {
                    key,
                    generation,
                    protocol,
                }));
            });
        }
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.entries.len()
    }
}

/// Render `protocol` at `position` inside `clip_area`, clearing any text
/// modifiers underneath and mapping half-block colours to the 256-colour
/// palette when `colors == ColorDepth::Indexed256`.
pub fn render_sliced(
    protocol: &SlicedProtocol,
    position: SignedPosition,
    clip_area: Rect,
    buf: &mut Buffer,
    colors: ColorDepth,
) {
    let size = protocol.size();
    let Some(vis) = visible_rect(
        u16::try_from(position.x.max(0)).unwrap_or(0),
        position.y,
        size.width,
        size.height,
        clip_area,
    ) else {
        return;
    };
    for y in vis.top()..vis.bottom() {
        for x in vis.left()..vis.right() {
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.modifier = Modifier::empty();
            }
        }
    }
    SlicedImage::new(protocol, position).render(clip_area, buf);
    if matches!(protocol, SlicedProtocol::Halfblocks(_)) && colors == ColorDepth::Indexed256 {
        for y in vis.top()..vis.bottom() {
            for x in vis.left()..vis.right() {
                if let Some(cell) = buf.cell_mut((x, y)) {
                    cell.fg = crate::theme::nearest_256(cell.fg);
                    cell.bg = crate::theme::nearest_256(cell.bg);
                }
            }
        }
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

/// Active protocol for the current settings and terminal environment (D78).
pub fn resolve_protocol(
    setting: InlineImages,
    colors: ColorDepth,
    graphics: GraphicsSupport,
    detected: ProtocolType,
) -> Option<ProtocolType> {
    if setting == InlineImages::Off || colors == ColorDepth::Ansi16 {
        return None;
    }
    if setting == InlineImages::Halfblocks
        || colors == ColorDepth::Indexed256
        || graphics == GraphicsSupport::HalfblocksOnly
    {
        return Some(ProtocolType::Halfblocks);
    }
    Some(detected)
}

/// Compute the cell `(width, height)` box for an image of `img_width × img_height`
/// pixels, given the cell pixel size `font_size` and maximum cell bounds
/// `max_cols × max_rows` (D78).
pub fn box_size(
    img_width: u32,
    img_height: u32,
    font_size: FontSize,
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

/// Decode `key.path` and encode it into a [`SlicedProtocol`] of exact cell size
/// `(key.width, key.height)`. Runs on a blocking thread.
#[expect(
    clippy::disallowed_methods,
    reason = "decode_and_slice runs inside spawn_blocking off the async runtime"
)]
pub fn decode_and_slice(picker: &Picker, key: &ImageKey) -> Option<SlicedProtocol> {
    let bytes = match std::fs::read(&key.path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            std::thread::sleep(Duration::from_millis(15));
            std::fs::read(&key.path).ok()?
        }
        Err(_) => return None,
    };
    let reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    let dyn_img = reader.decode().ok()?;
    let font_size = picker.font_size();
    let px_w = u32::from(key.width.max(1)) * u32::from(font_size.width.max(1));
    let px_h = u32::from(key.height.max(1)) * u32::from(font_size.height.max(1));
    let resized = dyn_img.resize_exact(px_w, px_h, FilterType::Triangle);
    let size = Size::new(key.width.max(1), key.height.max(1));
    SlicedProtocol::new_with_resize(
        picker,
        resized,
        size,
        Resize::Fit(Some(FilterType::Triangle)),
    )
    .ok()
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
                ProtocolType::Kitty,
            ),
            Some(ProtocolType::Kitty)
        );
        // mosh or mixed tmux clients fall back to half-blocks.
        assert_eq!(
            resolve_protocol(
                InlineImages::Auto,
                ColorDepth::Truecolor,
                GraphicsSupport::HalfblocksOnly,
                ProtocolType::Kitty,
            ),
            Some(ProtocolType::Halfblocks)
        );
        // 256 colours forces half-blocks.
        assert_eq!(
            resolve_protocol(
                InlineImages::Auto,
                ColorDepth::Indexed256,
                GraphicsSupport::Direct,
                ProtocolType::Kitty,
            ),
            Some(ProtocolType::Halfblocks)
        );
        // 16 colours disables inline images.
        assert_eq!(
            resolve_protocol(
                InlineImages::Auto,
                ColorDepth::Ansi16,
                GraphicsSupport::Direct,
                ProtocolType::Kitty,
            ),
            None
        );
        // Explicit off disables inline images.
        assert_eq!(
            resolve_protocol(
                InlineImages::Off,
                ColorDepth::Truecolor,
                GraphicsSupport::Direct,
                ProtocolType::Kitty,
            ),
            None
        );
    }

    #[test]
    fn cache_evicts_least_recently_used_past_32() {
        let mut images = Images::halfblocks();
        let make_key = |i: u16| ImageKey {
            path: PathBuf::from(format!("/tmp/img-{i}.png")),
            width: i + 1,
            height: 4,
            protocol: ProtocolType::Halfblocks,
        };
        for i in 0..u16::try_from(CACHE_LIMIT).unwrap_or(32) {
            images.insert(DecodedImage {
                key: make_key(i),
                generation: 0,
                protocol: None,
            });
        }
        assert_eq!(images.len(), CACHE_LIMIT);
        // Touch key 0 so key 1 becomes the least recently used.
        assert!(images.get(&make_key(0)).is_some());
        images.insert(DecodedImage {
            key: make_key(99),
            generation: 0,
            protocol: None,
        });
        assert_eq!(images.len(), CACHE_LIMIT);
        assert!(images.get(&make_key(0)).is_some(), "touched entry kept");
        assert!(images.get(&make_key(1)).is_none(), "oldest entry evicted");
        assert!(images.get(&make_key(99)).is_some(), "new entry inserted");
    }
}
