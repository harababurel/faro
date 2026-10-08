//! The command palette's keys, mouse and commands (D44; tree, D52).

use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use faro_core::domain::{ConversationId, ServiceId};
use ratatui::layout::Position;

use super::{curation, navigation, open_conversation, settings};
use crate::app::event::Effect;
use crate::app::keymap::{Edit, PaletteKey};
use crate::app::palette::{self, ItemKind, Level, Palette, PaletteCommand, RECENT_MAX, Setting};
use crate::app::state::{Focus, Notice, State};

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
    let current = current_item(&items);
    let mut palette = Palette::new(path, items);
    if let Some(current) = current {
        palette.select(&current);
    }
    palette.note = palette::note(state, level);
    state.palette = Some(palette);
}

/// Open the palette typing a nickname for `id`, starting from the one there
/// is (D65).
pub fn open_nickname(state: &mut State, id: ServiceId) {
    state.popup = None;
    let level = Level::Nickname(id);
    let items = palette::items(state, level, &state.recent_commands);
    let query = state.nicknames.get(&id).cloned().unwrap_or_default();
    let mut palette = Palette::new(vec![Level::Root], Vec::new());
    palette.show(palette::nickname_path(id), items, &query, None);
    palette.note = palette::note(state, level);
    state.palette = Some(palette);
}

/// The item to start on in a level of values (themes, a setting's values):
/// the one in use, so opening it doesn't preview another one (D54, D59).
fn current_item(items: &[palette::Item]) -> Option<ItemKind> {
    items
        .iter()
        .find(|item| item.current == Some(true))
        .map(|item| item.kind.clone())
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
    let current = current_item(&items);
    let note = palette::note(state, level);
    if let Some(palette) = state.palette.as_mut() {
        let mut path = palette.path.clone();
        path.push(level);
        palette.show(path, items, query, current.as_ref());
        palette.note = note;
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
    let note = palette::note(state, parent);
    if let Some(palette) = state.palette.as_mut() {
        palette.show(path, items, "", left.as_ref());
        palette.note = note;
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
        if palette.selectable(index) {
            palette.selected = index;
            return choose(state);
        }
    } else if !geometry.area.contains(at) {
        state.palette = None;
    }
    vec![]
}

/// Open the selected category, flip the selected on/off setting (staying
/// open), or run the selected item and close. With nothing matching, stay
/// open.
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
        // Typing a nickname starts from the one there is.
        let query = match level {
            Level::Nickname(id) => state.nicknames.get(&id).cloned().unwrap_or_default(),
            Level::Root
            | Level::Chats
            | Level::Themes
            | Level::Settings
            | Level::Setting(_)
            | Level::Commands
            | Level::Keys
            | Level::People => String::new(),
        };
        enter(state, level, &query);
        return vec![];
    }
    if let ItemKind::Switch(setting) = kind {
        return flip(state, setting);
    }
    let typed = state
        .palette
        .as_ref()
        .map(|p| p.input.text().trim().to_owned())
        .unwrap_or_default();
    state.palette = None;
    match kind {
        ItemKind::Conversation(id) => show_conversation(state, id),
        ItemKind::Setting(choice) => settings::choose(state, choice),
        ItemKind::Theme(name) => settings::choose_theme(state, &name),
        ItemKind::Command(command) => {
            remember(state, command);
            run(state, command)
        }
        ItemKind::SetNickname(id) => set_nickname(state, id, Some(typed)),
        ItemKind::ClearNickname(id) => set_nickname(state, id, None),
        ItemKind::Key | ItemKind::Level(_) | ItemKind::Switch(_) | ItemKind::Header => vec![],
    }
}

/// Give `id` a nickname shown only in faro, or take it away (`None` or an
/// empty name) (D65).
pub fn set_nickname(state: &mut State, id: ServiceId, nickname: Option<String>) -> Vec<Effect> {
    let nickname = nickname.filter(|name| !name.is_empty());
    let before = state.person_name(id);
    match &nickname {
        Some(name) => {
            state.nicknames.insert(id, name.clone());
        }
        None => {
            if state.nicknames.remove(&id).is_none() {
                return vec![];
            }
        }
    }
    state.recount_names();
    let after = state.person_name(id);
    state.notice = Some(match &nickname {
        Some(_) => Notice::info(format!("{before} is now {after} (only in faro)")),
        None => Notice::info(format!("Nickname removed: {before} is {after} again")),
    });
    vec![Effect::SaveNickname(id, nickname)]
}

/// Flip an on/off setting from the list: applied and saved, the palette
/// left open with the query kept and the row showing its new value (D57).
fn flip(state: &mut State, setting: Setting) -> Vec<Effect> {
    let Some(choice) = palette::flipped(state, setting) else {
        return vec![];
    };
    let effects = settings::choose(state, choice);
    let level = state
        .palette
        .as_ref()
        .map_or(Level::Settings, Palette::level);
    let items = palette::items(state, level, &state.recent_commands);
    if let Some(palette) = state.palette.as_mut() {
        let (path, query, offset) = (
            palette.path.clone(),
            palette.input.text().to_owned(),
            palette.offset,
        );
        palette.show(path, items, &query, Some(&ItemKind::Switch(setting)));
        palette.offset = offset;
    }
    effects
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
