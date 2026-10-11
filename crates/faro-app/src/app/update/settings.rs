//! Settings (M5, D48): the config file applied to the state at startup and
//! whenever it changes, and settings changed in the app (palette `:`, the
//! allowlist key) written back to it.

use faro_core::config::{
    self, AutoDownload, BubbleStyle, Config, DesktopNotifications, EmojiChoice, InlineImages,
    MessageDensity, NotificationPrivacy, Problem, Severity, TimestampFormat, TranscribeMode, key,
};

use super::{check_terminal, curation, selection};
use crate::app::event::Effect;
use crate::app::keymap::Keymap;
use crate::app::palette::{self, Setting, SettingChoice};
use crate::app::state::{EmojiMode, MessageLayout, Notice, State};
use crate::theme::{ColorDepth, Theme, ThemeText};

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
    problems.extend(unknown_theme(state, &config.theme));
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
        if !startup && old.messages.bubble_style != state.config.messages.bubble_style {
            effects.extend(apply_bubble_style(state));
        }
        if !startup && old.media.inline_images != state.config.media.inline_images {
            state.env.repaint = true;
            effects.extend(apply_inline_images(state));
        }
        if !startup && old.media.auto_download != state.config.media.auto_download {
            effects.extend(selection::queue_auto_downloads(state));
        }
        if !startup && old.voice.transcribe != state.config.voice.transcribe {
            effects.extend(selection::queue_auto_transcriptions(state));
        }
        // The mouse, the theme and `reduce_motion` are read from
        // `state.config` by the app loop.
    }
    report(state, &problems, changed && !startup);
    effects
}

/// A warning if there's no theme called `name` (the default is drawn).
fn unknown_theme(state: &State, name: &str) -> Option<Problem> {
    Theme::find(name, &state.user_themes).is_none().then(|| {
        Problem::warning(format!(
            "theme: there's no theme called \"{name}\"; using {} (Ctrl+P › Themes lists them)",
            config::DEFAULT_THEME,
        ))
    })
}

/// The user's theme files were read again (one changed): use them, and
/// say what's wrong with any that can't be used.
pub fn on_themes_read(state: &mut State, files: Vec<ThemeText>) -> Vec<Effect> {
    let mut problems = state.user_themes.load(files);
    for problem in &problems {
        tracing::warn!(%problem, "theme file");
    }
    problems.extend(unknown_theme(state, &state.config.theme).map(|p| p.text));
    state.notice = Some(match problems.split_first() {
        None => Notice::info("Themes reloaded"),
        Some((first, [])) => Notice::warning(format!("Themes: {first}")),
        Some((first, more)) => Notice::warning(format!(
            "Themes: {first} (and {} more in the log)",
            more.len()
        )),
    });
    vec![]
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
        EmojiChoice::Full => state.env.emoji.full(),
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

/// Following kitty (`auto`) needs a fresh look at the terminal (D58); the
/// renderer reads the style from `state.config`.
fn apply_bubble_style(state: &mut State) -> Vec<Effect> {
    if state.config.messages.bubble_style != BubbleStyle::Auto {
        return vec![];
    }
    state.env.emoji.checked_at = None;
    check_terminal(state)
}

/// Following terminal graphics (`auto`) needs a fresh look at the terminal
/// (D78); the app loop updates `Images` before the next frame.
fn apply_inline_images(state: &mut State) -> Vec<Effect> {
    if state.config.media.inline_images != InlineImages::Auto {
        return vec![];
    }
    state.env.emoji.checked_at = None;
    check_terminal(state)
}

/// A setting chosen in the palette: applied now and saved to the file.
pub fn choose(state: &mut State, choice: SettingChoice) -> Vec<Effect> {
    match choice {
        SettingChoice::Colors(colors) => {
            state.config.ui.colors = colors;
            let name = state.colors().name();
            save_with_notice(state, key::COLORS, format!("Drawing with {name}"))
        }
        SettingChoice::Layout(layout) => {
            state.layout = layout;
            state.config.messages.layout = layout;
            let msg = match layout {
                MessageLayout::Bubbles => "Using the bubbles layout",
                MessageLayout::Irc => "Using the IRC layout",
            };
            save_with_notice(state, key::LAYOUT, msg)
        }
        SettingChoice::BubbleStyle(style) => {
            state.config.messages.bubble_style = style;
            let msg = match style {
                BubbleStyle::Auto => "Bubbles: rounded in kitty, bars elsewhere",
                BubbleStyle::Rounded => "Bubbles have rounded ends",
                BubbleStyle::Bar => "Bubbles have a bar on the outer side",
                BubbleStyle::Mixed => "Bubbles have a bar outside and round inside",
            };
            let mut effects = apply_bubble_style(state);
            effects.extend(save_with_notice(state, key::BUBBLE_STYLE, msg));
            effects
        }
        SettingChoice::Density(density) => {
            state.config.messages.density = density;
            let msg = match density {
                MessageDensity::Comfortable => "Message density: comfortable",
                MessageDensity::Compact => "Message density: compact",
            };
            save_with_notice(state, key::DENSITY, msg)
        }
        SettingChoice::Timestamps(timestamps) => {
            state.config.messages.timestamps = timestamps;
            let msg = match timestamps {
                TimestampFormat::Absolute => "Timestamps: absolute (HH:MM)",
                TimestampFormat::Relative => "Timestamps: relative (now, 5m, 2h, 3d)",
                TimestampFormat::Off => "Timestamps off (shown on the selected message)",
            };
            save_with_notice(state, key::TIMESTAMPS, msg)
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
        SettingChoice::Desktop(_)
        | SettingChoice::Privacy(_)
        | SettingChoice::InlineImages(_)
        | SettingChoice::AutoDownload(_)
        | SettingChoice::Transcribe(_) => choose_media_and_notifications(state, choice),
        SettingChoice::Mouse(on)
        | SettingChoice::Bell(on)
        | SettingChoice::Title(on)
        | SettingChoice::ReduceMotion(on)
        | SettingChoice::BubblePadding(on)
        | SettingChoice::MatchWidths(on) => choose_toggle(state, choice, on),
    }
}

fn choose_media_and_notifications(state: &mut State, choice: SettingChoice) -> Vec<Effect> {
    match choice {
        SettingChoice::Desktop(mode) => {
            state.config.notifications.desktop = mode;
            let msg = match mode {
                DesktopNotifications::Auto => "Desktop notifications: auto (through the terminal)",
                DesktopNotifications::Osc => "Desktop notifications: through the terminal (OSC)",
                DesktopNotifications::Native => "Desktop notifications: native OS notifications",
                DesktopNotifications::Off => "Desktop notifications off",
            };
            save_with_notice(state, key::DESKTOP, msg)
        }
        SettingChoice::Privacy(privacy) => {
            state.config.notifications.privacy = privacy;
            let msg = match privacy {
                NotificationPrivacy::None => "Notification privacy: generic alert only",
                NotificationPrivacy::Sender => "Notification privacy: sender name only",
                NotificationPrivacy::Preview => "Notification privacy: sender and message preview",
            };
            save_with_notice(state, key::PRIVACY, msg)
        }
        SettingChoice::InlineImages(mode) => {
            state.config.media.inline_images = mode;
            state.env.repaint = true;
            let msg = match mode {
                InlineImages::Auto => "Inline images: terminal graphics where supported",
                InlineImages::Kitty => "Inline images: Kitty graphics (forced)",
                InlineImages::Sixel => "Inline images: Sixel (forced)",
                InlineImages::Iterm2 => "Inline images: iTerm2 (forced)",
                InlineImages::Halfblocks => "Inline images: Unicode half-blocks",
                InlineImages::Off => "Inline images off",
            };
            let mut effects = apply_inline_images(state);
            effects.extend(save_with_notice(state, key::INLINE_IMAGES, msg));
            effects
        }
        SettingChoice::AutoDownload(policy) => {
            state.config.media.auto_download = policy;
            let msg = match policy {
                AutoDownload::Images => "Auto-download: images and stickers (up to 25 MB)",
                AutoDownload::All => "Auto-download: all attachments (up to 25 MB)",
                AutoDownload::Off => "Auto-download off (press d to download)",
            };
            let mut effects = selection::queue_auto_downloads(state);
            effects.extend(save_with_notice(state, key::AUTO_DOWNLOAD, msg));
            effects
        }
        SettingChoice::Transcribe(mode) => {
            state.config.voice.transcribe = mode;
            let msg = match mode {
                TranscribeMode::Off => "Voice note transcription off",
                TranscribeMode::Local => "Voice note transcription: local Whisper",
                TranscribeMode::Remote => "Voice note transcription: remote API (asks per chat)",
            };
            let mut effects = selection::queue_auto_transcriptions(state);
            effects.extend(save_with_notice(state, key::VOICE_TRANSCRIBE, msg));
            effects
        }
        SettingChoice::Colors(_)
        | SettingChoice::Emoji(_)
        | SettingChoice::Layout(_)
        | SettingChoice::BubbleStyle(_)
        | SettingChoice::Density(_)
        | SettingChoice::Timestamps(_)
        | SettingChoice::Allowlist(_)
        | SettingChoice::Mouse(_)
        | SettingChoice::Bell(_)
        | SettingChoice::Title(_)
        | SettingChoice::ReduceMotion(_)
        | SettingChoice::BubblePadding(_)
        | SettingChoice::MatchWidths(_) => vec![],
    }
}

fn choose_toggle(state: &mut State, choice: SettingChoice, on: bool) -> Vec<Effect> {
    match choice {
        SettingChoice::Mouse(_) => {
            state.config.mouse.enabled = on;
            let msg = if on {
                "Mouse on"
            } else {
                "Mouse off: the terminal selects text (Ctrl+P, \"mouse\" turns it back on)"
            };
            save_with_notice(state, key::MOUSE, msg)
        }
        SettingChoice::Bell(_) => {
            state.config.notifications.bell = on;
            let msg = if on {
                "Bell on: new messages ring again"
            } else {
                "Do not disturb: the bell is off"
            };
            save_with_notice(state, key::BELL, msg)
        }
        SettingChoice::Title(_) => {
            state.config.notifications.title = on;
            let msg = if on {
                "The window title shows the unread count"
            } else {
                "The window title is left to the terminal"
            };
            save_with_notice(state, key::TITLE, msg)
        }
        SettingChoice::ReduceMotion(_) => {
            // The app loop reads it from `state.config` (D61).
            state.config.reduce_motion = on;
            let msg = if on {
                "Reduce motion: the lighthouse stands still"
            } else {
                "The lighthouse's beam turns again"
            };
            save_with_notice(state, key::REDUCE_MOTION, msg)
        }
        SettingChoice::BubblePadding(_) => {
            state.config.messages.bubble_padding = on;
            let msg = if on {
                "Rounded bubbles keep a blank cell beside the text"
            } else {
                "Rounded bubbles hug the text"
            };
            save_with_notice(state, key::BUBBLE_PADDING, msg)
        }
        SettingChoice::MatchWidths(_) => {
            state.config.messages.match_widths = on;
            let msg = if on {
                "Compact bubbles of nearly the same width line up"
            } else {
                "Compact bubbles keep their own widths"
            };
            save_with_notice(state, key::MATCH_WIDTHS, msg)
        }
        SettingChoice::Colors(_)
        | SettingChoice::Emoji(_)
        | SettingChoice::Desktop(_)
        | SettingChoice::Privacy(_)
        | SettingChoice::Layout(_)
        | SettingChoice::BubbleStyle(_)
        | SettingChoice::Density(_)
        | SettingChoice::Timestamps(_)
        | SettingChoice::Allowlist(_)
        | SettingChoice::InlineImages(_)
        | SettingChoice::AutoDownload(_)
        | SettingChoice::Transcribe(_) => vec![],
    }
}

/// Put `setting` back to its default (`Ctrl+R` in the settings list, D100):
/// applied like a reload, then written to the file with the default value
/// spelled out (not commented out), so the file says what's in use.
pub fn reset(state: &mut State, setting: Setting) -> Vec<Effect> {
    let path = setting.key();
    let Some(default) = Config::default().value(path) else {
        return vec![];
    };
    if state.config.value(path).as_ref() == Some(&default) {
        state.notice = Some(Notice::info(format!(
            "{} is already at its default",
            setting.title()
        )));
        return vec![];
    }
    let mut config = state.config.clone();
    if let Err(error) = config.set(path, &default) {
        tracing::warn!(%error, path, "resetting a setting failed");
        return vec![];
    }
    let mut effects = apply(state, config, Vec::new(), false);
    effects.extend(save(state, path));
    let value = palette::value_title(state, setting);
    state.notice = Some(Notice::info(format!(
        "{}: back to its default ({value})",
        setting.title()
    )));
    effects
}

fn save_with_notice(state: &mut State, path: &'static str, text: impl Into<String>) -> Vec<Effect> {
    state.notice = Some(Notice::info(text));
    save(state, path)
}

/// Do not disturb (`Alt+N`, the palette): the bell off, or back on. It's
/// the `[notifications] bell` setting, so it's saved and survives restarts.
pub fn toggle_bell(state: &mut State) -> Vec<Effect> {
    choose(state, SettingChoice::Bell(!state.config.notifications.bell))
}

/// A theme chosen in the palette: drawn from the next frame and saved. At
/// 16 colours it's saved all the same, but `system` is drawn until the
/// terminal (or `[ui] colors`) allows more (D57).
pub fn choose_theme(state: &mut State, name: &str) -> Vec<Effect> {
    name.clone_into(&mut state.config.theme);
    let text = if state.colors() == ColorDepth::Ansi16 && name != "system" {
        format!("Theme: {name} (drawn as system until 256 colours or more)")
    } else {
        format!("Theme: {name}")
    };
    state.notice = Some(Notice::info(text));
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
