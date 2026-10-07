//! Views (DESIGN.md §7.1): pure functions from `State` to a frame. No I/O here.

use jiff::tz::TimeZone;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::widgets::Block;

use crate::app::state::{Focus, Geometry, State};
use crate::theme::Theme;

mod conversation;
mod footer;
mod info;
mod messages;
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
}

const LEFT_WIDTH: u16 = 30;
const RIGHT_WIDTH: u16 = 28;
/// Below these terminal widths the sidebars hide themselves (DESIGN.md §7.1).
const LEFT_MIN_TERMINAL: u16 = 70;
const RIGHT_MIN_TERMINAL: u16 = 110;

/// Draw everything and report where things ended up (for mouse hits, page
/// sizes and the normalised scroll position; see `Geometry`).
pub fn render(frame: &mut Frame<'_>, state: &State, view: &View<'_>) -> Geometry {
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
    geometry
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
