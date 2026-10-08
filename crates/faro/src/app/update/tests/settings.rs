//! The config file applied while running (M5, D48).

use faro_core::config::{self as core, EmojiChoice};

use super::*;
use crate::app::keymap::Action;
use crate::app::state::{EmojiMode, EmojiSetting, MessageLayout, NoticeLevel};

fn reload(state: &mut State, text: &str) -> Vec<Effect> {
    update(state, AppEvent::ConfigRead(Ok(text.to_owned())))
}

fn notice(state: &State) -> Option<(NoticeLevel, String)> {
    state.notice.as_ref().map(|n| (n.level, n.text.clone()))
}

#[test]
fn editing_the_file_changes_behaviour() {
    let mut state = loaded_state();
    assert_eq!(state.layout, MessageLayout::Irc);
    let effects = reload(
        &mut state,
        "[messages]\nlayout = \"bubbles\"\n[mouse]\nenabled = false\n\
         [keybindings.global]\n\"f2\" = \"palette.open\"\n\"ctrl+p\" = \"none\"\n",
    );
    assert_eq!(effects, vec![]);
    assert_eq!(state.layout, MessageLayout::Bubbles);
    assert!(!state.config.mouse.enabled);
    assert_eq!(
        notice(&state),
        Some((NoticeLevel::Info, "Settings reloaded".to_owned()))
    );
    let f2 = KeyEvent::new(KeyCode::F(2), KeyModifiers::NONE);
    let ctrl_p = KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL);
    assert_eq!(
        state.keymap.lookup(f2, Focus::Input),
        Some(Action::OpenPalette)
    );
    assert_eq!(state.keymap.lookup(ctrl_p, Focus::Input), None);

    // Removing a setting brings back its default.
    reload(&mut state, "");
    assert_eq!(state.layout, MessageLayout::Bubbles);
    assert!(state.config.mouse.enabled);
    assert_eq!(
        state.keymap.lookup(ctrl_p, Focus::Input),
        Some(Action::OpenPalette)
    );
}

#[test]
fn reading_back_the_same_settings_does_nothing() {
    let mut state = loaded_state();
    let text = "[messages]\nlayout = \"irc\"\n";
    state.notice = None;
    assert_eq!(reload(&mut state, text), vec![]);
    assert_eq!(notice(&state), None, "nothing changed");
}

#[test]
fn a_bad_value_is_reported_once_and_the_old_value_kept() {
    let mut state = loaded_state();
    let text = "[messages]\nlayout = \"bubble\"\nwhatever = 1\n";
    reload(&mut state, text);
    assert_eq!(state.layout, MessageLayout::Irc);
    assert_eq!(
        notice(&state),
        Some((
            NoticeLevel::Error,
            "Config: messages.layout: expected \"bubbles\" or \"irc\", not \"bubble\"; keeping \
             \"irc\" (and 1 more in the log)"
                .to_owned()
        ))
    );
    state.notice = None;
    reload(&mut state, text);
    assert_eq!(notice(&state), None, "already reported");

    // Fixed: the problems go away.
    reload(&mut state, "[messages]\nlayout = \"irc\"\n");
    assert_eq!(
        notice(&state),
        Some((NoticeLevel::Info, "Settings reloaded".to_owned()))
    );
    assert_eq!(state.config_problems, Vec::<String>::new());
}

#[test]
fn a_broken_file_keeps_the_last_good_config() {
    let mut state = loaded_state();
    reload(&mut state, "[mouse]\nenabled = false\n");
    reload(&mut state, "[mouse]\nenabled = \n");
    assert!(!state.config.mouse.enabled);
    let (level, text) = notice(&state).unwrap();
    assert_eq!(level, NoticeLevel::Error);
    assert!(text.starts_with("Config: line 2: "), "{text}");
}

#[test]
fn bad_keybindings_and_themes_are_reported_and_skipped() {
    let mut state = loaded_state();
    reload(
        &mut state,
        "theme = \"no-such-theme\"\n[keybindings.global]\n\"ctrl+k\" = \"no.such\"\n",
    );
    assert_eq!(state.config.theme, "no-such-theme");
    assert_eq!(
        state.config_problems,
        vec![
            "keybindings.global: unknown action \"no.such\"".to_owned(),
            "theme: there's no theme called \"no-such-theme\"; using faro-dark (Ctrl+P › Themes lists them)"
                .to_owned(),
        ]
    );
    assert_eq!(state.keymap, crate::app::keymap::Keymap::default());
}

#[test]
fn allowlist_mode_follows_the_file() {
    let mut state = loaded_state();
    reload(&mut state, "[conversations]\nallowlist = true\n");
    assert!(state.list.allowlist);
    reload(&mut state, "");
    assert!(!state.list.allowlist);
}

#[test]
fn the_emoji_setting_follows_the_file_unless_chosen_on_the_command_line() {
    let mut state = loaded_state();
    state.env.emoji = EmojiSetting {
        mode: EmojiMode::Full,
        auto: true,
        checked_at: None,
        from_cli: false,
    };
    reload(&mut state, "[ui]\nemoji = \"compatible\"\n");
    assert_eq!(state.env.emoji.mode, EmojiMode::Compatible);
    assert!(!state.env.emoji.auto);
    assert!(state.env.repaint);

    let effects = reload(&mut state, "[ui]\nemoji = \"auto\"\n");
    assert_eq!(effects, vec![Effect::CheckTerminal]);
    assert!(state.env.emoji.auto);

    state.env.emoji.from_cli = true;
    state.env.emoji.auto = false;
    let effects = reload(&mut state, "[ui]\nemoji = \"full\"\n");
    assert_eq!(effects, vec![]);
    assert_eq!(state.config.ui.emoji, EmojiChoice::Full);
    assert_eq!(state.env.emoji.mode, EmojiMode::Compatible);
}

#[test]
fn startup_applies_the_config_and_shows_its_problems() {
    let mut state = State::new(ME, crate::fixtures::contacts(), vec![]);
    let mut config = core::Config::default();
    config.messages.layout = MessageLayout::Irc;
    let problems = vec![core::Problem::warning("unknown setting x; ignored")];
    configure(&mut state, config, problems);
    assert_eq!(state.layout, MessageLayout::Irc);
    assert_eq!(
        notice(&state),
        Some((
            NoticeLevel::Warning,
            "Config: unknown setting x; ignored".to_owned()
        ))
    );
}

#[test]
fn a_failed_save_says_so() {
    let mut state = loaded_state();
    update(
        &mut state,
        AppEvent::SettingNotSaved("the config file has an error (line 1: x)".into()),
    );
    assert_eq!(notice(&state).map(|n| n.0), Some(NoticeLevel::Error));
}

#[test]
fn themes_and_colours_are_chosen_in_the_palette_and_saved() {
    use crate::app::palette::SettingChoice;
    use crate::app::update::settings::{choose, choose_theme};
    use crate::theme::ColorDepth;

    let mut state = loaded_state();
    state.env.colors = ColorDepth::Indexed256;
    assert_eq!(
        state.colors(),
        ColorDepth::Indexed256,
        "auto follows detection"
    );
    let effects = choose_theme(&mut state, "faro-light");
    assert_eq!(state.config.theme, "faro-light");
    assert_eq!(
        effects,
        vec![Effect::SaveSetting(core::SettingEdit {
            path: core::key::THEME,
            value: core::ConfigValue::Text("faro-light".to_owned()),
        })]
    );
    let effects = choose(
        &mut state,
        SettingChoice::Colors(core::ColorChoice::Truecolor),
    );
    assert_eq!(state.colors(), ColorDepth::Truecolor);
    assert_eq!(
        effects,
        vec![Effect::SaveSetting(core::SettingEdit {
            path: core::key::COLORS,
            value: core::ConfigValue::Text("truecolor".to_owned()),
        })]
    );
    // And from the file, live.
    reload(&mut state, "theme = \"system\"\n[ui]\ncolors = \"16\"\n");
    assert_eq!(state.config.theme, "system");
    assert_eq!(state.colors(), ColorDepth::Ansi16);
}

mod theme_files {
    use super::*;
    use crate::theme::{Source, Theme, ThemeText};

    const MINE: &str =
        r##"{ "theme": { "background": "#101010", "text": "#e0e0e0", "primary": "#e0a040" } }"##;

    fn file(name: &str, text: &str) -> ThemeText {
        ThemeText {
            file: name.to_owned(),
            text: Ok(text.to_owned()),
        }
    }

    #[test]
    fn a_users_theme_can_be_the_configs() {
        let mut state = State::new(ME, crate::fixtures::contacts(), vec![]);
        state.user_themes.load(vec![file("mine.json", MINE)]);
        let config = core::Config {
            theme: "mine".to_owned(),
            ..core::Config::default()
        };
        configure(&mut state, config, vec![]);
        assert_eq!(state.config_problems, Vec::<String>::new());
        let theme = Theme::find(state.theme_name(), &state.user_themes).unwrap();
        assert_eq!(theme.source, Source::User);
    }

    #[test]
    fn changed_files_are_used_and_their_problems_shown() {
        let mut state = loaded_state();
        update(
            &mut state,
            AppEvent::ThemesRead(vec![file("mine.json", MINE)]),
        );
        assert_eq!(
            notice(&state),
            Some((NoticeLevel::Info, "Themes reloaded".to_owned()))
        );
        assert_eq!(state.user_themes.generation, 1);
        reload(&mut state, "theme = \"mine\"\n");
        assert_eq!(state.config_problems, Vec::<String>::new());

        // The file breaks: it's skipped and the theme in use is gone.
        update(
            &mut state,
            AppEvent::ThemesRead(vec![file("mine.json", "{"), file("ok.json", MINE)]),
        );
        let (level, text) = notice(&state).unwrap();
        assert_eq!(level, NoticeLevel::Warning);
        assert!(
            text.starts_with("Themes: mine.json: not valid JSON")
                && text.ends_with("(and 1 more in the log)"),
            "{text}"
        );
        assert_eq!(state.user_themes.generation, 2);
        assert_eq!(
            state
                .user_themes
                .themes
                .iter()
                .map(|t| t.name.as_str())
                .collect::<Vec<_>>(),
            vec!["ok"]
        );
    }
}
