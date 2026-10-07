//! The command palette's keys, mouse and commands (M4 task 8, D44).

use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use faro_core::domain::ConversationId;
use ratatui::layout::Position;

use super::{curation, navigation, open_conversation};
use crate::app::event::Effect;
use crate::app::keymap::PaletteKey;
use crate::app::palette::{self, ItemKind, Palette, PaletteCommand, RECENT_MAX};
use crate::app::state::{Focus, State};

/// Result rows shown on the last frame (a guess before the first one).
fn rows(state: &State) -> usize {
    state
        .geometry
        .palette
        .map_or(10, |g| usize::from(g.list.height))
}

/// Open the palette with `query` (`""`, or `"?"` for the keybindings).
pub fn open(state: &mut State, query: &str) {
    state.popup = None;
    let items = palette::items(state, &state.recent_commands);
    state.palette = Some(Palette::new(items, query));
}

pub fn on_key(state: &mut State, key: PaletteKey) -> Vec<Effect> {
    let rows = rows(state);
    let Some(palette) = state.palette.as_mut() else {
        return vec![];
    };
    let page = isize::try_from(rows.saturating_sub(1).max(1)).unwrap_or(1);
    match key {
        PaletteKey::Previous => palette.step(-1, rows),
        PaletteKey::Next => palette.step(1, rows),
        PaletteKey::PageUp => palette.step(-page, rows),
        PaletteKey::PageDown => palette.step(page, rows),
        PaletteKey::Edit(edit) => {
            if navigation::edit_input(&mut palette.input, edit) {
                palette.refilter();
            }
        }
        PaletteKey::Close => state.palette = None,
        PaletteKey::Choose => return choose(state),
    }
    vec![]
}

/// Pasted text goes into the query, on one line.
pub fn paste(state: &mut State, text: &str) {
    if let Some(palette) = state.palette.as_mut() {
        let line: String = text
            .chars()
            .map(|c| if c.is_control() { ' ' } else { c })
            .collect();
        palette.input.insert_str(&line);
        palette.refilter();
    }
}

/// Wheel: move the selection. Click on a result: run it. Click outside the
/// box: close.
pub fn on_mouse(state: &mut State, mouse: MouseEvent) -> Vec<Effect> {
    let rows = rows(state);
    let (Some(palette), Some(geometry)) = (state.palette.as_mut(), state.geometry.palette) else {
        return vec![];
    };
    let at = Position::new(mouse.column, mouse.row);
    if let MouseEventKind::ScrollUp | MouseEventKind::ScrollDown = mouse.kind {
        palette.step(
            if mouse.kind == MouseEventKind::ScrollUp {
                -1
            } else {
                1
            },
            rows,
        );
        return vec![];
    }
    if mouse.kind != MouseEventKind::Down(MouseButton::Left) {
        return vec![];
    }
    if geometry.list.contains(at) {
        let index = geometry.offset + usize::from(at.y - geometry.list.y);
        if index < palette.matches.len() {
            palette.selected = index;
            return choose(state);
        }
    } else if !geometry.area.contains(at) {
        state.palette = None;
    }
    vec![]
}

/// Run the selected result. With nothing matching, stay open.
fn choose(state: &mut State) -> Vec<Effect> {
    let Some(palette) = state.palette.take() else {
        return vec![];
    };
    let Some(kind) = palette.selected_item().map(|item| item.kind) else {
        state.palette = Some(palette);
        return vec![];
    };
    match kind {
        ItemKind::Conversation(id) => show_conversation(state, id),
        ItemKind::Command(command) => {
            remember(state, command);
            run(state, command)
        }
        ItemKind::Key => vec![],
    }
}

fn remember(state: &mut State, command: PaletteCommand) {
    state.recent_commands.retain(|c| *c != command);
    state.recent_commands.insert(0, command);
    state.recent_commands.truncate(RECENT_MAX);
}

fn run(state: &mut State, command: PaletteCommand) -> Vec<Effect> {
    let target = state.open.as_ref().map(|o| o.id);
    match (command, target) {
        (PaletteCommand::Action(action), _) => navigation::on_action(state, action),
        (PaletteCommand::Conversation(action), Some(id)) => curation::perform(state, id, action),
        (PaletteCommand::MuteFor(choice), Some(id)) => curation::mute_for(state, id, choice),
        (PaletteCommand::Conversation(_) | PaletteCommand::MuteFor(_), None) => vec![],
        (PaletteCommand::HideInactive(choice), _) => curation::hide_inactive(state, choice),
        (PaletteCommand::ShowKeys, _) => {
            open(state, "?");
            vec![]
        }
        (PaletteCommand::SwitchLayout, _) => {
            state.layout = state.layout.other();
            vec![Effect::SetMessageLayout(state.layout)]
        }
    }
}

/// Open `id` in the list it's in (it may be hidden or archived) and write.
fn show_conversation(state: &mut State, id: ConversationId) -> Vec<Effect> {
    if let Some(summary) = state.summary(id) {
        state.list.view = state.section(summary).view();
    }
    state.focus = Focus::Input;
    state.sidebar.reveal = true;
    if state.open.as_ref().is_some_and(|o| o.id == id) {
        vec![]
    } else {
        open_conversation(state, id)
    }
}
