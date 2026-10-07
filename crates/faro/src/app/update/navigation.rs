//! Acting on keys and the mouse: focus, conversation switching, scrolling and
//! simple input editing (M4 task 1).

use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use faro_core::domain::ConversationId;
use ratatui::layout::Position;

use super::keys::{Action, Edit, WHEEL_ROWS};
use super::{open_conversation, quit};
use crate::app::event::Effect;
use crate::app::state::{Anchor, Focus, Notice, NoticeLevel, State};

pub fn on_action(state: &mut State, action: Action) -> Vec<Effect> {
    if action != Action::Interrupt && state.quit_armed {
        state.quit_armed = false;
        state.notice = None;
    }
    match action {
        Action::Quit => return quit(state),
        Action::Interrupt => return interrupt(state),
        Action::PreviousConversation => return step_conversation(state, -1),
        Action::NextConversation => return step_conversation(state, 1),
        Action::FirstConversation => return step_conversation(state, isize::MIN),
        Action::LastConversation => return step_conversation(state, isize::MAX),
        Action::ToggleLeftSidebar => {
            state.panels.left = !state.panels.left;
            state.sidebar.reveal = true;
            if !state.panels.left && state.focus == Focus::Conversations {
                state.focus = Focus::Input;
            }
        }
        Action::ToggleRightSidebar => state.panels.right = !state.panels.right,
        Action::FocusNext => cycle_focus(state, 1),
        Action::FocusPrevious => cycle_focus(state, -1),
        Action::FocusInput => state.focus = Focus::Input,
        Action::Escape => {
            state.notice = None;
            state.focus = Focus::Input;
        }
        Action::Scroll(rows) => scroll(state, rows),
        Action::PageUp => scroll(state, page(state)),
        Action::PageDown => scroll(state, -page(state)),
        Action::ScrollTop => scroll(state, isize::MAX),
        Action::ScrollBottom => scroll_to_bottom(state),
        Action::Send => return super::sending::send(state),
        Action::Edit(edit) => apply_edit(state, edit),
    }
    vec![]
}

fn apply_edit(state: &mut State, edit: Edit) {
    let Some(open) = state.open.as_mut() else {
        return;
    };
    let input = &mut open.input;
    let before = input.text().len();
    let changes_text = match edit {
        Edit::Insert(c) => {
            input.insert(c);
            true
        }
        Edit::NewLine => {
            input.newline();
            true
        }
        Edit::DeleteBackward => {
            input.backspace();
            true
        }
        Edit::DeleteForward => {
            input.delete();
            true
        }
        Edit::DeleteWordBackward => {
            input.delete_word_back();
            true
        }
        Edit::DeleteWordForward => {
            input.delete_word_forward();
            true
        }
        Edit::DeleteToLineStart => {
            input.delete_to_line_start();
            true
        }
        Edit::DeleteToLineEnd => {
            input.delete_to_line_end();
            true
        }
        Edit::Left => {
            input.left();
            false
        }
        Edit::Right => {
            input.right();
            false
        }
        Edit::WordLeft => {
            input.word_left();
            false
        }
        Edit::WordRight => {
            input.word_right();
            false
        }
        Edit::LineStart => {
            input.home();
            false
        }
        Edit::LineEnd => {
            input.end();
            false
        }
        Edit::Up => {
            input.up();
            false
        }
        Edit::Down => {
            input.down();
            false
        }
    };
    // Every text change alters the length (inserts add, deletes remove).
    if changes_text && input.text().len() != before {
        open.draft_edited = Some(tokio::time::Instant::now());
    }
}

/// Pasted text (bracketed paste) goes into the input as typed, newlines kept.
pub fn paste(state: &mut State, text: &str) {
    state.focus = Focus::Input;
    if let Some(open) = state.open.as_mut() {
        open.input.insert_str(text);
        open.draft_edited = Some(tokio::time::Instant::now());
    }
}

fn interrupt(state: &mut State) -> Vec<Effect> {
    if state.quit_armed {
        return quit(state);
    }
    {
        state.quit_armed = true;
        state.notice = Some(Notice {
            text: "Press Ctrl+C again to quit (Ctrl+Q quits at once)".to_owned(),
            level: NoticeLevel::Warning,
        });
    }
    vec![]
}

/// Move `step` places in the sidebar order (saturating at either end).
fn step_conversation(state: &mut State, step: isize) -> Vec<Effect> {
    let ids: Vec<ConversationId> = state.visible_conversations().map(|c| c.id).collect();
    let current = state
        .open
        .as_ref()
        .and_then(|o| ids.iter().position(|id| *id == o.id));
    let next = match current {
        Some(i) => i
            .saturating_add_signed(step)
            .min(ids.len().saturating_sub(1)),
        None => 0,
    };
    match ids.get(next) {
        Some(&id) if Some(next) != current => open_conversation(state, id),
        Some(_) | None => vec![],
    }
}

/// Tab order: conversations (if shown), messages, input.
fn cycle_focus(state: &mut State, step: isize) {
    let mut order = Vec::with_capacity(3);
    if state.geometry.sidebar.is_some() {
        order.push(Focus::Conversations);
    }
    order.extend([Focus::Messages, Focus::Input]);
    let current = order.iter().position(|f| *f == state.focus).unwrap_or(0);
    let len = order.len().cast_signed();
    let next = (current.cast_signed() + step)
        .rem_euclid(len)
        .cast_unsigned();
    if let Some(focus) = order.get(next) {
        state.focus = *focus;
    }
}

/// A page is the message area's height, keeping two rows of context.
fn page(state: &State) -> isize {
    let height = state.geometry.history.map_or(10, |r| r.height);
    isize::try_from(height.saturating_sub(2).max(1)).unwrap_or(1)
}

/// Scroll `rows` up (negative: down). The renderer clamps and normalises the
/// anchor on the next frame, and returns to following the newest message when
/// the bottom is reached.
fn scroll(state: &mut State, rows: isize) {
    let Some(open) = state.open.as_mut() else {
        return;
    };
    open.scroll = match open.scroll {
        Some(anchor) => Some(Anchor {
            offset: anchor.offset.saturating_add(rows),
            ..anchor
        }),
        None if rows > 0 => open.messages.last().map(|last| Anchor {
            message: last.reference(),
            offset: rows,
        }),
        None => None,
    };
}

fn scroll_to_bottom(state: &mut State) {
    if let Some(open) = state.open.as_mut() {
        open.scroll = None;
        open.unseen = 0;
    }
}

pub fn on_mouse(state: &mut State, mouse: MouseEvent) -> Vec<Effect> {
    let at = Position::new(mouse.column, mouse.row);
    let geometry = &state.geometry;
    let in_sidebar = geometry.sidebar.filter(|r| r.contains(at));
    let in_history = geometry.history.is_some_and(|r| r.contains(at));
    let in_input = geometry.input.is_some_and(|r| r.contains(at));
    match mouse.kind {
        MouseEventKind::Down(MouseButton::Left) => {
            if let Some(sidebar) = in_sidebar {
                let row = usize::from(at.y.saturating_sub(sidebar.y));
                let clicked = geometry.sidebar_rows.get(row).copied().flatten();
                if let Some(id) = clicked {
                    // Open it and get ready to type.
                    state.focus = Focus::Input;
                    if state.open.as_ref().is_none_or(|o| o.id != id) {
                        return open_conversation(state, id);
                    }
                } else {
                    state.focus = Focus::Conversations;
                }
            } else if in_history {
                state.focus = Focus::Messages;
            } else if in_input {
                state.focus = Focus::Input;
            }
        }
        MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
            let up = mouse.kind == MouseEventKind::ScrollUp;
            if in_sidebar.is_some() {
                let offset = state.sidebar.offset;
                state.sidebar.offset = if up {
                    offset.saturating_sub(WHEEL_ROWS.cast_unsigned())
                } else {
                    offset.saturating_add(WHEEL_ROWS.cast_unsigned())
                };
                state.sidebar.reveal = false;
            } else if in_history || in_input {
                scroll(state, if up { WHEEL_ROWS } else { -WHEEL_ROWS });
            }
        }
        MouseEventKind::Down(MouseButton::Right | MouseButton::Middle)
        | MouseEventKind::Up(_)
        | MouseEventKind::Drag(_)
        | MouseEventKind::Moved
        | MouseEventKind::ScrollLeft
        | MouseEventKind::ScrollRight => {}
    }
    vec![]
}

/// Whether a mouse event can change what's on screen (moves and drags can't
/// yet, so they don't cost a frame).
pub fn mouse_affects_view(mouse: MouseEvent) -> bool {
    matches!(
        mouse.kind,
        MouseEventKind::Down(MouseButton::Left)
            | MouseEventKind::ScrollUp
            | MouseEventKind::ScrollDown
    )
}
