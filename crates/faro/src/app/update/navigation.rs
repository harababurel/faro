//! Acting on keys and the mouse: focus, conversation switching, scrolling and
//! simple input editing (M4 task 1).

use crate::app::term_event::{
    KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use faro_core::domain::ConversationId;
use ratatui::layout::Position;

use super::{curation, open_conversation, palette, quit, selection};
use crate::app::event::Effect;
use crate::app::input::Input;
use crate::app::keymap::{Action, Edit, WHEEL_ROWS};
use crate::app::palette::Level;
use crate::app::state::{Anchor, Focus, MessageAction, Notice, SidebarRow, State};

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
        Action::FocusInput => return focus_input(state),
        Action::Escape => return escape(state),
        Action::PageUp => scroll(state, page(state)),
        Action::PageDown => scroll(state, -page(state)),
        Action::Send => return super::sending::send(state),
        Action::Edit(Edit::Up)
            if state
                .open
                .as_ref()
                .is_some_and(|o| o.input.is_empty() && o.editing.is_none()) =>
        {
            selection::enter(state);
        }
        Action::Edit(edit) => {
            if apply_edit(state, edit) && state.open.as_ref().is_none_or(|o| o.editing.is_none()) {
                return super::presence::on_edited(state);
            }
        }
        Action::SelectOlder => selection::step(state, true),
        Action::SelectNewer => selection::step(state, false),
        Action::SelectOldest => selection::jump(state, true),
        Action::SelectNewest => selection::jump(state, false),
        Action::MessageMenu => selection::menu(state),
        Action::MessageInfo => return selection::on_selected(state, MessageAction::Info),
        Action::Reply => return selection::on_selected(state, MessageAction::Reply),
        Action::ReactPicker => return selection::on_selected(state, MessageAction::React),
        Action::Copy => return selection::on_selected(state, MessageAction::Copy),
        Action::ForwardMessage => return selection::forward_message(state),
        Action::EditMessage => return selection::edit_message(state),
        Action::DeleteMessage => return selection::delete_message(state),
        Action::OpenLink => return selection::open_link(state),
        Action::PlayVoiceNote => return selection::play_voice_note(state),
        Action::TranscribeVoiceNote => return selection::transcribe_voice_note(state),
        Action::DownloadAttachment => return selection::download_attachment(state),
        Action::OpenAttachment => return selection::open_attachment(state),
        Action::SaveAttachment => return selection::save_attachment(state),
        Action::Popup(key) => return selection::on_popup(state, key),
        Action::JumpToPinned(n) => return curation::jump_to_pinned(state, n),
        Action::Curate(curate) => return curation::on_curate(state, curate),
        Action::OpenPalette => palette::open(state, Level::Root),
        Action::SearchMessages => {
            let scope = state.open.as_ref().map(|o| o.id);
            palette::open(state, Level::Search(scope));
        }
        Action::JumpUnread => return selection::jump_unread(state),
        Action::Help => palette::open(state, Level::Keys),
        Action::Palette(key) => return palette::on_key(state, key),
        Action::Redraw => state.env.repaint = true,
        Action::ToggleBell => return super::settings::toggle_bell(state),
        Action::PasteImage => return super::sending::paste_image(state),
        Action::ViewStagedImage => super::sending::view_staged(state),
    }
    vec![]
}

/// Intercepts navigation, selection and dismissal keys when the inline `@`
/// mention autocomplete popup is active above the input bar.
pub fn on_mention_key(state: &mut State, key: KeyEvent) -> Option<Vec<Effect>> {
    if key.kind == KeyEventKind::Release {
        return None;
    }
    let candidates = state.mention_candidates();
    if candidates.is_empty() {
        return None;
    }
    let len = candidates.len();
    match (key.code, key.modifiers) {
        (KeyCode::Up, KeyModifiers::NONE) | (KeyCode::Char('p' | 'P'), KeyModifiers::CONTROL) => {
            if let Some(open) = state.open.as_mut() {
                open.mention_selected = selection::cycle(open.mention_selected, len, false);
            }
            Some(vec![])
        }
        (KeyCode::Down, KeyModifiers::NONE) | (KeyCode::Char('n' | 'N'), KeyModifiers::CONTROL) => {
            if let Some(open) = state.open.as_mut() {
                open.mention_selected = selection::cycle(open.mention_selected, len, true);
            }
            Some(vec![])
        }
        (KeyCode::Tab | KeyCode::Enter, KeyModifiers::NONE) => {
            let open = state.open.as_mut()?;
            let idx = open.mention_selected.min(len.saturating_sub(1));
            let (who, name) = candidates.get(idx)?.clone();
            let (at_pos, _) = open.active_mention_token()?;
            let cursor = open.input.cursor();
            let replacement = format!("@{name} ");
            open.input.replace_range(at_pos..cursor, &replacement);
            if !open.mentions.iter().any(|(id, n)| *id == who && *n == name) {
                open.mentions.push((who, name));
            }
            open.mention_selected = 0;
            open.mention_dismissed_at = None;
            if open.editing.is_none() {
                open.draft_edited = Some(faro_core::rt::Instant::now());
                Some(super::presence::on_edited(state))
            } else {
                Some(vec![])
            }
        }
        (KeyCode::Esc, KeyModifiers::NONE) => {
            if let Some(open) = state.open.as_mut() {
                open.mention_dismissed_at = open.active_mention_token().map(|(at, _)| at);
                open.mention_selected = 0;
            }
            Some(vec![])
        }
        _ => None,
    }
}

/// Intercepts navigation, selection and dismissal keys when the inline
/// `:shortcode:` emoji autocomplete popup is active above the input bar.
pub fn on_shortcode_key(state: &mut State, key: KeyEvent) -> Option<Vec<Effect>> {
    if key.kind == KeyEventKind::Release {
        return None;
    }
    let candidates = state.shortcode_candidates();
    if candidates.is_empty() {
        return None;
    }
    let len = candidates.len();
    match (key.code, key.modifiers) {
        (KeyCode::Up, KeyModifiers::NONE) | (KeyCode::Char('p' | 'P'), KeyModifiers::CONTROL) => {
            if let Some(open) = state.open.as_mut() {
                open.shortcode_selected = selection::cycle(open.shortcode_selected, len, false);
            }
            Some(vec![])
        }
        (KeyCode::Down, KeyModifiers::NONE) | (KeyCode::Char('n' | 'N'), KeyModifiers::CONTROL) => {
            if let Some(open) = state.open.as_mut() {
                open.shortcode_selected = selection::cycle(open.shortcode_selected, len, true);
            }
            Some(vec![])
        }
        (KeyCode::Tab | KeyCode::Enter, KeyModifiers::NONE) => {
            let open = state.open.as_mut()?;
            let idx = open.shortcode_selected.min(len.saturating_sub(1));
            let &(_, emoji) = candidates.get(idx)?;
            let (colon_pos, _) = open.active_shortcode_token()?;
            let cursor = open.input.cursor();
            open.input.replace_range(colon_pos..cursor, emoji);
            open.shortcode_selected = 0;
            open.shortcode_dismissed_at = None;
            if open.editing.is_none() {
                open.draft_edited = Some(faro_core::rt::Instant::now());
                Some(super::presence::on_edited(state))
            } else {
                Some(vec![])
            }
        }
        (KeyCode::Char(':'), KeyModifiers::NONE | KeyModifiers::SHIFT) => {
            let open = state.open.as_mut()?;
            let (colon_pos, query) = open.active_shortcode_token()?;
            let emoji = crate::ui::shortcodes::exact(query)?;
            let cursor = open.input.cursor();
            open.input.replace_range(colon_pos..cursor, emoji);
            open.shortcode_selected = 0;
            open.shortcode_dismissed_at = None;
            if open.editing.is_none() {
                open.draft_edited = Some(faro_core::rt::Instant::now());
                Some(super::presence::on_edited(state))
            } else {
                Some(vec![])
            }
        }
        (KeyCode::Esc, KeyModifiers::NONE) => {
            if let Some(open) = state.open.as_mut() {
                open.shortcode_dismissed_at = open.active_shortcode_token().map(|(colon, _)| colon);
                open.shortcode_selected = 0;
            }
            Some(vec![])
        }
        _ => None,
    }
}

/// Esc, one step at a time (D60, D77): dismiss the `@` mention or `:shortcode:`
/// popup; else cancel an in-progress message edit; else remove a staged
/// attachment; else cancel a reply being written; else dismiss the notice and
/// go back to the input; else close the open conversation (its draft kept),
/// leaving the list highlighting it.
fn escape(state: &mut State) -> Vec<Effect> {
    if state.focus == Focus::Input && !state.mention_candidates().is_empty() {
        if let Some(open) = state.open.as_mut() {
            open.mention_dismissed_at = open.active_mention_token().map(|(at, _)| at);
            open.mention_selected = 0;
        }
        return vec![];
    }
    if state.focus == Focus::Input && !state.shortcode_candidates().is_empty() {
        if let Some(open) = state.open.as_mut() {
            open.shortcode_dismissed_at = open.active_shortcode_token().map(|(colon, _)| colon);
            open.shortcode_selected = 0;
        }
        return vec![];
    }
    let Some(open) = state.open.as_mut() else {
        state.notice = None;
        return vec![];
    };
    if state.focus == Focus::Input && open.editing.take().is_some() {
        let id = open.id;
        open.mentions.clear();
        open.mention_selected = 0;
        open.mention_dismissed_at = None;
        open.shortcode_selected = 0;
        open.shortcode_dismissed_at = None;
        let restored = state
            .summary(id)
            .and_then(|s| s.state.draft.as_ref())
            .map(|d| Input::new(d.expose()))
            .unwrap_or_default();
        if let Some(open) = state.open.as_mut() {
            open.input = restored;
        }
        return vec![];
    }
    if state.focus == Focus::Input && open.staged_attachment.is_some() {
        open.staged_attachment = None;
        return vec![];
    }
    if state.focus == Focus::Input && open.reply_to.is_some() {
        open.reply_to = None;
        return vec![];
    }
    if state.notice.is_some() || state.focus != Focus::Input {
        state.notice = None;
        state.focus = Focus::Input;
        return vec![];
    }
    curation::close(state)
}

/// `Enter` in the list: with nothing open, open the highlighted
/// conversation (D60); either way, write in it.
fn focus_input(state: &mut State) -> Vec<Effect> {
    let effects = match (state.open.is_none(), state.highlighted()) {
        (true, Some(id)) => open_conversation(state, id),
        (true | false, _) => vec![],
    };
    if state.open.is_some() {
        state.focus = Focus::Input;
    }
    effects
}

/// Whether the text changed.
fn apply_edit(state: &mut State, edit: Edit) -> bool {
    let Some(open) = state.open.as_mut() else {
        return false;
    };
    let changed = edit_input(&mut open.input, edit);
    if open.active_mention_token().map(|(at, _)| at) != open.mention_dismissed_at {
        open.mention_dismissed_at = None;
    }
    if open.active_shortcode_token().map(|(colon, _)| colon) != open.shortcode_dismissed_at {
        open.shortcode_dismissed_at = None;
    }
    if changed {
        open.mention_selected = 0;
        open.shortcode_selected = 0;
        if open.editing.is_none() {
            open.draft_edited = Some(faro_core::rt::Instant::now());
        }
    }
    changed
}

/// Apply `edit` to `input` (the message input or the palette's query);
/// whether the text changed.
pub fn edit_input(input: &mut Input, edit: Edit) -> bool {
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
    changes_text && input.text().len() != before
}

/// Pasted text (bracketed paste): if it looks like a single path and a conversation
/// is open, check off-thread whether it is a regular file on disk (D77); otherwise
/// insert it into the input as typed, newlines kept.
pub fn paste(state: &mut State, text: &str) -> Vec<Effect> {
    state.focus = Focus::Input;
    if state.open.is_some()
        && let Some(path) = crate::app::files::candidate_paste_path(text)
    {
        return vec![Effect::CheckPaste {
            raw: text.to_owned(),
            path,
        }];
    }
    insert_pasted_text(state, text);
    super::presence::on_edited(state)
}

pub fn insert_pasted_text(state: &mut State, text: &str) {
    state.focus = Focus::Input;
    if let Some(open) = state.open.as_mut() {
        open.input.insert_str(text);
        open.draft_edited = Some(faro_core::rt::Instant::now());
    }
}

fn interrupt(state: &mut State) -> Vec<Effect> {
    if state.quit_armed {
        return quit(state);
    }
    {
        state.quit_armed = true;
        state.notice = Some(Notice::warning(
            "Press Ctrl+C again to quit (Ctrl+Q quits at once)",
        ));
    }
    vec![]
}

/// Move `step` places in the sidebar order (saturating at either end): the
/// open conversation switches; with nothing open, only the highlight moves
/// and nothing is read (D60).
fn step_conversation(state: &mut State, step: isize) -> Vec<Effect> {
    let ids: Vec<ConversationId> = state.visible_conversations().map(|c| c.id).collect();
    if state.open.is_none() {
        let current = state
            .highlighted()
            .and_then(|h| ids.iter().position(|id| *id == h));
        let next = current.map_or(0, |i| {
            i.saturating_add_signed(step)
                .min(ids.len().saturating_sub(1))
        });
        state.list.cursor = ids.get(next).copied();
        state.sidebar.reveal = true;
        if state.panels.left {
            state.focus = Focus::Conversations;
        }
        return vec![];
    }
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
    if state.focus == Focus::Messages {
        selection::select_initial(state);
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

pub fn on_mouse(state: &mut State, mouse: MouseEvent) -> Vec<Effect> {
    let at = Position::new(mouse.column, mouse.row);
    let geometry = &state.geometry;
    let in_sidebar = geometry.sidebar.filter(|r| r.contains(at));
    let in_history = geometry.history.filter(|r| r.contains(at));
    let in_input = geometry.input.is_some_and(|r| r.contains(at));
    let message_at = in_history.and_then(|history| {
        let row = usize::from(at.y.saturating_sub(history.y));
        geometry.history_rows.get(row).copied().flatten()
    });
    let sidebar_row = in_sidebar.and_then(|sidebar| {
        let row = usize::from(at.y.saturating_sub(sidebar.y));
        geometry.sidebar_rows.get(row).copied()
    });
    if state.popup.is_some() && mouse_affects_view(mouse) {
        // A click or scroll outside the popup closes it.
        state.popup = None;
        return vec![];
    }
    match mouse.kind {
        MouseEventKind::Down(MouseButton::Left) => {
            if in_sidebar.is_some() {
                match sidebar_row {
                    Some(SidebarRow::Conversation(id)) => {
                        // Open it and get ready to type.
                        state.focus = Focus::Input;
                        if state.open.as_ref().is_none_or(|o| o.id != id) {
                            return open_conversation(state, id);
                        }
                    }
                    Some(SidebarRow::SwitchView) => {
                        state.focus = Focus::Conversations;
                        curation::switch_view(state);
                    }
                    Some(SidebarRow::Label) | None => state.focus = Focus::Conversations,
                }
            } else if in_history.is_some() {
                if let Some(message) = message_at {
                    selection::select(state, message);
                } else {
                    state.focus = Focus::Messages;
                    selection::select_initial(state);
                }
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
            } else if in_history.is_some() || in_input {
                scroll(state, if up { WHEEL_ROWS } else { -WHEEL_ROWS });
            }
        }
        MouseEventKind::Down(MouseButton::Right) => {
            if let Some(message) = message_at {
                selection::select(state, message);
                selection::menu(state);
            } else if let Some(SidebarRow::Conversation(id)) = sidebar_row {
                state.focus = Focus::Conversations;
                curation::menu(state, id);
            }
        }
        MouseEventKind::Down(MouseButton::Middle)
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
        MouseEventKind::Down(MouseButton::Left | MouseButton::Right)
            | MouseEventKind::ScrollUp
            | MouseEventKind::ScrollDown
    )
}
