//! Views (DESIGN.md §7.1): pure functions from `State` to a frame. No I/O here.

use jiff::tz::TimeZone;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::widgets::Block;

use crate::app::state::{Focus, Geometry, State};
use crate::theme::Theme;

mod bubbles;
mod conversation;
mod emoji;
mod footer;
mod info;
pub mod logo;
mod messages;
mod palette;
mod popup;
pub mod shortcodes;
mod sidebar;
#[cfg(test)]
mod tests;
pub mod text;

/// Everything rendering needs besides the state.
pub struct View<'a> {
    pub theme: &'a Theme,
    /// For message times and date separators.
    pub tz: &'a TimeZone,
    /// For "Today"/"Yesterday".
    pub now: jiff::Timestamp,
    /// Seconds into the logo's animation (D61); `None` draws it still.
    pub logo_time: Option<f64>,
    /// Inline image protocol and cache of decoded slices (D78).
    pub images: &'a crate::app::images::Images,
}

impl View<'_> {
    /// `now` as a domain timestamp (for mute ends).
    pub fn now_timestamp(&self) -> faro_core::domain::Timestamp {
        faro_core::domain::Timestamp::from_millis(
            u64::try_from(self.now.as_millisecond()).unwrap_or(0),
        )
    }
}

const LEFT_WIDTH: u16 = 30;
const RIGHT_WIDTH: u16 = 28;
/// Below these terminal widths the sidebars hide themselves (DESIGN.md §7.1).
const LEFT_MIN_TERMINAL: u16 = 70;
const RIGHT_MIN_TERMINAL: u16 = 110;

/// Draw everything and report where things ended up (for mouse hits, page
/// sizes and the normalised scroll position; see `Geometry`).
pub fn render(frame: &mut Frame<'_>, state: &State, view: &View<'_>) -> Geometry {
    emoji::set_mode(state.env.emoji.mode);
    let mut geometry = Geometry::default();
    let area = frame.area();
    frame.render_widget(Block::new().style(view.theme.base()), area);
    let [main, footer] = Layout::vertical([Constraint::Fill(1), Constraint::Length(1)]).areas(area);
    let panes = Panes::split(main, state);
    if let Some(left) = panes.left {
        sidebar::render(frame, left, state, view, &mut geometry);
    }
    conversation::render(frame, panes.center, state, view, &mut geometry);
    if let Some(right) = panes.right {
        info::render(frame, right, state, view);
    }
    footer::render(frame, footer, state, view, &geometry);
    popup::render(frame, state, view, &mut geometry);
    palette::render(frame, state, view, &mut geometry);
    emoji::finish(frame.buffer_mut());
    geometry
}

/// Draw an inline image (or its loading/overlay placeholder) inside `clip_area`
/// at `(x, y)` relative to `clip_area`, recording missing cache keys in
/// `geometry.images_wanted` (D78).
#[expect(
    clippy::too_many_arguments,
    reason = "rendering an image box needs the frame, state, view, geometry, clip rect and box coordinates"
)]
pub(super) fn render_image_box(
    frame: &mut Frame<'_>,
    state: &State,
    view: &View<'_>,
    geometry: &mut Geometry,
    clip_area: Rect,
    x: u16,
    y: i16,
    width: u16,
    height: u16,
    path: Option<&std::path::Path>,
    overlay_blocks_graphics: bool,
) {
    use crate::app::images::{CachedImage, ImageKey, render_sliced, visible_rect};
    use ratatui_image::sliced::SignedPosition;

    let Some(vis) = visible_rect(x, y, width, height, clip_area) else {
        return;
    };
    let Some(protocol) = view.images.active_protocol() else {
        return;
    };
    let Some(path) = path else {
        draw_image_placeholder(frame, vis, "downloading…", view);
        return;
    };
    let key = ImageKey {
        path: path.to_owned(),
        width,
        height,
        protocol,
    };
    match view.images.get(&key) {
        Some(CachedImage::Ready(sliced)) => {
            if overlay_blocks_graphics && view.images.is_graphics_protocol() {
                draw_image_placeholder(frame, vis, "", view);
            } else {
                let pos = SignedPosition {
                    x: i16::try_from(x).unwrap_or(i16::MAX),
                    y,
                };
                render_sliced(sliced, pos, clip_area, frame.buffer_mut(), state.colors());
            }
        }
        Some(CachedImage::Failed) => {
            draw_image_placeholder(frame, vis, "image unavailable", view);
        }
        None => {
            if !geometry.images_wanted.contains(&key) {
                geometry.images_wanted.push(key);
            }
            draw_image_placeholder(frame, vis, "loading image…", view);
        }
    }
}

fn draw_image_placeholder(frame: &mut Frame<'_>, vis: Rect, label: &str, view: &View<'_>) {
    use ratatui::text::{Line, Span};
    use text::{truncate, width};

    let buf = frame.buffer_mut();
    let panel = view.theme.panel();
    for py in vis.top()..vis.bottom() {
        for px in vis.left()..vis.right() {
            if let Some(cell) = buf.cell_mut((px, py)) {
                cell.set_symbol(" ").set_style(panel);
            }
        }
    }
    if !label.is_empty() && vis.height > 0 && vis.width >= 4 {
        let mid_y = vis.y + vis.height / 2;
        let text = truncate(label, usize::from(vis.width.saturating_sub(2)));
        let text_w = u16::try_from(width(&text)).unwrap_or(0);
        let text_x = vis.x + vis.width.saturating_sub(text_w) / 2;
        let line = Line::from(Span::styled(
            text.into_owned(),
            panel.patch(view.theme.muted()),
        ));
        buf.set_line(
            text_x,
            mid_y,
            &line,
            vis.width.saturating_sub(text_x - vis.x),
        );
    }
}

/// `emoji::finish` for tests outside `ui` that build buffers by hand.
#[cfg(test)]
pub fn finish_for_tests(buffer: &mut ratatui::buffer::Buffer) {
    emoji::finish(buffer);
}

/// Whether `pane` has focus. Sidebar focus falls back to the input while the
/// sidebar is hidden (the reducer catches up after the frame). Call after the
/// sidebar has been drawn.
fn focused(state: &State, geometry: &Geometry, pane: Focus) -> bool {
    let focus = match state.focus {
        Focus::Conversations if geometry.sidebar.is_none() => Focus::Input,
        Focus::Conversations | Focus::Messages | Focus::Input => state.focus,
    };
    focus == pane
}

struct Panes {
    left: Option<Rect>,
    center: Rect,
    right: Option<Rect>,
}

impl Panes {
    fn split(area: Rect, state: &State) -> Self {
        let left = state.panels.left && area.width >= LEFT_MIN_TERMINAL;
        let right = state.panels.right && area.width >= RIGHT_MIN_TERMINAL;
        let [l, center, r] = Layout::horizontal([
            Constraint::Length(if left { LEFT_WIDTH } else { 0 }),
            Constraint::Fill(1),
            Constraint::Length(if right { RIGHT_WIDTH } else { 0 }),
        ])
        .areas(area);
        Self {
            left: left.then_some(l),
            center,
            right: right.then_some(r),
        }
    }
}
