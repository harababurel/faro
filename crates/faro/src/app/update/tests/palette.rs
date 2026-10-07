//! The command palette (M4 task 8, D44).

use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use faro_core::store::ConversationState;
use ratatui::layout::Rect;

use super::*;
use crate::app::keymap::Action;
use crate::app::palette::{ItemKind, Mode, PaletteCommand};
use crate::app::state::{ConversationAction, Focus, PaletteGeometry, SidebarView};

fn ctrl(c: char) -> AppEvent {
    key(KeyCode::Char(c), KeyModifiers::CONTROL)
}

fn palette_open(state: &mut State, query: &str) {
    update(state, ctrl('p'));
    typed(state, query);
}

fn selected(state: &State) -> Option<ItemKind> {
    state
        .palette
        .as_ref()
        .and_then(|p| p.selected_item())
        .map(|i| i.kind)
}

fn selected_title(state: &State) -> String {
    state
        .palette
        .as_ref()
        .and_then(|p| p.selected_item())
        .map(|i| i.title.clone())
        .unwrap_or_default()
}

fn kinds(state: &State) -> Vec<ItemKind> {
    let palette = state.palette.as_ref().unwrap();
    palette
        .matches
        .iter()
        .map(|m| palette.items[m.item].kind)
        .collect()
}

fn enter(state: &mut State) -> Vec<Effect> {
    update(state, key(KeyCode::Enter, KeyModifiers::NONE))
}

#[test]
fn ctrl_p_opens_on_conversations_and_esc_closes() {
    let mut state = loaded_state();
    update(&mut state, ctrl('p'));
    let palette = state.palette.as_ref().unwrap();
    assert_eq!(palette.mode(), Mode::All);
    // Nothing typed, nothing used yet: conversations first, in list order.
    assert_eq!(
        kinds(&state)[..3],
        [
            ItemKind::Conversation(bob()),
            ItemKind::Conversation(climbing()),
            ItemKind::Conversation(alice()),
        ]
    );
    update(&mut state, key(KeyCode::Esc, KeyModifiers::NONE));
    assert!(state.palette.is_none());
    // The message input wasn't touched.
    assert_eq!(state.open.as_ref().unwrap().input.text(), "");
}

#[test]
fn typing_finds_a_conversation_and_enter_opens_it() {
    let mut state = loaded_state();
    palette_open(&mut state, "ali");
    assert_eq!(selected(&state), Some(ItemKind::Conversation(alice())));
    // The query isn't typed into the message input.
    assert_eq!(state.open.as_ref().unwrap().input.text(), "");
    let effects = enter(&mut state);
    assert!(state.palette.is_none());
    assert_eq!(open_id(&state), Some(alice()));
    assert_eq!(state.focus, Focus::Input);
    assert!(effects.contains(&Effect::LoadMessages(alice())));
}

#[test]
fn prefixes_pick_what_is_searched() {
    let mut state = loaded_state();
    palette_open(&mut state, ">");
    assert!(
        kinds(&state)
            .iter()
            .all(|k| matches!(k, ItemKind::Command(_)))
    );
    update(&mut state, key(KeyCode::Backspace, KeyModifiers::NONE));
    typed(&mut state, "@");
    assert_eq!(
        kinds(&state),
        vec![
            ItemKind::Conversation(bob()),
            ItemKind::Conversation(climbing()),
            ItemKind::Conversation(alice()),
        ]
    );
    update(&mut state, ctrl('u'));
    typed(&mut state, "?");
    assert!(kinds(&state).iter().all(|k| *k == ItemKind::Key));
    update(&mut state, ctrl('u'));
    typed(&mut state, ":");
    assert_eq!(kinds(&state), vec![], "no settings before M5");
    // Enter with nothing to run keeps the palette open.
    enter(&mut state);
    assert!(state.palette.is_some());
}

#[test]
fn commands_run_and_recent_ones_come_first() {
    let mut state = loaded_state();
    palette_open(&mut state, "conversation list");
    assert_eq!(selected_title(&state), "Hide the conversation list");
    enter(&mut state);
    assert!(!state.panels.left);
    assert_eq!(
        state.recent_commands,
        vec![PaletteCommand::Action(Action::ToggleLeftSidebar)]
    );

    // Reopened with nothing typed: the same command (now "Show…") on top.
    update(&mut state, ctrl('p'));
    assert_eq!(selected_title(&state), "Show the conversation list");
    enter(&mut state);
    assert!(state.panels.left);
}

#[test]
fn curating_the_open_conversation() {
    let mut state = loaded_state();
    palette_open(&mut state, "pin");
    assert_eq!(
        selected(&state),
        Some(ItemKind::Command(PaletteCommand::Conversation(
            ConversationAction::Pin
        )))
    );
    // Its path is the conversation, its key the sidebar's.
    let item = state.palette.as_ref().unwrap().selected_item().unwrap();
    assert_eq!(item.path.as_deref(), Some("Climbing 🧗"));
    assert_eq!(item.detail, "p");
    enter(&mut state);
    assert!(state.summary(climbing()).unwrap().state.pinned.is_some());

    // Mute durations are commands of their own: no picker.
    palette_open(&mut state, "mute 8");
    assert_eq!(selected_title(&state), "Mute for 8 hours");
    enter(&mut state);
    assert!(state.popup.is_none());
    let now = sending::wall_clock().as_millis();
    let until = state
        .summary(climbing())
        .unwrap()
        .state
        .muted_until
        .unwrap();
    assert!(until.as_millis() > now + 7 * 3_600_000);
}

#[test]
fn hidden_conversations_can_be_found_and_opened() {
    let mut state = loaded_state();
    if let Some(alice) = state.conversations.iter_mut().find(|c| c.id == alice()) {
        alice.state.hidden = true;
    }
    palette_open(&mut state, "@alice");
    let item = state.palette.as_ref().unwrap().selected_item().unwrap();
    assert_eq!(item.kind, ItemKind::Conversation(alice()));
    assert_eq!(item.detail, "1 unread · hidden");
    enter(&mut state);
    // Opened where it's listed.
    assert_eq!(open_id(&state), Some(alice()));
    assert_eq!(state.list.view, SidebarView::Archive);
}

#[test]
fn hiding_inactive_conversations_in_bulk() {
    let mut state = loaded_state();
    let now = sending::wall_clock().as_millis();
    let day = 24 * 3_600_000;
    for summary in &mut state.conversations {
        let age = if summary.id == alice() {
            40 * day
        } else if summary.id == bob() {
            400 * day // pinned: kept
        } else {
            60_000
        };
        summary.last_activity = Some(Timestamp::from_millis(now - age));
    }
    palette_open(&mut state, "inactive month");
    assert_eq!(
        selected_title(&state),
        "Hide conversations inactive for 1 month"
    );
    let effects = enter(&mut state);
    let hidden = ConversationState {
        hidden: true,
        ..state.summary(alice()).unwrap().state.clone()
    };
    assert_eq!(
        effects,
        vec![Effect::SaveConversationState(alice(), Box::new(hidden))]
    );
    assert!(!state.summary(bob()).unwrap().state.hidden);
    assert_eq!(
        state.notice.as_ref().map(|n| n.text.as_str()),
        Some("Hid 1 conversation inactive for 1 month")
    );
}

#[test]
fn moving_through_results() {
    let mut state = loaded_state();
    update(&mut state, ctrl('p'));
    let count = state.palette.as_ref().unwrap().matches.len();
    let at = |state: &State| state.palette.as_ref().unwrap().selected;
    update(&mut state, key(KeyCode::Down, KeyModifiers::NONE));
    update(&mut state, ctrl('n'));
    assert_eq!(at(&state), 2);
    update(&mut state, ctrl('p'));
    assert_eq!(at(&state), 1);
    update(&mut state, key(KeyCode::Up, KeyModifiers::NONE));
    update(&mut state, key(KeyCode::Up, KeyModifiers::NONE));
    assert_eq!(at(&state), count - 1, "wraps around");
    update(&mut state, key(KeyCode::PageDown, KeyModifiers::NONE));
    assert_eq!(at(&state), count - 1, "pages stop at the end");
}

#[test]
fn the_query_is_edited_with_the_input_keys() {
    let mut state = loaded_state();
    palette_open(&mut state, "hide the");
    update(&mut state, ctrl('w'));
    assert_eq!(state.palette.as_ref().unwrap().input.text(), "hide ");
    // No new lines in a query.
    update(&mut state, key(KeyCode::Enter, KeyModifiers::ALT));
    assert_eq!(state.palette.as_ref().unwrap().input.text(), "hide ");
    update(
        &mut state,
        AppEvent::Terminal(Event::Paste("in\nactive".into())),
    );
    assert_eq!(
        state.palette.as_ref().unwrap().input.text(),
        "hide in active"
    );
    // Ctrl+Q still quits.
    update(&mut state, ctrl('q'));
    assert!(state.quit);
}

#[test]
fn keybinding_help() {
    let mut state = loaded_state();
    update(&mut state, key(KeyCode::F(1), KeyModifiers::NONE));
    assert_eq!(state.palette.as_ref().unwrap().mode(), Mode::Keys);
    typed(&mut state, "next conv");
    let item = state.palette.as_ref().unwrap().selected_item().unwrap();
    assert_eq!(item.title, "Next conversation");
    assert_eq!(item.path.as_deref(), Some("Everywhere"));
    assert_eq!(item.detail, "alt+j, alt+↓");
    // A reference: choosing closes.
    assert_eq!(enter(&mut state), vec![]);
    assert!(state.palette.is_none());

    // `?` outside the input opens it too; in the input it's typed.
    state.focus = Focus::Conversations;
    update(&mut state, key(KeyCode::Char('?'), KeyModifiers::SHIFT));
    assert_eq!(state.palette.as_ref().unwrap().mode(), Mode::Keys);
    update(&mut state, key(KeyCode::Esc, KeyModifiers::NONE));
    state.focus = Focus::Input;
    update(&mut state, key(KeyCode::Char('?'), KeyModifiers::SHIFT));
    assert!(state.palette.is_none());
    assert_eq!(state.open.as_ref().unwrap().input.text(), "?");

    // "Show keybindings" from the palette switches to the list.
    update(&mut state, key(KeyCode::Backspace, KeyModifiers::NONE));
    palette_open(&mut state, "show keyb");
    enter(&mut state);
    assert_eq!(state.palette.as_ref().unwrap().mode(), Mode::Keys);
}

#[test]
fn mouse_picks_a_result_or_closes() {
    let mut state = loaded_state();
    update(&mut state, ctrl('p'));
    state.geometry.palette = Some(PaletteGeometry {
        area: Rect::new(10, 2, 60, 12),
        list: Rect::new(11, 5, 58, 8),
        offset: 0,
    });
    let click = |column, row| {
        AppEvent::Terminal(Event::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column,
            row,
            modifiers: KeyModifiers::NONE,
        }))
    };
    // Second result row: Climbing (already open, so nothing to load).
    update(&mut state, click(20, 6));
    assert!(state.palette.is_none());
    assert_eq!(open_id(&state), Some(climbing()));

    update(&mut state, ctrl('p'));
    update(&mut state, click(2, 20));
    assert!(state.palette.is_none(), "a click outside closes");
}
