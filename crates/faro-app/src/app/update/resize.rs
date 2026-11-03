//! Resizing the conversation list (D156): drag its right border with the
//! mouse, step it with keys, or pick a width in the palette. However it
//! changes, it's `[ui] sidebar_width`.

use faro_core::config::{SIDEBAR_WIDTH_MIN, key};
use ratatui::layout::{Position, Rect};

use super::settings;
use crate::app::event::Effect;
use crate::app::state::{Notice, State};
use crate::app::term_event::{MouseButton, MouseEvent, MouseEventKind};

/// Columns a key press narrows or widens the list by.
pub const STEP: u16 = 2;

/// A mouse event on the list's border, or while dragging it; `None` leaves
/// the event to the rest of the mouse handling.
pub fn on_mouse(state: &mut State, mouse: MouseEvent) -> Option<Vec<Effect>> {
    let at = Position::new(mouse.column, mouse.row);
    let border = state.geometry.sidebar_border;
    let on_edge = border.is_some_and(|area| on_edge(area, at));
    match mouse.kind {
        MouseEventKind::Down(MouseButton::Left) if on_edge => {
            state.resize.dragging = border.map(|area| area.width);
            state.resize.moved = false;
            Some(vec![])
        }
        MouseEventKind::Drag(MouseButton::Left) => {
            state.resize.dragging?;
            let area = border?;
            // The border follows the pointer.
            let wanted = mouse.column.saturating_sub(area.x).saturating_add(1);
            let width = clamp(state, wanted);
            if state.resize.dragging != Some(width) {
                state.resize.dragging = Some(width);
                state.resize.moved = true;
            }
            Some(vec![])
        }
        MouseEventKind::Up(MouseButton::Left) => {
            let width = state.resize.dragging.take()?;
            state.resize.hover = on_edge;
            if !std::mem::take(&mut state.resize.moved) {
                return Some(vec![]);
            }
            Some(set_width(state, width))
        }
        MouseEventKind::Moved => {
            state.resize.hover = on_edge;
            Some(vec![])
        }
        MouseEventKind::Down(_)
        | MouseEventKind::Up(_)
        | MouseEventKind::Drag(_)
        | MouseEventKind::ScrollUp
        | MouseEventKind::ScrollDown
        | MouseEventKind::ScrollLeft
        | MouseEventKind::ScrollRight => None,
    }
}

/// Whether `at` is on the line between the list and the chat: the list's
/// right border or the chat's left one, so it's easy to hit.
fn on_edge(area: Rect, at: Position) -> bool {
    let right = area.x.saturating_add(area.width);
    at.y >= area.y && at.y < area.bottom() && (at.x + 1 == right || at.x == right)
}

/// `Alt+,` and `Alt+.`: narrower or wider by [`STEP`], from the width drawn.
pub fn step(state: &mut State, wider: bool) -> Vec<Effect> {
    let Some(area) = state.geometry.sidebar_border else {
        state.notice = Some(Notice::info(
            "The conversation list is hidden; Alt+S shows it",
        ));
        return vec![];
    };
    let width = if wider {
        area.width.saturating_add(STEP)
    } else {
        area.width.saturating_sub(STEP)
    };
    let width = clamp(state, width);
    if width == area.width {
        let limit = if wider { "widest" } else { "narrowest" };
        state.notice = Some(Notice::info(format!(
            "The conversation list is as {limit} as it gets here"
        )));
        return vec![];
    }
    set_width(state, width)
}

/// Between the narrowest it may be and the widest this terminal allows.
fn clamp(state: &State, width: u16) -> u16 {
    width.clamp(
        SIDEBAR_WIDTH_MIN,
        state.geometry.sidebar_max.max(SIDEBAR_WIDTH_MIN),
    )
}

/// Use and save `width` (a drag, a key, a palette value).
pub fn set_width(state: &mut State, width: u16) -> Vec<Effect> {
    state.config.ui.sidebar_width = width;
    settings::save_with_notice(
        state,
        key::SIDEBAR_WIDTH,
        format!("Conversation list: {width} columns"),
    )
}
