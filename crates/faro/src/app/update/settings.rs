//! Settings (M5, D48): the config file applied to the state at startup and
//! whenever it changes, and settings changed in the app (palette `:`, the
//! allowlist key) written back to it.

use faro_core::config::{self, Config, EmojiChoice, Problem, Severity, key};

use super::{check_terminal, curation};
use crate::app::event::Effect;
use crate::app::keymap::Keymap;
use crate::app::palette::SettingChoice;
use crate::app::state::{EmojiMode, MessageLayout, Notice, State};
use crate::theme::Theme;

/// At startup: the config `main` read, with its problems. The emoji mode
/// was already decided by `main` (it looks for mosh before the first frame).
pub fn configure(state: &mut State, config: Config, problems: Vec<Problem>) -> Vec<Effect> {
    apply(state, config, problems, true)
}

/// The file changed and was read again.
pub fn on_config_read(state: &mut State, read: Result<String, String>) -> Vec<Effect> {
    match read {
        Ok(text) => {
            let parsed = config::parse(&text, &state.config);
            apply(state, parsed.config, parsed.problems, false)
        }
        Err(error) => {
            tracing::warn!(%error, "reading the config file failed");
            state.notice = Some(Notice::error(format!("Config: {error}")));
            vec![]
        }
    }
}

/// Make `config` the one in use. Only what changed is applied, so reading
/// back a file faro just wrote changes nothing.
fn apply(
    state: &mut State,
    config: Config,
    mut problems: Vec<Problem>,
    startup: bool,
) -> Vec<Effect> {
    let keymap = keymap(&config, &mut problems);
    if Theme::named(&config.theme).is_none() {
        problems.push(Problem::warning(format!(
            "theme: there's no theme called \"{}\"; using {} (built in: {})",
            config.theme,
            config::DEFAULT_THEME,
            Theme::NAMES.join(", ")
        )));
    }
    let changed = config != state.config;
    let old = std::mem::replace(&mut state.config, config);
    let mut effects = Vec::new();
    if changed || startup {
        state.keymap = keymap;
        state.layout = state.config.messages.layout;
        let allowlist = state.config.conversations.allowlist;
        if state.list.allowlist != allowlist {
            effects.extend(curation::apply_allowlist_mode(state, allowlist));
        }
        if !startup && old.ui.emoji != state.config.ui.emoji {
            effects.extend(apply_emoji(state));
        }
        // The mouse and the theme are read from `state.config` by the app loop.
    }
    report(state, &problems, changed && !startup);
    effects
}

/// The default key map with the config's bindings applied; bad ones are
/// skipped and reported.
fn keymap(config: &Config, problems: &mut Vec<Problem>) -> Keymap {
    let mut keymap = Keymap::default();
    for binding in &config.keybindings {
        if let Err(error) = keymap.apply(&binding.context, &binding.key, &binding.action) {
            problems.push(Problem::error(format!(
                "keybindings.{}: {error}",
                binding.context
            )));
        }
    }
    keymap
}

/// Show what's wrong with the file, unless it's what was already shown;
/// otherwise confirm a reload that changed something (or fixed the file).
fn report(state: &mut State, problems: &[Problem], reloaded: bool) {
    let texts: Vec<String> = problems.iter().map(|p| p.text.clone()).collect();
    let fresh = texts != state.config_problems;
    state.config_problems = texts;
    if fresh {
        for problem in problems {
            tracing::warn!(problem = %problem.text, "config file");
        }
    }
    let Some(first) = problems.first() else {
        if reloaded || fresh {
            state.notice = Some(Notice::info("Settings reloaded"));
        }
        return;
    };
    if !fresh && !reloaded {
        return;
    }
    let text = match problems.len() - 1 {
        0 => format!("Config: {}", first.text),
        more => format!("Config: {} (and {more} more in the log)", first.text),
    };
    state.notice = Some(if problems.iter().any(|p| p.severity == Severity::Error) {
        Notice::error(text)
    } else {
        Notice::warning(text)
    });
}

/// Follow `state.config.ui.emoji`, unless `--emoji` decided for this run.
fn apply_emoji(state: &mut State) -> Vec<Effect> {
    if state.env.emoji.from_cli {
        return vec![];
    }
    let mode = match state.config.ui.emoji {
        EmojiChoice::Auto => {
            state.env.emoji.auto = true;
            state.env.emoji.checked_at = None;
            return check_terminal(state);
        }
        EmojiChoice::Full => EmojiMode::Full,
        EmojiChoice::Compatible => EmojiMode::Compatible,
    };
    let emoji = &mut state.env.emoji;
    emoji.auto = false;
    if emoji.mode != mode {
        emoji.mode = mode;
        state.env.repaint = true;
    }
    vec![]
}

/// A setting chosen in the palette: applied now and saved to the file.
pub fn choose(state: &mut State, choice: SettingChoice) -> Vec<Effect> {
    match choice {
        SettingChoice::Colors(colors) => {
            state.config.ui.colors = colors;
            state.notice = Some(Notice::info(format!(
                "Drawing with {}",
                state.colors().name()
            )));
            save(state, key::COLORS)
        }
        SettingChoice::Layout(layout) => {
            state.layout = layout;
            state.config.messages.layout = layout;
            state.notice = Some(Notice::info(match layout {
                MessageLayout::Bubbles => "Using the bubbles layout",
                MessageLayout::Irc => "Using the IRC layout",
            }));
            save(state, key::LAYOUT)
        }
        SettingChoice::Allowlist(on) => curation::set_allowlist_mode(state, on),
        SettingChoice::Emoji(emoji) => {
            state.config.ui.emoji = emoji;
            let mut effects = apply_emoji(state);
            if state.env.emoji.from_cli {
                state.notice = Some(Notice::info(
                    "Saved; --emoji or FARO_EMOJI decides until faro restarts",
                ));
            }
            effects.extend(save(state, key::EMOJI));
            effects
        }
        SettingChoice::Mouse(on) => {
            state.config.mouse.enabled = on;
            state.notice = Some(Notice::info(if on {
                "Mouse on"
            } else {
                "Mouse off: the terminal selects text (Ctrl+P, \"mouse\" turns it back on)"
            }));
            save(state, key::MOUSE)
        }
        SettingChoice::Bell(on) => {
            state.config.notifications.bell = on;
            state.notice = Some(Notice::info(if on {
                "Bell on: new messages ring again"
            } else {
                "Do not disturb: the bell is off"
            }));
            save(state, key::BELL)
        }
        SettingChoice::Title(on) => {
            state.config.notifications.title = on;
            state.notice = Some(Notice::info(if on {
                "The window title shows the unread count"
            } else {
                "The window title is left to the terminal"
            }));
            save(state, key::TITLE)
        }
    }
}

/// Do not disturb (`Alt+N`, the palette): the bell off, or back on. It's
/// the `[notifications] bell` setting, so it's saved and survives restarts.
pub fn toggle_bell(state: &mut State) -> Vec<Effect> {
    choose(state, SettingChoice::Bell(!state.config.notifications.bell))
}

/// A theme chosen in the palette: drawn from the next frame and saved.
pub fn choose_theme(state: &mut State, name: &str) -> Vec<Effect> {
    name.clone_into(&mut state.config.theme);
    state.notice = Some(Notice::info(format!("Theme: {name}")));
    save(state, key::THEME)
}

/// Write the current value of the setting at `path` to the config file.
pub fn save(state: &State, path: &'static str) -> Vec<Effect> {
    state
        .config
        .edit(path)
        .map(Effect::SaveSetting)
        .into_iter()
        .collect()
}
