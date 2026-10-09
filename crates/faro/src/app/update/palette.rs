//! The command palette's keys, mouse and commands (D44; tree, D52).

use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use faro_core::domain::{ConversationId, MessageRef, ServiceId};
use faro_core::store::SearchHit;
use ratatui::layout::Position;

use super::{curation, navigation, open_conversation, settings};
use crate::app::event::Effect;
use crate::app::keymap::{Edit, PaletteKey};
use crate::app::palette::{self, ItemKind, Level, Palette, PaletteCommand, RECENT_MAX, Setting};
use crate::app::state::{Focus, Notice, Selection, State};

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
    let setting_at_cursor = selected_setting(palette);
    let opens_level = match (
        palette.level(),
        palette.selected_item().map(|item| &item.kind),
    ) {
        (Level::Search(_), _) => false,
        (_, Some(ItemKind::Level(level))) => !matches!(level, Level::Setting(_)),
        (_, Some(ItemKind::EnterDir(_))) => true,
        _ => false,
    };
    match key {
        PaletteKey::Previous => palette.step(-1, rows),
        PaletteKey::Next => palette.step(1, rows),
        PaletteKey::PageUp => palette.step(-page, rows),
        PaletteKey::PageDown => palette.step(page, rows),
        PaletteKey::Edit(Edit::Left) if palette.input.is_empty() && setting_at_cursor.is_some() => {
            if let Some(setting) = setting_at_cursor {
                return cycle(state, setting, false);
            }
        }
        PaletteKey::Edit(Edit::Right)
            if palette.input.is_empty() && setting_at_cursor.is_some() =>
        {
            if let Some(setting) = setting_at_cursor {
                return cycle(state, setting, true);
            }
        }
        PaletteKey::Edit(Edit::Left) if at_start => up(state),
        PaletteKey::Edit(Edit::DeleteBackward) if palette.input.is_empty() => up(state),
        PaletteKey::Edit(Edit::Right) if at_end && opens_level => return choose(state),
        PaletteKey::Edit(edit) => {
            let (old_dir, old_file) = crate::app::system::split_file_query(palette.input.text());
            let (old_dir, old_dot) = (old_dir.to_owned(), old_file.starts_with('.'));
            if navigation::edit_input(&mut palette.input, edit) {
                if palette.level() == Level::AttachFile {
                    let (new_dir, new_file) =
                        crate::app::system::split_file_query(palette.input.text());
                    if new_dir != old_dir || new_file.starts_with('.') != old_dot {
                        let query = palette.input.text().to_owned();
                        palette.items.clear();
                        palette.refilter();
                        return vec![Effect::ListFiles(query)];
                    }
                    palette.refilter();
                    return vec![];
                }
                if let Level::Search(scope) = palette.level() {
                    return on_search_edited(state, scope);
                }
                palette.refilter();
                return jump_by_prefix(state);
            }
        }
        PaletteKey::Close => state.palette = None,
        PaletteKey::Choose => return choose(state),
    }
    vec![]
}

/// The selected setting row when the palette is at `Level::Settings`.
fn selected_setting(palette: &Palette) -> Option<Setting> {
    if palette.level() != Level::Settings {
        return None;
    }
    match palette.selected_item().map(|item| &item.kind) {
        Some(ItemKind::Level(Level::Setting(s)) | ItemKind::Switch(s)) => Some(*s),
        Some(
            ItemKind::Level(_)
            | ItemKind::Command(_)
            | ItemKind::Conversation(_)
            | ItemKind::ForwardTo { .. }
            | ItemKind::Setting(_)
            | ItemKind::Header
            | ItemKind::Theme(_)
            | ItemKind::Key
            | ItemKind::SetNickname(_)
            | ItemKind::ClearNickname(_)
            | ItemKind::EnterDir(_)
            | ItemKind::AttachFile(_)
            | ItemKind::SearchHit { .. }
            | ItemKind::ChatLink { .. }
            | ItemKind::ChatAttachment { .. },
        )
        | None => None,
    }
}

/// Pasted text goes into the query, on one line.
pub fn paste(state: &mut State, text: &str) -> Vec<Effect> {
    if let Some(palette) = state.palette.as_mut() {
        let (old_dir, old_file) = crate::app::system::split_file_query(palette.input.text());
        let (old_dir, old_dot) = (old_dir.to_owned(), old_file.starts_with('.'));
        let line: String = text
            .chars()
            .map(|c| if c.is_control() { ' ' } else { c })
            .collect();
        palette.input.insert_str(&line);
        if palette.level() == Level::AttachFile {
            let (new_dir, new_file) = crate::app::system::split_file_query(palette.input.text());
            let effects = if new_dir != old_dir || new_file.starts_with('.') != old_dot {
                palette.items.clear();
                vec![Effect::ListFiles(palette.input.text().to_owned())]
            } else {
                vec![]
            };
            palette.refilter();
            return effects;
        }
        if let Level::Search(scope) = palette.level() {
            return on_search_edited(state, scope);
        }
        palette.refilter();
        return jump_by_prefix(state);
    }
    vec![]
}

/// Populate `Level::AttachFile` when off-thread directory listing finishes (D77).
pub fn on_files_listed(state: &mut State, query: &str, entries: &[crate::app::state::FileEntry]) {
    let Some(palette) = state.palette.as_mut() else {
        return;
    };
    if palette.level() != Level::AttachFile {
        return;
    }
    let (cur_dir, cur_file) = crate::app::system::split_file_query(palette.input.text());
    let (listed_dir, listed_file) = crate::app::system::split_file_query(query);
    if cur_dir != listed_dir || cur_file.starts_with('.') != listed_file.starts_with('.') {
        return;
    }
    palette.items = palette::file_items(entries);
    palette.refilter();
}

/// Populate `Level::Search` when SQLite search results arrive (M12).
pub fn on_search_results(
    state: &mut State,
    conversation: Option<ConversationId>,
    query: &str,
    hits: Vec<SearchHit>,
) -> Vec<Effect> {
    let Some(palette) = state.palette.as_mut() else {
        return vec![];
    };
    if palette.level() != Level::Search(conversation) || palette.query().trim() != query.trim() {
        return vec![];
    }
    palette.search_hits = hits;
    let items = palette::items(state, Level::Search(conversation), &state.recent_commands);
    if let Some(palette) = state.palette.as_mut() {
        palette.items = items;
        palette.refilter();
    }
    vec![]
}

fn on_search_edited(state: &mut State, scope: Option<ConversationId>) -> Vec<Effect> {
    let query = state
        .palette
        .as_ref()
        .map(|p| p.query().to_owned())
        .unwrap_or_default();
    if query.trim().is_empty() {
        if let Some(palette) = state.palette.as_mut() {
            palette.search_hits.clear();
        }
        let items = palette::items(state, Level::Search(scope), &state.recent_commands);
        if let Some(palette) = state.palette.as_mut() {
            palette.items = items;
            palette.refilter();
        }
        return vec![];
    }
    if let Some(palette) = state.palette.as_mut() {
        palette.refilter();
    }
    vec![Effect::SearchMessages {
        conversation: scope,
        query,
    }]
}

/// At the root, a query starting with `@` `/` `:` `>` `?` moves into that
/// category, keeping what follows as its query.
fn jump_by_prefix(state: &mut State) -> Vec<Effect> {
    let Some(palette) = state.palette.as_ref() else {
        return vec![];
    };
    if palette.level() != Level::Root {
        return vec![];
    }
    let text = palette.input.text();
    let mut chars = text.chars();
    let open = state.open.as_ref().map(|o| o.id);
    let Some(level) = chars.next().and_then(|c| Level::for_prefix(c, open)) else {
        return vec![];
    };
    let rest = chars.as_str().to_owned();
    enter(state, level, &rest);
    if let Level::Search(scope) = level
        && !rest.trim().is_empty()
    {
        return vec![Effect::SearchMessages {
            conversation: scope,
            query: rest,
        }];
    }
    vec![]
}

/// Show `level` under the current one, with `query` typed.
fn enter(state: &mut State, level: Level, query: &str) {
    if let Some(palette) = state.palette.as_mut() {
        palette.search_hits.clear();
    }
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
    let left = path.pop().map(|level| match level {
        Level::Search(_) => ItemKind::Level(Level::Search(state.open.as_ref().map(|o| o.id))),
        Level::Root
        | Level::Chats
        | Level::Themes
        | Level::Settings
        | Level::Setting(_)
        | Level::Commands
        | Level::Keys
        | Level::People
        | Level::Nickname(_)
        | Level::AttachFile
        | Level::ChatLinks
        | Level::ForwardTo(_) => ItemKind::Level(level),
    });
    let parent = path.last().copied().unwrap_or(Level::Root);
    if let Some(palette) = state.palette.as_mut() {
        palette.search_hits.clear();
    }
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

/// Open the selected category, flip or apply the selected setting/theme
/// (staying in the palette), or run the selected item and close. With
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
        return choose_level(state, level);
    }
    if let ItemKind::EnterDir(dir_query) = kind {
        if let Some(palette) = state.palette.as_mut() {
            palette.input = crate::app::input::Input::new(&dir_query);
            palette.items.clear();
            palette.refilter();
        }
        return vec![Effect::ListFiles(dir_query)];
    }
    if let ItemKind::Switch(setting) = kind {
        return flip(state, setting);
    }
    if let ItemKind::Setting(choice) = kind {
        let effects = settings::choose(state, choice);
        up(state);
        return effects;
    }
    if let ItemKind::Theme(name) = kind {
        let effects = settings::choose_theme(state, &name);
        refresh_in_place(state, &ItemKind::Theme(name));
        return effects;
    }
    let typed = state
        .palette
        .as_ref()
        .map(|p| p.input.text().trim().to_owned())
        .unwrap_or_default();
    state.palette = None;
    match kind {
        ItemKind::Conversation(id) => show_conversation(state, id),
        ItemKind::ForwardTo {
            target,
            destination,
        } => forward_to(state, target, destination),
        ItemKind::SearchHit {
            conversation,
            message,
        } => jump_to_message(state, conversation, message),
        ItemKind::ChatLink { message, url } => {
            if let Some(open) = state.open.as_mut() {
                open.selected = Some(Selection {
                    message,
                    reveal: true,
                });
            }
            super::selection::open_shared_url(state, url)
        }
        ItemKind::ChatAttachment { message, index } => {
            if let Some(open) = state.open.as_mut() {
                open.selected = Some(Selection {
                    message,
                    reveal: true,
                });
            }
            state.focus = Focus::Messages;
            super::selection::activate_attachment(state, message, index)
        }
        ItemKind::Command(command) => {
            remember(state, command);
            run(state, command)
        }
        ItemKind::SetNickname(id) => set_nickname(state, id, Some(typed)),
        ItemKind::ClearNickname(id) => set_nickname(state, id, None),
        ItemKind::AttachFile(path) => vec![Effect::StageFile(path)],
        ItemKind::Key
        | ItemKind::Level(_)
        | ItemKind::EnterDir(_)
        | ItemKind::Setting(_)
        | ItemKind::Theme(_)
        | ItemKind::Switch(_)
        | ItemKind::Header => vec![],
    }
}

fn choose_level(state: &mut State, level: Level) -> Vec<Effect> {
    if let (Some(Level::Search(_)), Level::Search(scope)) =
        (state.palette.as_ref().map(Palette::level), level)
    {
        return switch_search_scope(state, scope);
    }
    // Typing a nickname starts from the one there is; AttachFile starts at ~/.
    let query = match level {
        Level::Nickname(id) => state.nicknames.get(&id).cloned().unwrap_or_default(),
        Level::AttachFile => "~/".to_owned(),
        Level::Root
        | Level::Chats
        | Level::Search(_)
        | Level::Themes
        | Level::Settings
        | Level::Setting(_)
        | Level::Commands
        | Level::Keys
        | Level::People
        | Level::ChatLinks
        | Level::ForwardTo(_) => String::new(),
    };
    enter(state, level, &query);
    if level == Level::AttachFile {
        return vec![Effect::ListFiles(query)];
    }
    vec![]
}

fn forward_to(state: &mut State, target: MessageRef, destination: ConversationId) -> Vec<Effect> {
    let Some((body_text, downloaded)) = state
        .open
        .as_ref()
        .and_then(|open| {
            open.messages
                .iter()
                .rfind(|m| m.reference() == target && !m.deleted)
        })
        .map(|message| {
            let body_text = state.readable_body(message).unwrap_or_default();
            let downloaded: Vec<faro_core::domain::AttachmentRef> = message
                .attachments
                .iter()
                .filter(|att| att.is_downloaded())
                .cloned()
                .collect();
            (body_text, downloaded)
        })
    else {
        return vec![];
    };
    if body_text.is_empty() && downloaded.is_empty() {
        return vec![];
    }
    let mut effects = show_conversation(state, destination);
    effects.extend(super::sending::forward(
        state,
        destination,
        &body_text,
        &downloaded,
    ));
    effects
}

fn switch_search_scope(state: &mut State, scope: Option<ConversationId>) -> Vec<Effect> {
    let level = Level::Search(scope);
    let query = state
        .palette
        .as_ref()
        .map(|p| p.query().to_owned())
        .unwrap_or_default();
    if let Some(palette) = state.palette.as_mut() {
        palette.search_hits.clear();
    }
    let items = palette::items(state, level, &state.recent_commands);
    let note = palette::note(state, level);
    if let Some(palette) = state.palette.as_mut() {
        let mut path = palette.path.clone();
        if matches!(path.last(), Some(Level::Search(_))) {
            path.pop();
        }
        path.push(level);
        palette.show(path, items, &query, None);
        palette.note = note;
    }
    if query.trim().is_empty() {
        vec![]
    } else {
        vec![Effect::SearchMessages {
            conversation: scope,
            query,
        }]
    }
}

fn jump_to_message(
    state: &mut State,
    conversation: ConversationId,
    message: MessageRef,
) -> Vec<Effect> {
    if state.open.as_ref().is_some_and(|o| {
        o.id == conversation && o.messages.iter().any(|m| m.reference() == message)
    }) {
        if let Some(open) = state.open.as_mut() {
            open.selected = Some(Selection {
                message,
                reveal: true,
            });
        }
        state.focus = Focus::Messages;
        return vec![];
    }
    let mut effects = if state.open.as_ref().is_some_and(|o| o.id == conversation) {
        vec![]
    } else {
        show_conversation(state, conversation)
    };
    effects.retain(|e| !matches!(e, Effect::LoadMessages(id) if *id == conversation));
    effects.push(Effect::LoadMessagesAround {
        conversation,
        target: message,
    });
    effects
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

const fn setting_row_kind(setting: Setting) -> ItemKind {
    if setting.is_switch() {
        ItemKind::Switch(setting)
    } else {
        ItemKind::Level(Level::Setting(setting))
    }
}

/// Rebuild the current palette level's items in place, keeping the query,
/// scroll offset, and selecting `select`.
fn refresh_in_place(state: &mut State, select: &ItemKind) {
    let level = state
        .palette
        .as_ref()
        .map_or(Level::Settings, Palette::level);
    let items = palette::items(state, level, &state.recent_commands);
    let note = palette::note(state, level);
    if let Some(palette) = state.palette.as_mut() {
        let (path, query, offset) = (
            palette.path.clone(),
            palette.input.text().to_owned(),
            palette.offset,
        );
        palette.show(path, items, &query, Some(select));
        palette.offset = offset;
        palette.note = note;
    }
}

/// Flip an on/off setting from the list: applied and saved, the palette
/// left open with the query kept and the row showing its new value (D57).
fn flip(state: &mut State, setting: Setting) -> Vec<Effect> {
    let Some(choice) = palette::flipped(state, setting) else {
        return vec![];
    };
    let effects = settings::choose(state, choice);
    refresh_in_place(state, &setting_row_kind(setting));
    effects
}

/// Cycle a setting's value backward or forward from `Level::Settings` (`←`/`→`).
fn cycle(state: &mut State, setting: Setting, forward: bool) -> Vec<Effect> {
    let Some(choice) = palette::cycled(state, setting, forward) else {
        return vec![];
    };
    let effects = settings::choose(state, choice);
    refresh_in_place(state, &setting_row_kind(setting));
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
        (PaletteCommand::CopyAttachmentPath, _) => super::selection::copy_attachment_path(state),
        (PaletteCommand::SearchMessages, target) => {
            open(state, Level::Search(target));
            vec![]
        }
        (PaletteCommand::JumpUnread, _) => super::selection::jump_unread(state),
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
