//! The command palette (M4 task 8, D44).

use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use faro_core::config::{ConfigValue, EmojiChoice, SettingEdit, key};
use faro_core::store::ConversationState;
use ratatui::layout::Rect;

use super::*;
use crate::app::keymap::Action;
use crate::app::palette::{ItemKind, Level, PaletteCommand, Setting, SettingChoice, SettingsGroup};
use crate::app::state::{
    ConversationAction, EmojiMode, Focus, MessageLayout, PaletteGeometry, SidebarView,
};

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
        .map(|i| i.kind.clone())
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
        .map(|m| palette.items[m.item].kind.clone())
        .collect()
}

fn titles(state: &State) -> Vec<String> {
    let palette = state.palette.as_ref().unwrap();
    palette
        .matches
        .iter()
        .map(|m| palette.items[m.item].title.clone())
        .collect()
}

fn breadcrumb(state: &State) -> String {
    state.palette.as_ref().unwrap().breadcrumb()
}

fn level(state: &State) -> Level {
    state.palette.as_ref().unwrap().level()
}

fn enter(state: &mut State) -> Vec<Effect> {
    update(state, key(KeyCode::Enter, KeyModifiers::NONE))
}

fn press(state: &mut State, code: KeyCode) {
    update(state, key(code, KeyModifiers::NONE));
}

/// Type `query` in the level shown and open (or run) the best match.
fn pick(state: &mut State, query: &str) -> Vec<Effect> {
    typed(state, query);
    enter(state)
}

#[test]
fn ctrl_p_opens_the_categories_and_esc_closes() {
    let mut state = loaded_state();
    update(&mut state, ctrl('p'));
    assert_eq!(level(&state), Level::Root);
    assert_eq!(breadcrumb(&state), "Command palette");
    assert_eq!(
        kinds(&state),
        Level::CATEGORIES.map(ItemKind::Level).to_vec()
    );
    assert_eq!(
        titles(&state),
        vec!["Chats", "Themes", "Settings", "Commands", "Keys"]
    );
    update(&mut state, key(KeyCode::Esc, KeyModifiers::NONE));
    assert!(state.palette.is_none());
    // The message input wasn't touched.
    assert_eq!(state.open.as_ref().unwrap().input.text(), "");
}

#[test]
fn typing_filters_only_the_level_shown() {
    let mut state = loaded_state();
    // The root lists categories, not conversations.
    palette_open(&mut state, "ali");
    assert_eq!(kinds(&state), vec![]);
    update(&mut state, ctrl('u'));
    // Opening Chats and typing there finds her.
    pick(&mut state, "chats");
    assert_eq!(breadcrumb(&state), "Chats");
    typed(&mut state, "ali");
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
fn prefixes_jump_into_a_category() {
    let mut state = loaded_state();
    palette_open(&mut state, ">");
    assert_eq!(level(&state), Level::Commands);
    assert_eq!(state.palette.as_ref().unwrap().input.text(), "");
    assert!(
        kinds(&state)
            .iter()
            .all(|k| matches!(k, ItemKind::Command(_)))
    );
    // Backspace on an empty query goes back up.
    press(&mut state, KeyCode::Backspace);
    assert_eq!(level(&state), Level::Root);
    assert_eq!(selected(&state), Some(ItemKind::Level(Level::Commands)));
    typed(&mut state, "@");
    assert_eq!(
        kinds(&state),
        vec![
            ItemKind::Conversation(bob()),
            ItemKind::Conversation(climbing()),
            ItemKind::Conversation(alice()),
        ]
    );
    press(&mut state, KeyCode::Backspace);
    typed(&mut state, "?");
    assert!(kinds(&state).iter().all(|k| *k == ItemKind::Key));
    press(&mut state, KeyCode::Backspace);
    typed(&mut state, ":");
    assert_eq!(
        titles(&state),
        vec!["Appearance", "Notifications", "Conversations", "Mouse"]
    );
    // What follows the prefix is the category's query.
    press(&mut state, KeyCode::Backspace);
    press(&mut state, KeyCode::Backspace);
    typed(&mut state, "@bo");
    assert_eq!(state.palette.as_ref().unwrap().input.text(), "bo");
    assert_eq!(selected(&state), Some(ItemKind::Conversation(bob())));
    // Below the root a prefix is just a character.
    update(&mut state, ctrl('u'));
    typed(&mut state, ">zzz");
    assert_eq!(level(&state), Level::Chats);
    assert_eq!(kinds(&state), vec![]);
    // Enter with nothing to run keeps the palette open.
    enter(&mut state);
    assert!(state.palette.is_some());
}

#[test]
fn moving_down_and_up_the_tree() {
    let mut state = loaded_state();
    update(&mut state, ctrl('p'));
    pick(&mut state, "sett");
    pick(&mut state, "notif");
    assert_eq!(breadcrumb(&state), "Settings › Notifications");
    let item = state.palette.as_ref().unwrap().selected_item().unwrap();
    assert_eq!(item.title, "Bell");
    assert_eq!(item.detail, "on");
    // → opens a category too; a setting lists its values, the current one marked.
    press(&mut state, KeyCode::Right);
    assert_eq!(breadcrumb(&state), "Settings › Notifications › Bell");
    let values: Vec<(String, Option<bool>)> = {
        let palette = state.palette.as_ref().unwrap();
        palette
            .items
            .iter()
            .map(|i| (i.title.clone(), i.current))
            .collect()
    };
    assert_eq!(
        values,
        vec![
            ("on".to_owned(), Some(true)),
            ("off".to_owned(), Some(false))
        ]
    );
    // → on a value doesn't run it.
    press(&mut state, KeyCode::Right);
    assert_eq!(level(&state), Level::Setting(Setting::Bell));
    // ← goes back, the setting selected; so does Backspace on an empty query.
    press(&mut state, KeyCode::Left);
    assert_eq!(breadcrumb(&state), "Settings › Notifications");
    assert_eq!(
        selected(&state),
        Some(ItemKind::Level(Level::Setting(Setting::Bell)))
    );
    press(&mut state, KeyCode::Backspace);
    assert_eq!(breadcrumb(&state), "Settings");
    assert_eq!(
        selected(&state),
        Some(ItemKind::Level(Level::Group(SettingsGroup::Notifications)))
    );
    // With a query, ← and Backspace edit it.
    typed(&mut state, "ab");
    press(&mut state, KeyCode::Left);
    press(&mut state, KeyCode::Backspace);
    assert_eq!(state.palette.as_ref().unwrap().input.text(), "b");
    assert_eq!(level(&state), Level::Settings);
    // ← at the start of the query goes up; at the root, ← and Backspace
    // do nothing.
    press(&mut state, KeyCode::Delete);
    assert_eq!(state.palette.as_ref().unwrap().input.text(), "");
    press(&mut state, KeyCode::Left);
    assert_eq!(level(&state), Level::Root);
    press(&mut state, KeyCode::Left);
    press(&mut state, KeyCode::Backspace);
    assert_eq!(level(&state), Level::Root);
    assert!(state.palette.is_some());
}

#[test]
fn commands_run_and_recent_ones_come_first() {
    let mut state = loaded_state();
    palette_open(&mut state, ">conversation list");
    assert_eq!(selected_title(&state), "Hide the conversation list");
    enter(&mut state);
    assert!(!state.panels.left);
    assert_eq!(
        state.recent_commands,
        vec![PaletteCommand::Action(Action::ToggleLeftSidebar)]
    );

    // Reopened with nothing typed: the same command (now "Show…") on top.
    palette_open(&mut state, ">");
    assert_eq!(selected_title(&state), "Show the conversation list");
    enter(&mut state);
    assert!(state.panels.left);
}

#[test]
fn curating_the_open_conversation() {
    let mut state = loaded_state();
    palette_open(&mut state, ">pin");
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
    palette_open(&mut state, ">mute 8");
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
    palette_open(&mut state, ">inactive month");
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
    palette_open(&mut state, ">");
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
    palette_open(&mut state, ">hide the");
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
    assert_eq!(level(&state), Level::Keys);
    assert_eq!(breadcrumb(&state), "Keys");
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
    assert_eq!(level(&state), Level::Keys);
    update(&mut state, key(KeyCode::Esc, KeyModifiers::NONE));
    state.focus = Focus::Input;
    update(&mut state, key(KeyCode::Char('?'), KeyModifiers::SHIFT));
    assert!(state.palette.is_none());
    assert_eq!(state.open.as_ref().unwrap().input.text(), "?");

    // "Show keybindings" from the palette switches to the list.
    update(&mut state, key(KeyCode::Backspace, KeyModifiers::NONE));
    palette_open(&mut state, ">show keyb");
    enter(&mut state);
    assert_eq!(level(&state), Level::Keys);
    // Back up from there is the root.
    press(&mut state, KeyCode::Backspace);
    assert_eq!(level(&state), Level::Root);
}

#[test]
fn mouse_picks_a_result_or_closes() {
    let mut state = loaded_state();
    palette_open(&mut state, "@");
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

    // A click on a category opens it.
    update(&mut state, ctrl('p'));
    update(&mut state, click(20, 7));
    assert_eq!(level(&state), Level::Settings);
    update(&mut state, click(2, 20));
    assert!(state.palette.is_none(), "a click outside closes");
}

fn saved(path: &'static str, value: ConfigValue) -> Effect {
    Effect::SaveSetting(SettingEdit { path, value })
}

#[test]
fn the_layout_switches_from_the_palette_and_is_saved() {
    let mut state = loaded_state();
    state.layout = MessageLayout::Bubbles;
    state.config.messages.layout = MessageLayout::Bubbles;
    palette_open(&mut state, ":appear");
    enter(&mut state);
    pick(&mut state, "layout");
    assert_eq!(breadcrumb(&state), "Settings › Appearance › Message layout");
    typed(&mut state, "irc");
    assert_eq!(selected_title(&state), "IRC");
    let effects = enter(&mut state);
    assert_eq!(state.layout, MessageLayout::Irc);
    assert_eq!(state.config.messages.layout, MessageLayout::Irc);
    assert_eq!(
        effects,
        vec![saved(key::LAYOUT, ConfigValue::Text("irc".into()))]
    );

    palette_open(&mut state, ":appear");
    enter(&mut state);
    typed(&mut state, "layout");
    assert_eq!(
        state
            .palette
            .as_ref()
            .unwrap()
            .selected_item()
            .unwrap()
            .detail,
        "IRC"
    );
    enter(&mut state);
    let current: Vec<(String, Option<bool>)> = state
        .palette
        .as_ref()
        .unwrap()
        .items
        .iter()
        .map(|item| (item.title.clone(), item.current))
        .collect();
    assert_eq!(
        current,
        vec![
            ("bubbles".to_owned(), Some(false)),
            ("IRC".to_owned(), Some(true)),
        ]
    );
    let effects = enter(&mut state);
    assert_eq!(state.layout, MessageLayout::Bubbles);
    assert_eq!(
        effects,
        vec![saved(key::LAYOUT, ConfigValue::Text("bubbles".into()))]
    );
}

#[test]
fn settings_from_the_palette_apply_and_are_saved() {
    let mut state = loaded_state();
    palette_open(&mut state, ":mouse");
    enter(&mut state);
    typed(&mut state, "off");
    assert_eq!(
        selected(&state),
        Some(ItemKind::Setting(SettingChoice::Mouse(false)))
    );
    assert_eq!(
        enter(&mut state),
        vec![saved(key::MOUSE, ConfigValue::Bool(false))]
    );
    assert!(!state.config.mouse.enabled);

    palette_open(&mut state, ":conv");
    enter(&mut state);
    pick(&mut state, "allow");
    let effects = pick(&mut state, "on");
    // Climbing isn't allowed, so the next conversation opens first.
    assert_eq!(
        effects.last(),
        Some(&saved(key::ALLOWLIST, ConfigValue::Bool(true)))
    );
    assert!(state.list.allowlist);

    palette_open(&mut state, ":appear");
    enter(&mut state);
    pick(&mut state, "emoji");
    let effects = pick(&mut state, "compat");
    assert_eq!(
        effects,
        vec![saved(key::EMOJI, ConfigValue::Text("compatible".into()))]
    );
    assert_eq!(state.env.emoji.mode, EmojiMode::Compatible);
    assert!(!state.env.emoji.auto);
    assert!(state.env.repaint);

    // `--emoji` decides for this run; the choice is only saved.
    state.env.emoji.from_cli = true;
    palette_open(&mut state, ":appear");
    enter(&mut state);
    pick(&mut state, "emoji");
    assert_eq!(
        state.palette.as_ref().unwrap().items[0].detail,
        "--emoji decides for now"
    );
    let effects = pick(&mut state, "full");
    assert_eq!(
        effects,
        vec![saved(key::EMOJI, ConfigValue::Text("full".into()))]
    );
    assert_eq!(state.env.emoji.mode, EmojiMode::Compatible);
    assert_eq!(state.config.ui.emoji, EmojiChoice::Full);
}

#[test]
fn themes_are_a_category_and_under_appearance() {
    let mut state = loaded_state();
    update(&mut state, ctrl('p'));
    let themes = state.palette.as_ref().unwrap().items[1].clone();
    assert_eq!(themes.detail, "faro-dark", "the theme in use");
    pick(&mut state, "themes");
    assert_eq!(breadcrumb(&state), "Themes");
    assert_eq!(
        titles(&state),
        vec!["faro-dark", "faro-dusk", "faro-light", "system"]
    );
    assert_eq!(state.palette.as_ref().unwrap().items[0].current, Some(true));
    let effects = pick(&mut state, "dusk");
    assert!(state.palette.is_none());
    assert_eq!(state.config.theme, "faro-dusk");
    assert_eq!(
        effects,
        vec![saved(key::THEME, ConfigValue::Text("faro-dusk".into()))]
    );

    palette_open(&mut state, ":appear");
    enter(&mut state);
    let theme = state.palette.as_ref().unwrap().selected_item().unwrap();
    assert_eq!(
        (theme.title.as_str(), theme.detail.as_str()),
        ("Theme", "faro-dusk")
    );
    enter(&mut state);
    assert_eq!(breadcrumb(&state), "Settings › Appearance › Themes");
}
