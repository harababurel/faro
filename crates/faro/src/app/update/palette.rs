//! The command palette's keys, mouse and commands (D44; tree, D52).

use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use faro_core::domain::ConversationId;
use ratatui::layout::Position;

use super::{curation, navigation, open_conversation, settings};
use crate::app::event::Effect;
use crate::app::keymap::{Edit, PaletteKey};
use crate::app::palette::{self, ItemKind, Level, Palette, PaletteCommand, RECENT_MAX};
use crate::app::state::{Focus, State};

/// Result rows shown on the last frame (a guess before the first one).
fn rows(state: &State) -> usize {
    state
        .geometry
        .palette
        .map_or(10, |g| usize::from(g.list.height))
}

/// Open the palette at `level` (the root, or a category under it).
pub fn open(state: &mut State, level: Level) {
    state.popup = None;
    let path = if level == Level::Root {
        vec![Level::Root]
    } else {
        vec![Level::Root, level]
    };
    let items = palette::items(state, level, &state.recent_commands);
    state.palette = Some(Palette::new(path, items));
}

pub fn on_key(state: &mut State, key: PaletteKey) -> Vec<Effect> {
    let rows = rows(state);
    let Some(palette) = state.palette.as_mut() else {
        return vec![];
    };
    let page = isize::try_from(rows.saturating_sub(1).max(1)).unwrap_or(1);
    let at_start = palette.input.cursor() == 0;
    let at_end = palette.input.cursor() == palette.input.text().len();
    let opens_level = matches!(
        palette.selected_item().map(|item| &item.kind),
        Some(ItemKind::Level(_))
    );
    match key {
        PaletteKey::Previous => palette.step(-1, rows),
        PaletteKey::Next => palette.step(1, rows),
        PaletteKey::PageUp => palette.step(-page, rows),
        PaletteKey::PageDown => palette.step(page, rows),
        PaletteKey::Edit(Edit::Left) if at_start => up(state),
        PaletteKey::Edit(Edit::DeleteBackward) if palette.input.is_empty() => up(state),
        PaletteKey::Edit(Edit::Right) if at_end && opens_level => return choose(state),
        PaletteKey::Edit(edit) => {
            if navigation::edit_input(&mut palette.input, edit) {
                palette.refilter();
                jump_by_prefix(state);
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
        jump_by_prefix(state);
    }
}

/// At the root, a query starting with `@` `:` `>` `?` moves into that
/// category, keeping what follows as its query.
fn jump_by_prefix(state: &mut State) {
    let Some(palette) = state.palette.as_ref() else {
        return;
    };
    if palette.level() != Level::Root {
        return;
    }
    let text = palette.input.text();
    let mut chars = text.chars();
    let Some(level) = chars.next().and_then(Level::for_prefix) else {
        return;
    };
    let rest = chars.as_str().to_owned();
    enter(state, level, &rest);
}

/// Show `level` under the current one, with `query` typed.
fn enter(state: &mut State, level: Level, query: &str) {
    let items = palette::items(state, level, &state.recent_commands);
    if let Some(palette) = state.palette.as_mut() {
        let mut path = palette.path.clone();
        path.push(level);
        palette.show(path, items, query, None);
    }
}

/// Back to the level above, the item that led here selected. Nothing at the
/// root.
fn up(state: &mut State) {
    let Some(palette) = state.palette.as_ref() else {
        return;
    };
    let mut path = palette.path.clone();
    if path.len() < 2 {
        return;
    }
    let left = path.pop().map(ItemKind::Level);
    let parent = path.last().copied().unwrap_or(Level::Root);
    let items = palette::items(state, parent, &state.recent_commands);
    if let Some(palette) = state.palette.as_mut() {
        palette.show(path, items, "", left.as_ref());
    }
}

/// Wheel: move the selection. Click on a result: open or run it. Click
/// outside the box: close.
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

/// Open the selected category, or run the selected item and close. With
/// nothing matching, stay open.
fn choose(state: &mut State) -> Vec<Effect> {
    let Some(kind) = state
        .palette
        .as_ref()
        .and_then(|p| p.selected_item())
        .map(|item| item.kind.clone())
    else {
        return vec![];
    };
    if let ItemKind::Level(level) = kind {
        enter(state, level, "");
        return vec![];
    }
    state.palette = None;
    match kind {
        ItemKind::Conversation(id) => show_conversation(state, id),
        ItemKind::Setting(choice) => settings::choose(state, choice),
        ItemKind::Theme(name) => settings::choose_theme(state, &name),
        ItemKind::Command(command) => {
            remember(state, command);
            run(state, command)
        }
        ItemKind::Key | ItemKind::Level(_) => vec![],
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
            open(state, Level::Keys);
            vec![]
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
