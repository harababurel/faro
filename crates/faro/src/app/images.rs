//! Inline image protocol detection, off-thread decoding and LRU cache (D78,
//! D79): the terminal's [`ImageCache`], with ratatui-image.
//!
//! Decoded and encoded [`SlicedProtocol`]s are rendering resources, not app
//! state: they live in `app::run` (passed to [`crate::ui::render`] via
//! [`crate::ui::View`]), keyed by `(path, width, height, protocol)` and capped
//! at [`CACHE_LIMIT`] entries. Finished decodes come back on the cache's own
//! channel ([`Images::next_decoded`]), never through the reducer.

use std::cell::Cell;
use std::collections::HashSet;
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

use super::image_cache::{
    CachedImage, CellSize, DrawImage, HALFBLOCK_FONT, ImageCache, ImageKey, ImageProtocol,
    resolve_protocol, visible_rect,
};
use super::state::{GraphicsSupport, State};

pub mod probe;
use crate::theme::ColorDepth;

/// Maximum number of decoded image slices kept in memory (D78).
pub const CACHE_LIMIT: usize = 32;

const QUERY_TIMEOUT: Duration = Duration::from_millis(500);

/// The cell size ratatui-image assumes when the terminal didn't say.
const LIBRARY_FONT_SIZE: FontSize = FontSize::new(10, 20);

/// The cell size from the window's size in pixels (`TIOCGWINSZ`), if the
/// terminal (or tmux, or ssh) reports pixels at all.
fn window_font_size() -> Option<FontSize> {
    let size = crossterm::terminal::window_size().ok()?;
    if size.width == 0 || size.height == 0 || size.columns == 0 || size.rows == 0 {
        return None;
    }
    Some(FontSize::new(
        size.width / size.columns,
        size.height / size.rows,
    ))
}

/// A picker for a cell size faro measured itself (it still detects tmux).
#[expect(
    deprecated,
    reason = "ratatui-image has no other way to give a picker a known cell size"
)]
fn sized_picker(font_size: FontSize) -> Picker {
    Picker::from_fontsize(font_size)
}

/// `picker`, with the window's cell size when the terminal answered the
/// query but not its cell-size part (ratatui-image then assumes 10 × 20).
fn with_window_size(picker: Picker, window: Option<FontSize>) -> Picker {
    let FontSize { width, height } = picker.font_size();
    match window {
        Some(window) if (width, height) == (LIBRARY_FONT_SIZE.width, LIBRARY_FONT_SIZE.height) => {
            let protocol = picker.protocol_type();
            let mut measured = sized_picker(window);
            measured.set_protocol_type(protocol);
            measured
        }
        Some(_) | None => picker,
    }
}

/// Result of off-thread decoding and encoding, sent back on the cache's
/// channel.
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

/// A decoded slice, drawn with ratatui-image.
struct Sliced(SlicedProtocol);

impl DrawImage for Sliced {
    fn draw(&self, x: i16, y: i16, clip_area: Rect, buf: &mut Buffer, colors: ColorDepth) {
        render_sliced(&self.0, SignedPosition { x, y }, clip_area, buf, colors);
    }
}

struct Entry {
    key: ImageKey,
    value: Option<Sliced>,
    last_used: Cell<u64>,
}

/// Holds the terminal's image [`Picker`], the active protocol for the current
/// frame, and the LRU cache of decoded [`SlicedProtocol`]s (D78).
pub struct Images {
    graphics_picker: Picker,
    halfblocks_picker: Picker,
    detected_protocol: ImageProtocol,
    queried_font_size: CellSize,
    /// The terminal has been asked (at startup or since, D137); `auto`
    /// asks once graphics become possible if it hasn't.
    asked: bool,
    /// The tmux clients attached when it was last asked: another set means
    /// another terminal may be looking, with other cells and none of the
    /// pictures sent so far (D137).
    asked_for: Vec<String>,
    active_protocol: Option<ImageProtocol>,
    generation: u64,
    clock: Cell<u64>,
    entries: Vec<Entry>,
    in_flight: HashSet<ImageKey>,
    decoded_tx: mpsc::UnboundedSender<DecodedImage>,
    decoded_rx: mpsc::UnboundedReceiver<DecodedImage>,
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
    /// passthrough (D78), or when `mode` forces a graphics protocol: that
    /// still needs the cell's size in pixels, or every image is encoded for
    /// a guessed 10 × 20 cell and fills only part of its box (D136).
    pub async fn detect(mode: InlineImages, graphics: GraphicsSupport) -> Self {
        let forced = matches!(
            mode,
            InlineImages::Kitty | InlineImages::Sixel | InlineImages::Iterm2
        );
        if forced || (mode == InlineImages::Auto && graphics == GraphicsSupport::Direct) {
            let queried = tokio::task::spawn_blocking(move || {
                let options = QueryStdioOptions {
                    timeout: QUERY_TIMEOUT,
                    ..QueryStdioOptions::default()
                };
                let picker = Picker::from_query_stdio_with_options(options);
                let window = window_font_size();
                let picker = match picker {
                    Ok(picker) => Ok(with_window_size(picker, window)),
                    // Forced: the protocol comes from the setting; only the
                    // cell size is wanted, and the window may still say it.
                    Err(_) if forced => Ok(sized_picker(window.unwrap_or(LIBRARY_FONT_SIZE))),
                    Err(error) => Err(error),
                };
                (picker, Picker::halfblocks())
            })
            .await;
            match queried {
                Ok((Ok(picker), halfblocks)) => {
                    tracing::info!(
                        ?mode,
                        protocol = ?picker.protocol_type(),
                        font_size = ?picker.font_size(),
                        "terminal image protocol detected"
                    );
                    let mut images = Self::with_pickers(picker, halfblocks);
                    images.asked = true;
                    return images;
                }
                Ok((Err(error), halfblocks)) => {
                    tracing::info!(%error, "terminal image query unsupported; using half-blocks");
                    let mut images = Self::with_pickers(halfblocks.clone(), halfblocks);
                    images.asked = true;
                    return images;
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
        let detected_protocol = ours(graphics_picker.protocol_type());
        let FontSize { width, height } = graphics_picker.font_size();
        let (decoded_tx, decoded_rx) = mpsc::unbounded_channel();
        Self {
            graphics_picker,
            halfblocks_picker,
            detected_protocol,
            queried_font_size: CellSize { width, height },
            asked: false,
            asked_for: Vec::new(),
            active_protocol: Some(detected_protocol),
            generation: 0,
            clock: Cell::new(0),
            entries: Vec::new(),
            in_flight: HashSet::new(),
            decoded_tx,
            decoded_rx,
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

    /// The tmux clients attached when the terminal was asked at startup.
    pub fn asked_for(&mut self, clients: &[String]) {
        clients.clone_into(&mut self.asked_for);
    }

    /// Whether to ask the terminal now (D137): images are drawn with a
    /// graphics protocol (or `auto`, and graphics have become possible: no
    /// mosh or other terminal attached any more), and the terminal hasn't
    /// been asked yet (pictures replace half-blocks without a restart) or
    /// was asked with other tmux clients attached (reattaching from another
    /// screen: new cells, and the new terminal has none of the pictures).
    pub fn wants_probe(&self, state: &State) -> bool {
        let graphics = match state.config.media.inline_images {
            InlineImages::Auto => state.env.graphics == GraphicsSupport::Direct,
            InlineImages::Kitty | InlineImages::Sixel | InlineImages::Iterm2 => true,
            InlineImages::Halfblocks | InlineImages::Off => false,
        };
        graphics && (!self.asked || self.asked_for != state.env.tmux_clients)
    }

    /// The terminal answered [`probe::ask`] with `clients` attached: use
    /// what it said (protocol and cell size) and drop every picture so each
    /// is encoded and sent again.
    pub fn adopt(&mut self, answer: probe::Probe, clients: Vec<String>) {
        self.asked = true;
        self.asked_for = clients;
        self.clear();
        if !answer.answered {
            return;
        }
        self.detected_protocol = answer.protocol.unwrap_or(ImageProtocol::Halfblocks);
        let cell = answer.cell.map_or_else(
            || window_font_size().unwrap_or(LIBRARY_FONT_SIZE),
            |cell| FontSize::new(cell.width, cell.height),
        );
        let mut picker = sized_picker(cell);
        picker.set_protocol_type(theirs(self.detected_protocol));
        self.graphics_picker = picker;
        self.queried_font_size = CellSize {
            width: cell.width,
            height: cell.height,
        };
    }

    /// Clear all cached slices and bump the generation so in-flight decodes
    /// from before the clear are discarded.
    pub fn clear(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.entries.clear();
        self.in_flight.clear();
    }

    /// The startup query's result, expressed as a user-facing setting.
    pub fn detected_mode(&self) -> InlineImages {
        match self.detected_protocol {
            ImageProtocol::Kitty => InlineImages::Kitty,
            ImageProtocol::Sixel => InlineImages::Sixel,
            ImageProtocol::Iterm2 => InlineImages::Iterm2,
            ImageProtocol::Halfblocks => InlineImages::Halfblocks,
        }
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
        let value = decoded.protocol.map(Sliced);
        if let Some(existing) = self.entries.iter_mut().find(|e| e.key == decoded.key) {
            existing.value = value;
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
            value,
            last_used: Cell::new(tick),
        });
        true
    }

    /// Spawn off-thread decode + encode tasks for any keys in `wanted` that
    /// aren't cached or already in flight; results come back through
    /// [`Self::next_decoded`].
    pub fn spawn_wanted(&mut self, wanted: &[ImageKey]) {
        for key in wanted {
            if self.entries.iter().any(|e| e.key == *key) || !self.in_flight.insert(key.clone()) {
                continue;
            }
            let picker = match key.protocol {
                ImageProtocol::Halfblocks => self.halfblocks_picker.clone(),
                ImageProtocol::Kitty | ImageProtocol::Sixel | ImageProtocol::Iterm2 => {
                    let mut p = self.graphics_picker.clone();
                    p.set_protocol_type(theirs(key.protocol));
                    p
                }
            };
            let key = key.clone();
            let generation = self.generation;
            let tx = self.decoded_tx.clone();
            tokio::spawn(async move {
                let k = key.clone();
                let protocol = tokio::task::spawn_blocking(move || decode_and_slice(&picker, &k))
                    .await
                    .ok()
                    .flatten();
                let _ = tx.send(DecodedImage {
                    key,
                    generation,
                    protocol,
                });
            });
        }
    }

    /// The next finished decode (never ends: the cache keeps a sender).
    pub async fn next_decoded(&mut self) -> Option<DecodedImage> {
        self.decoded_rx.recv().await
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.entries.len()
    }
}

impl ImageCache for Images {
    fn active_protocol(&self) -> Option<ImageProtocol> {
        self.active_protocol
    }

    fn font_size(&self) -> CellSize {
        match self.active_protocol {
            Some(ImageProtocol::Kitty | ImageProtocol::Sixel | ImageProtocol::Iterm2) => {
                self.queried_font_size
            }
            Some(ImageProtocol::Halfblocks) | None => HALFBLOCK_FONT,
        }
    }

    fn get(&self, key: &ImageKey) -> Option<CachedImage<'_>> {
        let entry = self.entries.iter().find(|e| e.key == *key)?;
        let tick = self.clock.get().wrapping_add(1);
        self.clock.set(tick);
        entry.last_used.set(tick);
        Some(match &entry.value {
            Some(sliced) => CachedImage::Ready(sliced),
            None => CachedImage::Failed,
        })
    }
}

const fn ours(protocol: ProtocolType) -> ImageProtocol {
    match protocol {
        ProtocolType::Halfblocks => ImageProtocol::Halfblocks,
        ProtocolType::Sixel => ImageProtocol::Sixel,
        ProtocolType::Kitty => ImageProtocol::Kitty,
        ProtocolType::Iterm2 => ImageProtocol::Iterm2,
    }
}

const fn theirs(protocol: ImageProtocol) -> ProtocolType {
    match protocol {
        ImageProtocol::Halfblocks => ProtocolType::Halfblocks,
        ImageProtocol::Sixel => ProtocolType::Sixel,
        ImageProtocol::Kitty => ProtocolType::Kitty,
        ImageProtocol::Iterm2 => ProtocolType::Iterm2,
    }
}

/// Render `protocol` at `position` inside `clip_area`, clearing any text
/// modifiers underneath and mapping half-block colours to the 256-colour
/// palette when `colors == ColorDepth::Indexed256`.
fn render_sliced(
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
    use std::path::PathBuf;

    use super::*;

    /// A terminal that answers the query but not its cell-size part leaves
    /// ratatui-image guessing 10 × 20; the window's own size is better
    /// (D136). A size the terminal did give always wins.
    #[test]
    fn an_unanswered_cell_size_comes_from_the_window() {
        let mut guessed = Picker::halfblocks();
        guessed.set_protocol_type(ProtocolType::Kitty);
        let measured = with_window_size(guessed.clone(), Some(FontSize::new(20, 42)));
        let FontSize { width, height } = measured.font_size();
        assert_eq!((width, height), (20, 42));
        assert_eq!(measured.protocol_type(), ProtocolType::Kitty);

        let answered = with_window_size(
            sized_picker(FontSize::new(18, 36)),
            Some(FontSize::new(20, 42)),
        );
        assert_eq!(answered.font_size().width, 18, "the terminal's answer wins");

        let unknown = with_window_size(guessed, None);
        assert_eq!(unknown.font_size().width, LIBRARY_FONT_SIZE.width);
    }

    /// When faro asks the terminal again (D137): `auto` once graphics become
    /// possible, and any graphics mode when other tmux clients attach; never
    /// twice for the same clients.
    #[test]
    fn the_terminal_is_asked_again_when_graphics_or_clients_change() {
        let mut images = Images::halfblocks();
        let mut state = crate::fixtures::loaded_state();
        state.config.media.inline_images = InlineImages::Auto;
        state.env.graphics = GraphicsSupport::HalfblocksOnly;
        state.env.tmux_clients = vec!["/dev/ttys011".to_owned(), "/dev/ttys022".to_owned()];
        images.asked_for(&state.env.tmux_clients);
        assert!(
            !images.wants_probe(&state),
            "a mosh client is still attached"
        );

        state.env.graphics = GraphicsSupport::Direct;
        state.env.tmux_clients = vec!["/dev/ttys011".to_owned()];
        assert!(images.wants_probe(&state), "graphics became possible");
        let kitty = probe::Probe {
            protocol: Some(ImageProtocol::Kitty),
            cell: Some(CellSize {
                width: 18,
                height: 38,
            }),
            answered: true,
            focus: None,
        };
        images.adopt(kitty, state.env.tmux_clients.clone());
        assert!(!images.wants_probe(&state), "asked for these clients");
        assert_eq!(images.detected_mode(), InlineImages::Kitty);
        images.prepare_frame(&state);
        assert_eq!(
            images.font_size(),
            CellSize {
                width: 18,
                height: 38
            }
        );

        state.env.tmux_clients = vec!["/dev/ttys030".to_owned()];
        assert!(images.wants_probe(&state), "another terminal attached");
        images.adopt(probe::Probe::default(), state.env.tmux_clients.clone());
        assert!(!images.wants_probe(&state), "no answer: not asked again");

        state.config.media.inline_images = InlineImages::Kitty;
        state.env.graphics = GraphicsSupport::HalfblocksOnly;
        state.env.tmux_clients = vec!["/dev/ttys031".to_owned()];
        assert!(
            images.wants_probe(&state),
            "forced kitty needs the new cells too"
        );
        state.config.media.inline_images = InlineImages::Halfblocks;
        assert!(!images.wants_probe(&state), "half-blocks need nothing");
    }

    #[test]
    fn switching_protocols_live_invalidates_in_flight_decodes() {
        let mut images = Images::halfblocks();
        let mut state = crate::fixtures::loaded_state();
        state.config.media.inline_images = InlineImages::Kitty;
        images.prepare_frame(&state);
        assert_eq!(images.active_protocol(), Some(ImageProtocol::Kitty));
        let old_generation = images.generation;
        state.config.media.inline_images = InlineImages::Sixel;
        images.prepare_frame(&state);
        assert_ne!(images.generation, old_generation);
        let key = ImageKey {
            path: PathBuf::from("old.png"),
            width: 2,
            height: 2,
            protocol: ImageProtocol::Kitty,
        };
        assert!(!images.insert(DecodedImage {
            key: key.clone(),
            generation: old_generation,
            protocol: None,
        }));
        assert_eq!(images.active_protocol(), Some(ImageProtocol::Sixel));
        assert!(images.get(&key).is_none());
    }

    #[test]
    fn cache_evicts_least_recently_used_past_32() {
        let mut images = Images::halfblocks();
        let make_key = |i: u16| ImageKey {
            path: PathBuf::from(format!("/tmp/img-{i}.png")),
            width: i + 1,
            height: 4,
            protocol: ImageProtocol::Halfblocks,
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

    /// Draw `state` at 100×30 with the real image cache.
    fn draw(
        state: &State,
        theme: &crate::theme::Theme,
        images: &Images,
    ) -> (ratatui::buffer::Buffer, crate::app::state::Geometry) {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let tz = jiff::tz::TimeZone::UTC;
        let now_ms = i64::try_from(crate::fixtures::NOW_MS).unwrap();
        let view = crate::ui::View {
            theme,
            tz: &tz,
            now: jiff::Timestamp::from_millisecond(now_ms).unwrap(),
            logo_time: None,
            images,
        };
        let mut geometry = crate::app::state::Geometry::default();
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        terminal
            .draw(|frame| geometry = crate::ui::render(frame, state, &view))
            .unwrap();
        (terminal.backend().buffer().clone(), geometry)
    }

    // Moved from the ui tests (M9, D78–D79): it needs ratatui-image.
    #[test]
    fn inline_images_show_a_placeholder_then_draw_halfblocks_and_map_to_256_colours() {
        use faro_core::domain::AttachmentStatus;
        use ratatui::style::Color;
        use ratatui_image::Resize;

        use crate::theme::{ColorDepth, Theme};

        let path = PathBuf::from("/tmp/wall.jpg");
        let mut state = crate::fixtures::loaded_state();
        let att = state
            .open
            .as_mut()
            .unwrap()
            .messages
            .iter_mut()
            .find_map(|m| m.attachments.first_mut())
            .unwrap();
        att.width = Some(400);
        att.height = Some(200);
        att.status = AttachmentStatus::Downloaded(path.clone());

        let mut images = Images::halfblocks();
        images.prepare_frame(&state);
        let (buffer, geometry) = draw(&state, &Theme::default(), &images);
        let text = faro_app::ui::text::grid(&buffer);
        assert!(text.contains("loading image…"), "{text}");
        assert_eq!(geometry.images_wanted.len(), 1);
        let key = geometry.images_wanted[0].clone();
        assert_eq!(key.path, path);
        assert_eq!(
            key.protocol,
            crate::app::image_cache::ImageProtocol::Halfblocks
        );

        let dyn_img =
            image::DynamicImage::ImageRgba8(image::RgbaImage::from_fn(400, 200, |_, y| {
                if (y / 10).is_multiple_of(2) {
                    image::Rgba([200, 60, 40, 255])
                } else {
                    image::Rgba([40, 120, 200, 255])
                }
            }));
        let sliced = SlicedProtocol::new_with_resize(
            &Picker::halfblocks(),
            dyn_img,
            Size::new(key.width, key.height),
            Resize::Fit(None),
        )
        .unwrap();
        images.insert(DecodedImage {
            key,
            generation: 0,
            protocol: Some(sliced),
        });

        let (buffer, geometry) = draw(&state, &Theme::default(), &images);
        let text = faro_app::ui::text::grid(&buffer);
        assert!(!text.contains("loading image…"), "{text}");
        assert_eq!(
            geometry.images_wanted,
            [] as [crate::app::image_cache::ImageKey; 0]
        );
        assert!(
            buffer
                .content()
                .iter()
                .any(|c| (c.symbol() == "▀" || c.symbol() == "▄")
                    && matches!(c.fg, Color::Rgb(..))
                    && matches!(c.bg, Color::Rgb(..))),
            "expected truecolor halfblocks:\n{text}"
        );

        // 256-colour mode maps halfblock RGB cells to indexed colours.
        state.config.ui.colors = faro_core::config::ColorChoice::Colors256;
        images.prepare_frame(&state);
        let theme_256 = Theme::default().for_depth(ColorDepth::Indexed256);
        let (buffer_256, _) = draw(&state, &theme_256, &images);
        assert!(
            buffer_256
                .content()
                .iter()
                .all(|c| !matches!(c.fg, Color::Rgb(..)) && !matches!(c.bg, Color::Rgb(..))),
            "expected no RGB cells in 256-colour mode"
        );
        assert!(
            buffer_256
                .content()
                .iter()
                .any(|c| c.symbol() == "▀" || c.symbol() == "▄")
        );
    }
}
