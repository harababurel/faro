#![expect(
    clippy::disallowed_methods,
    reason = "tests set up files synchronously; #[tokio::test] expands to block_on"
)]

use std::os::unix::fs::PermissionsExt as _;
use std::time::Duration;

use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use faro_core::config::{MessageLayout, key};

use super::*;
use crate::app::event::Effect;
use crate::app::keymap::{Context, Keymap};
use crate::app::state::State;
use crate::app::update::update;
use crate::fixtures::loaded_state;

fn mode(path: &Path) -> u32 {
    std::fs::metadata(path).unwrap().permissions().mode() & 0o777
}

/// The template with every commented setting and binding uncommented
/// (comment lines that aren't valid TOML on their own stay comments).
fn uncommented(text: &str) -> String {
    let valid = |line: &str| {
        let parsed = core::parse(line, &Config::default());
        !parsed.problems.iter().any(|p| p.text.starts_with("line "))
    };
    text.lines()
        .map(|line| match line.strip_prefix("# ") {
            Some(rest) if rest.contains(" = ") && valid(rest) => rest.to_owned(),
            Some(_) | None => line.to_owned(),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn the_template_lists_every_default_binding_and_changes_nothing_when_uncommented() {
    let text = template();
    let parsed = core::parse(&text, &Config::default());
    assert_eq!(parsed.problems, vec![]);
    assert_eq!(parsed.config, Config::default());
    assert!(text.contains("# \"ctrl+p\" = \"palette.open\""), "{text}");

    let parsed = core::parse(&uncommented(&text), &Config::default());
    assert_eq!(parsed.problems, vec![]);
    assert_ne!(parsed.config.keybindings, vec![]);
    let mut keymap = Keymap::default();
    for b in &parsed.config.keybindings {
        keymap.apply(&b.context, &b.key, &b.action).unwrap();
    }
    let defaults = Keymap::default();
    for context in Context::ALL {
        for action in crate::app::keymap::Action::bindable() {
            let mut a = keymap.keys_in(context, action);
            let mut b = defaults.keys_in(context, action);
            a.sort_by_key(|k| k.label());
            b.sort_by_key(|k| k.label());
            assert_eq!(a, b, "{context:?} {action:?}");
        }
    }
}

#[test]
fn first_run_writes_the_template_privately() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("new").join("config.toml");
    let (config, problems) = load(&path);
    assert_eq!((config, problems), (Config::default(), vec![]));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), template());
    assert_eq!(mode(&path), paths::FILE_MODE);
    assert_eq!(mode(path.parent().unwrap()), paths::DIR_MODE);

    // The second run reads it.
    std::fs::write(&path, "[messages]\nlayout = \"irc\"\nx = 1\n").unwrap();
    let (config, problems) = load(&path);
    assert_eq!(config.messages.layout, MessageLayout::Irc);
    assert_eq!(problems.len(), 1);
}

/// M5 acceptance: a palette toggle updates the file without losing comments,
/// and reading the file back (as the watcher does) changes nothing.
#[tokio::test]
async fn toggling_the_layout_from_the_palette_keeps_the_file_s_comments() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("config.toml");
    let original = format!("# my own note\n{}", template());
    std::fs::write(&path, &original).unwrap();

    let mut state: State = loaded_state();
    state.layout = MessageLayout::Bubbles;
    state.config.messages.layout = MessageLayout::Bubbles;
    update(&mut state, AppEvent::Terminal(ctrl('p')));
    // Settings › Appearance › Message layout › IRC.
    for c in ":appear\rlayout\rirc".chars() {
        let code = if c == '\r' {
            KeyCode::Enter
        } else {
            KeyCode::Char(c)
        };
        update(
            &mut state,
            AppEvent::Terminal(Event::Key(KeyEvent::new(code, KeyModifiers::NONE))),
        );
    }
    let effects = update(
        &mut state,
        AppEvent::Terminal(Event::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        ))),
    );
    assert_eq!(state.layout, MessageLayout::Irc);
    let [Effect::SaveSetting(edit)] = effects.as_slice() else {
        panic!("expected one SaveSetting, got {effects:?}");
    };
    assert_eq!(edit.path, key::LAYOUT);
    save(&path, edit).await.unwrap();

    let edited = std::fs::read_to_string(&path).unwrap();
    let changed: Vec<(&str, &str)> = original
        .lines()
        .zip(edited.lines())
        .filter(|(a, b)| a != b)
        .collect();
    assert_eq!(
        changed,
        vec![("# layout = \"bubbles\"", "layout = \"irc\"")]
    );
    assert_eq!(original.lines().count(), edited.lines().count());

    state.notice = None;
    let effects = update(&mut state, AppEvent::ConfigRead(Ok(edited)));
    assert_eq!(effects, vec![]);
    assert!(
        state.notice.is_none(),
        "reading back our own write is a no-op"
    );
    assert_eq!(state.layout, MessageLayout::Irc);
}

fn ctrl(c: char) -> Event {
    Event::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL))
}

#[tokio::test]
async fn saving_creates_a_missing_file_from_the_template() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("dir").join("config.toml");
    let edit = SettingEdit {
        path: key::MOUSE,
        value: ConfigValue::Bool(false),
    };
    save(&path, &edit).await.unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("\nenabled = false\n"));
    assert_eq!(text.lines().count(), template().lines().count());
    assert_eq!(mode(&path), paths::FILE_MODE);

    std::fs::write(&path, "[mouse\n").unwrap();
    assert!(
        save(&path, &edit).await.is_err(),
        "a broken file is left alone"
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "[mouse\n");
}

#[tokio::test]
async fn preferences_move_from_the_database_to_the_file_once() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("config.toml");
    std::fs::write(&path, template()).unwrap();
    let store = Store::open_in_memory().await.unwrap();
    store
        .set_app_state("ui.message_layout", "irc")
        .await
        .unwrap();
    store
        .set_app_state("ui.allowlist_mode", "on")
        .await
        .unwrap();

    let mut config = Config::default();
    assert_eq!(migrate(&store, &path, &mut config).await, vec![]);
    assert_eq!(config.messages.layout, MessageLayout::Irc);
    assert!(config.conversations.allowlist);
    let parsed = core::parse(&std::fs::read_to_string(&path).unwrap(), &Config::default());
    assert_eq!(parsed.config, config);
    assert_eq!(store.app_state("ui.message_layout").await.unwrap(), None);
    assert_eq!(store.app_state("ui.allowlist_mode").await.unwrap(), None);

    // The file wins over the database.
    std::fs::write(&path, "[messages]\nlayout = \"bubbles\"\n").unwrap();
    store
        .set_app_state("ui.message_layout", "irc")
        .await
        .unwrap();
    let mut config = Config::default();
    migrate(&store, &path, &mut config).await;
    assert_eq!(config.messages.layout, MessageLayout::Bubbles);
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "[messages]\nlayout = \"bubbles\"\n"
    );
    assert_eq!(store.app_state("ui.message_layout").await.unwrap(), None);
}

#[tokio::test]
async fn changes_to_the_file_are_read_after_a_pause() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("config.toml");
    std::fs::write(&path, "").unwrap();
    let (results, mut events) = mpsc::unbounded_channel();
    let _watch = watch(&path, results).unwrap();
    // Two quick writes are read once, with the last text.
    tokio::fs::write(&path, "[mouse]\nenabled = true\n")
        .await
        .unwrap();
    tokio::fs::write(&path, "[mouse]\nenabled = false\n")
        .await
        .unwrap();
    let event = tokio::time::timeout(Duration::from_secs(10), events.recv())
        .await
        .expect("a reload within 10 s");
    let Some(AppEvent::ConfigRead(Ok(text))) = event else {
        panic!("expected ConfigRead, got {event:?}");
    };
    assert_eq!(text, "[mouse]\nenabled = false\n");
    // Reading it (as above) isn't a change.
    let again = tokio::time::timeout(DEBOUNCE * 3, events.recv()).await;
    assert!(again.is_err(), "no second reload: {again:?}");
}

#[tokio::test]
async fn theme_files_are_read_at_startup_and_after_changes() {
    let tmp = tempfile::tempdir().unwrap();
    let config = tmp.path().join("faro").join("config.toml");
    let dir = themes_dir(&config);
    assert_eq!(dir, tmp.path().join("faro").join("themes"));
    // Startup creates the directory, private.
    assert_eq!(load_themes(&dir), vec![]);
    assert_eq!(mode(&dir), 0o700);
    std::fs::write(dir.join("mine.json"), "{}").unwrap();
    std::fs::write(dir.join("notes.txt"), "not a theme").unwrap();
    let read = load_themes(&dir);
    assert_eq!(
        read,
        vec![ThemeText {
            file: "mine.json".to_owned(),
            text: Ok("{}".to_owned())
        }]
    );

    let (results, mut events) = mpsc::unbounded_channel();
    let _watch = watch_themes(&dir, results).unwrap();
    tokio::fs::write(dir.join("other.json"), "{ }")
        .await
        .unwrap();
    let event = tokio::time::timeout(Duration::from_secs(10), events.recv())
        .await
        .expect("a reload within 10 s");
    let Some(AppEvent::ThemesRead(mut files)) = event else {
        panic!("expected ThemesRead, got {event:?}");
    };
    files.sort_by(|a, b| a.file.cmp(&b.file));
    let names: Vec<&str> = files.iter().map(|f| f.file.as_str()).collect();
    assert_eq!(names, vec!["mine.json", "other.json"]);
    // Files that aren't themes don't count.
    tokio::fs::write(dir.join("notes.txt"), "still not")
        .await
        .unwrap();
    let again = tokio::time::timeout(DEBOUNCE * 3, events.recv()).await;
    assert!(again.is_err(), "no reload: {again:?}");
}
