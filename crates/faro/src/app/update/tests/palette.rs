//! The command palette (M4 task 8, D44).

use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use faro_core::config::{BubbleStyle, ColorChoice, ConfigValue, EmojiChoice, SettingEdit, key};
use faro_core::store::ConversationState;
use ratatui::layout::Rect;

use super::*;
use crate::app::keymap::Action;
use crate::app::palette::{ItemKind, Level, PaletteCommand, Setting};
use crate::app::state::{
    ConversationAction, EmojiMode, Focus, MessageLayout, PaletteGeometry, SidebarView,
};
use crate::theme::ColorDepth;

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
    assert_eq!(titles(&state), SETTINGS_LIST.to_vec());
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

/// `Settings ›` with nothing typed: every setting under its section.
const SETTINGS_LIST: [&str; 13] = [
    "Appearance",
    "Colours",
    "Emoji",
    "Message layout",
    "Bubble style",
    "Reduce motion",
    "Notifications",
    "Bell",
    "Unread count in the window title",
    "Conversations",
    "Allowlist mode",
    "Input",
    "Mouse",
];

#[test]
fn moving_down_and_up_the_tree() {
    let mut state = loaded_state();
    update(&mut state, ctrl('p'));
    pick(&mut state, "sett");
    assert_eq!(breadcrumb(&state), "Settings");
    typed(&mut state, "colo");
    let item = state.palette.as_ref().unwrap().selected_item().unwrap();
    assert_eq!(item.title, "Colours");
    assert_eq!(item.detail, "auto (truecolor detected)");
    // → opens a setting too; it lists its values, the current one marked.
    press(&mut state, KeyCode::Right);
    assert_eq!(breadcrumb(&state), "Settings › Colours");
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
            ("auto (truecolor detected)".to_owned(), Some(true)),
            ("truecolor".to_owned(), Some(false)),
            ("256".to_owned(), Some(false)),
            ("16 (system theme)".to_owned(), Some(false)),
        ]
    );
    // → on a value doesn't run it.
    press(&mut state, KeyCode::Right);
    assert_eq!(level(&state), Level::Setting(Setting::Colors));
    // ← goes back, the setting selected; so does Backspace on an empty query.
    press(&mut state, KeyCode::Left);
    assert_eq!(breadcrumb(&state), "Settings");
    assert_eq!(
        selected(&state),
        Some(ItemKind::Level(Level::Setting(Setting::Colors)))
    );
    press(&mut state, KeyCode::Backspace);
    assert_eq!(level(&state), Level::Root);
    assert_eq!(selected(&state), Some(ItemKind::Level(Level::Settings)));
    pick(&mut state, "sett");
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
fn headers_are_skipped() {
    let mut state = loaded_state();
    palette_open(&mut state, ":");
    let at = |state: &State| state.palette.as_ref().unwrap().selected;
    // Opening starts on the first setting, under the first header.
    assert_eq!(at(&state), 1);
    assert_eq!(selected_title(&state), "Colours");
    press(&mut state, KeyCode::Up);
    assert_eq!(selected_title(&state), "Mouse", "wraps past the header");
    press(&mut state, KeyCode::Up);
    assert_eq!(selected_title(&state), "Allowlist mode", "skips Input");
    press(&mut state, KeyCode::Down);
    press(&mut state, KeyCode::Down);
    assert_eq!(selected_title(&state), "Colours", "wraps past Appearance");
    for _ in 0..5 {
        press(&mut state, KeyCode::Down);
    }
    assert_eq!(selected_title(&state), "Bell", "skips Notifications");
    // A page that lands on a header takes the setting below it.
    press(&mut state, KeyCode::PageUp);
    assert_eq!(at(&state), 1);
    // The wheel skips them too.
    state.geometry.palette = Some(PaletteGeometry {
        area: Rect::new(10, 2, 60, 18),
        list: Rect::new(11, 5, 58, 14),
        offset: 0,
    });
    let wheel = AppEvent::Terminal(Event::Mouse(MouseEvent {
        kind: MouseEventKind::ScrollUp,
        column: 20,
        row: 6,
        modifiers: KeyModifiers::NONE,
    }));
    update(&mut state, wheel);
    assert_eq!(selected_title(&state), "Mouse");
}

#[test]
fn typing_keeps_the_headers_of_groups_with_matches() {
    let mut state = loaded_state();
    palette_open(&mut state, ":bell");
    assert_eq!(titles(&state), vec!["Notifications", "Bell"]);
    assert_eq!(selected_title(&state), "Bell");
    // A section's title matches its settings.
    update(&mut state, ctrl('u'));
    typed(&mut state, "notif");
    assert_eq!(
        titles(&state),
        vec!["Notifications", "Bell", "Unread count in the window title"]
    );
    update(&mut state, ctrl('u'));
    typed(&mut state, "zzz");
    assert_eq!(titles(&state), Vec::<String>::new());
}

#[test]
fn on_off_settings_flip_in_place() {
    let mut state = loaded_state();
    palette_open(&mut state, ":bell");
    let effects = enter(&mut state);
    assert_eq!(effects, vec![saved(key::BELL, ConfigValue::Bool(false))]);
    assert!(!state.config.notifications.bell);
    // Still open, the query kept, the row showing its new value.
    assert_eq!(state.palette.as_ref().unwrap().input.text(), "bell");
    assert_eq!(selected(&state), Some(ItemKind::Switch(Setting::Bell)));
    let item = state.palette.as_ref().unwrap().selected_item().unwrap();
    assert_eq!(item.detail, "off");
    assert_eq!(
        state.notice.as_ref().map(|n| n.text.as_str()),
        Some("Do not disturb: the bell is off")
    );
    let effects = enter(&mut state);
    assert_eq!(effects, vec![saved(key::BELL, ConfigValue::Bool(true))]);
    assert!(state.config.notifications.bell);
    // → doesn't flip it (it opens nothing).
    press(&mut state, KeyCode::End);
    press(&mut state, KeyCode::Right);
    assert!(state.config.notifications.bell);
    assert_eq!(level(&state), Level::Settings);
}

#[test]
fn the_theme_list_says_how_the_colour_depth_changes_it() {
    let mut state = loaded_state();
    let note = |state: &mut State| {
        palette_open(state, "themes");
        enter(state);
        let note = state.palette.as_ref().unwrap().note.clone();
        press(state, KeyCode::Esc);
        note
    };
    state.env.colors = ColorDepth::Truecolor;
    assert_eq!(note(&mut state), None);
    state.env.colors = ColorDepth::Indexed256;
    assert_eq!(
        note(&mut state).as_deref(),
        Some("256 colours: themes are approximated (truecolor: Settings › Colours)")
    );
    // The config's override counts, not just what was detected.
    state.config.ui.colors = ColorChoice::Colors16;
    assert_eq!(
        note(&mut state).as_deref(),
        Some("16 colours: the system theme is drawn; others need 256 colours or more")
    );
    // Only at Themes.
    palette_open(&mut state, ":");
    assert_eq!(state.palette.as_ref().unwrap().note, None);
    press(&mut state, KeyCode::Esc);
    palette_open(&mut state, "themes");
    enter(&mut state);
    assert_eq!(breadcrumb(&state), "Themes");
    assert!(state.palette.as_ref().unwrap().note.is_some());
    press(&mut state, KeyCode::Left);
    assert_eq!(state.palette.as_ref().unwrap().note, None);
}

#[test]
fn a_theme_chosen_at_16_colours_is_saved_for_later() {
    let mut state = loaded_state();
    state.config.ui.colors = ColorChoice::Colors16;
    palette_open(&mut state, "themes");
    enter(&mut state);
    let effects = pick(&mut state, "nord");
    assert_eq!(state.config.theme, "nord");
    assert_eq!(
        effects,
        vec![saved(key::THEME, ConfigValue::Text("nord".into()))]
    );
    assert_eq!(
        state.notice.as_ref().map(|n| n.text.as_str()),
        Some("Theme: nord (drawn as system until 256 colours or more)")
    );
    // Choosing system itself needs no explanation.
    palette_open(&mut state, "themes");
    enter(&mut state);
    pick(&mut state, "system");
    assert_eq!(
        state.notice.as_ref().map(|n| n.text.as_str()),
        Some("Theme: system")
    );
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
    // A click on a header does nothing; on an on/off setting, flips it.
    update(&mut state, click(20, 5));
    assert_eq!(level(&state), Level::Settings);
    assert_eq!(selected_title(&state), "Colours");
    update(&mut state, click(20, 12));
    assert_eq!(selected_title(&state), "Bell");
    assert!(!state.config.notifications.bell);
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
    palette_open(&mut state, ":layout");
    enter(&mut state);
    assert_eq!(breadcrumb(&state), "Settings › Message layout");
    typed(&mut state, "irc");
    assert_eq!(selected_title(&state), "IRC");
    let effects = enter(&mut state);
    assert_eq!(state.layout, MessageLayout::Irc);
    assert_eq!(state.config.messages.layout, MessageLayout::Irc);
    assert_eq!(
        effects,
        vec![saved(key::LAYOUT, ConfigValue::Text("irc".into()))]
    );

    palette_open(&mut state, ":layout");
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
    // It opens on the value in use (D59).
    assert_eq!(selected_title(&state), "IRC");
    press(&mut state, KeyCode::Up);
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
    assert_eq!(selected(&state), Some(ItemKind::Switch(Setting::Mouse)));
    assert_eq!(
        enter(&mut state),
        vec![saved(key::MOUSE, ConfigValue::Bool(false))]
    );
    assert!(!state.config.mouse.enabled);
    press(&mut state, KeyCode::Esc);

    palette_open(&mut state, ":allow");
    let effects = enter(&mut state);
    // Climbing isn't allowed, so the next conversation opens first.
    assert_eq!(
        effects.last(),
        Some(&saved(key::ALLOWLIST, ConfigValue::Bool(true)))
    );
    assert!(state.list.allowlist);
    press(&mut state, KeyCode::Esc);

    palette_open(&mut state, ":emoji");
    enter(&mut state);
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
    palette_open(&mut state, ":emoji");
    enter(&mut state);
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
fn themes_are_a_category() {
    let mut state = loaded_state();
    update(&mut state, ctrl('p'));
    let themes = state.palette.as_ref().unwrap().items[1].clone();
    assert_eq!(themes.detail, "faro-dark", "the theme in use");
    pick(&mut state, "themes");
    assert_eq!(breadcrumb(&state), "Themes");
    // faro's own first, then OpenCode's (dark, then light), labelled.
    let titles = titles(&state);
    assert_eq!(
        titles[..5],
        ["faro-dark", "faro-dusk", "faro-light", "system", "aura"]
    );
    assert!(titles.contains(&"github-light".to_owned()));
    let items = &state.palette.as_ref().unwrap().items;
    assert_eq!(items[0].current, Some(true));
    assert_eq!(
        (items[0].detail.as_str(), items[4].detail.as_str()),
        ("", "OpenCode")
    );
    let effects = pick(&mut state, "dusk");
    assert!(state.palette.is_none());
    assert_eq!(state.config.theme, "faro-dusk");
    assert_eq!(
        effects,
        vec![saved(key::THEME, ConfigValue::Text("faro-dusk".into()))]
    );

    // The root shows the new one; the list starts on it and previews.
    update(&mut state, ctrl('p'));
    assert_eq!(state.palette.as_ref().unwrap().items[1].detail, "faro-dusk");
    pick(&mut state, "themes");
    assert_eq!(selected_title(&state), "faro-dusk");
    press(&mut state, KeyCode::Down);
    assert_eq!(state.theme_name(), "faro-light");
    // Themes are listed once: not in the settings (D60).
    press(&mut state, KeyCode::Esc);
    palette_open(&mut state, ":theme");
    assert_eq!(kinds(&state), vec![]);
}

#[test]
fn moving_through_themes_previews_them() {
    let mut state = loaded_state();
    palette_open(&mut state, "themes");
    enter(&mut state);
    // Opening the list starts on the theme in use: nothing changes yet.
    assert_eq!(selected_title(&state), "faro-dark");
    assert_eq!(state.theme_name(), "faro-dark");
    press(&mut state, KeyCode::Down);
    assert_eq!(state.theme_name(), "faro-dusk", "previewed");
    assert_eq!(state.config.theme, "faro-dark", "not chosen yet");
    typed(&mut state, "nord");
    assert_eq!(state.theme_name(), "nord", "the best match is previewed");
    // Esc puts the theme in use back, unsaved.
    press(&mut state, KeyCode::Esc);
    assert!(state.palette.is_none());
    assert_eq!(state.theme_name(), "faro-dark");

    // Going back up stops the preview too.
    palette_open(&mut state, "themes");
    enter(&mut state);
    press(&mut state, KeyCode::Down);
    update(&mut state, ctrl('u'));
    press(&mut state, KeyCode::Backspace);
    assert_eq!(level(&state), Level::Root);
    assert_eq!(state.theme_name(), "faro-dark");

    // Enter keeps it and saves it.
    pick(&mut state, "themes");
    let effects = pick(&mut state, "tokyonight");
    assert_eq!(state.config.theme, "tokyonight");
    assert_eq!(state.theme_name(), "tokyonight");
    assert_eq!(
        effects,
        vec![saved(key::THEME, ConfigValue::Text("tokyonight".into()))]
    );
    // Opening the list again starts on it.
    palette_open(&mut state, "themes");
    enter(&mut state);
    assert_eq!(selected_title(&state), "tokyonight");
}

#[test]
fn moving_through_a_visual_setting_previews_its_values() {
    let mut state = loaded_state();
    assert_eq!(state.config.messages.bubble_style, BubbleStyle::Auto);
    palette_open(&mut state, ":bubble");
    enter(&mut state);
    assert_eq!(breadcrumb(&state), "Settings › Bubble style");
    // It starts on the value in use: nothing changes yet.
    assert_eq!(selected_title(&state), "auto");
    assert_eq!(state.bubble_style(), BubbleStyle::Auto);
    press(&mut state, KeyCode::Down);
    assert_eq!(state.bubble_style(), BubbleStyle::Rounded, "previewed");
    assert_eq!(
        state.config.messages.bubble_style,
        BubbleStyle::Auto,
        "not chosen yet"
    );
    // Esc puts the value in use back, unsaved.
    press(&mut state, KeyCode::Esc);
    assert_eq!(state.bubble_style(), BubbleStyle::Auto);
    // So does going back up to the settings list.
    palette_open(&mut state, ":bubble");
    enter(&mut state);
    typed(&mut state, "bar");
    assert_eq!(state.bubble_style(), BubbleStyle::Bar);
    update(&mut state, ctrl('u'));
    press(&mut state, KeyCode::Backspace);
    assert_eq!(breadcrumb(&state), "Settings");
    assert_eq!(state.bubble_style(), BubbleStyle::Auto);
    // Enter keeps it, saves it and closes.
    enter(&mut state);
    let effects = pick(&mut state, "bar");
    assert!(state.palette.is_none());
    assert_eq!(state.config.messages.bubble_style, BubbleStyle::Bar);
    assert_eq!(state.bubble_style(), BubbleStyle::Bar);
    assert_eq!(
        effects,
        vec![saved(key::BUBBLE_STYLE, ConfigValue::Text("bar".into()))]
    );

    // The layout and the colours preview the same way.
    palette_open(&mut state, ":layout");
    enter(&mut state);
    assert_eq!(state.layout, MessageLayout::Irc, "the fixtures' layout");
    assert_eq!(state.message_layout(), MessageLayout::Irc);
    typed(&mut state, "bubbles");
    assert_eq!(state.message_layout(), MessageLayout::Bubbles);
    assert_eq!(state.layout, MessageLayout::Irc, "not chosen yet");
    press(&mut state, KeyCode::Esc);
    assert_eq!(state.message_layout(), MessageLayout::Irc);
    palette_open(&mut state, ":colours");
    enter(&mut state);
    typed(&mut state, "16");
    assert_eq!(state.colors(), ColorDepth::Ansi16);
    press(&mut state, KeyCode::Esc);
    assert_eq!(state.colors(), state.env.colors);
}

#[test]
fn emoji_values_are_not_previewed() {
    let mut state = loaded_state();
    palette_open(&mut state, ":emoji");
    enter(&mut state);
    typed(&mut state, "compatible");
    assert_eq!(selected_title(&state), "compatible");
    assert_eq!(state.palette.as_ref().unwrap().preview, None);
}
