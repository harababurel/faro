//! `faro doctor` (`#47`): one diagnostic report explaining terminal
//! capabilities, graphics fallback, config conflicts, and keymap clashes.
//!
//! Assembles the existing terminal, graphics, settings, and keymap resolvers
//! into structured sections for `Level::Doctor` in the command palette and a
//! copy-pasteable plain-text report for clipboard export and `faro doctor` on
//! the CLI.

use std::fmt::Write as _;

use faro_core::config::{Avatars, Choice as _, EmojiChoice, InlineImages, TranscribeMode};
use faro_core::store::Profile;

use super::keymap::{Action, Context, KeyChord, Keymap};
use super::palette::{self, SettingsGroup};
use super::state::{
    BlockGlyphs, EmojiMode, GraphicsBlocker, GraphicsSupport, Guard, MessageLayout, State,
};
use crate::theme::ColorDepth;

/// One group of diagnostic rows in `faro doctor`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    pub title: &'static str,
    pub rows: Vec<Row>,
}

/// A single diagnostic line (`title` on the left, short `detail` badge on the
/// right in the palette; `title  [detail]` in the plain-text report).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub title: String,
    pub detail: String,
}

impl Row {
    fn new(title: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            detail: detail.into(),
        }
    }
}

/// Build the structured diagnostic sections for `state`.
pub fn sections(state: &State) -> Vec<Section> {
    vec![
        Section {
            title: "Terminal & session",
            rows: terminal_rows(state),
        },
        Section {
            title: "Graphics & media",
            rows: graphics_rows(state),
        },
        Section {
            title: "Config & keymap",
            rows: config_and_keymap_rows(state),
        },
    ]
}

/// Format a copy-pasteable plain-text diagnostic report for `state`.
pub fn report(state: &State) -> String {
    let mut out = format!("faro doctor ({})\n", env!("CARGO_PKG_VERSION"));
    for section in sections(state) {
        let _ = writeln!(out, "\n[{}]", section.title);
        for row in section.rows {
            if row.detail.is_empty() {
                let _ = writeln!(out, "- {}", row.title);
            } else {
                let _ = writeln!(out, "- {} [{}]", row.title, row.detail);
            }
        }
    }
    out
}

fn terminal_rows(state: &State) -> Vec<Row> {
    let profile = match state.profile {
        Profile::Demo => "demo",
        Profile::Signal => "signal",
    };
    let guard = if state.guard == Guard::On {
        " · allowlist guard"
    } else {
        ""
    };
    let version_row = Row::new(
        format!(
            "faro {} · profile: {profile}{guard}",
            env!("CARGO_PKG_VERSION")
        ),
        profile,
    );

    let effective_colors = state.colors();
    let color_detail = match effective_colors {
        ColorDepth::Truecolor => "ok",
        ColorDepth::Indexed256 => "themes approximated",
        ColorDepth::Ansi16 => "system theme only",
    };
    let colors_row = Row::new(
        format!(
            "Colours: {} (detected: {}, config: {})",
            effective_colors.name(),
            state.env.colors.name(),
            state.config.ui.colors.name()
        ),
        color_detail,
    );

    let emoji_mode = match state.env.emoji.mode {
        EmojiMode::Full | EmojiMode::Exact => "full",
        EmojiMode::Compatible => "compatible",
    };
    let (emoji_why, emoji_detail) = if state.env.emoji.from_cli {
        ("--emoji / FARO_EMOJI".to_owned(), "cli override")
    } else if state.env.emoji.auto && state.env.emoji.mosh {
        ("auto: mosh detected".to_owned(), "mosh fallback")
    } else if state.env.emoji.auto {
        ("auto: no mosh".to_owned(), "ok")
    } else {
        (format!("config: {}", state.config.ui.emoji.name()), "ok")
    };
    let emoji_row = Row::new(format!("Emoji: {emoji_mode} ({emoji_why})"), emoji_detail);

    let glyphs = match state.env.glyphs {
        BlockGlyphs::Native => "kitty (rounded)",
        BlockGlyphs::Font => "standard (bars)",
    };
    let bubble_detail = if state.message_layout() == MessageLayout::Bubbles {
        "ok"
    } else {
        "irc layout"
    };
    let bubble_row = Row::new(
        format!(
            "Bubble glyphs: {} (terminal: {glyphs}, config: {})",
            state.bubble_style().name(),
            state.config.messages.bubble_style.name()
        ),
        bubble_detail,
    );

    let remote = if state.env.over_ssh { "ssh" } else { "local" };
    let mosh = if state.env.emoji.mosh {
        "mosh"
    } else {
        "no mosh"
    };
    let tmux = if state.env.tmux_clients.is_empty() {
        "no tmux clients".to_owned()
    } else {
        format!("tmux: {}", state.env.tmux_clients.join(", "))
    };
    let session_detail = if state.env.emoji.mosh {
        "mosh"
    } else if state.env.over_ssh {
        "ssh"
    } else {
        "local"
    };
    let session_row = Row::new(
        format!("Session: {remote} · {mosh} · {tmux}"),
        session_detail,
    );

    vec![version_row, colors_row, emoji_row, bubble_row, session_row]
}

fn graphics_rows(state: &State) -> Vec<Row> {
    let mut rows = Vec::new();
    let mode = state.config.media.inline_images;
    let note = palette::inline_images_note(state);
    let direct_protocol = matches!(
        state.env.image_protocol,
        Some(InlineImages::Kitty | InlineImages::Sixel | InlineImages::Iterm2)
    ) && state.env.graphics == GraphicsSupport::Direct
        && state.env.graphics_blocker.is_none()
        && state.colors() == ColorDepth::Truecolor;

    let inline_detail = match mode {
        InlineImages::Off => "off",
        InlineImages::Halfblocks => "halfblocks",
        InlineImages::Kitty | InlineImages::Sixel | InlineImages::Iterm2 => "forced",
        InlineImages::Auto if direct_protocol => "direct",
        InlineImages::Auto => "fallback",
    };
    rows.push(Row::new(format!("Inline images: {note}"), inline_detail));

    let probed = state
        .env
        .image_protocol
        .map_or("not probed / none", InlineImages::name);
    let transport = match state.env.graphics {
        GraphicsSupport::Direct => "direct graphics allowed",
        GraphicsSupport::HalfblocksOnly => "halfblocks only",
    };
    rows.push(Row::new(
        format!(
            "Graphics probe: {probed} · {transport} (config: {})",
            mode.name()
        ),
        probed,
    ));

    if let Some(blocker) = &state.env.graphics_blocker {
        let (explanation, badge) = match blocker {
            GraphicsBlocker::Mosh => (
                "Graphics blocker: faro runs under mosh, which cannot carry terminal graphics"
                    .to_owned(),
                "mosh",
            ),
            GraphicsBlocker::MoshClient(tty) => (
                format!(
                    "Graphics blocker: tmux client {tty} is on mosh (fix: tmux detach-client -t {tty})"
                ),
                "tmux mosh",
            ),
            GraphicsBlocker::OtherClient { tty, term } => (
                format!(
                    "Graphics blocker: tmux client {tty} is {term} (fix: tmux detach-client -t {tty})"
                ),
                "tmux mixed",
            ),
            GraphicsBlocker::NoClient => (
                "Graphics blocker: no tmux client was attached when faro last checked".to_owned(),
                "tmux detached",
            ),
        };
        rows.push(Row::new(explanation, badge));
    }

    let avatars = match state.config.conversations.avatars {
        Avatars::Off => "off",
        Avatars::Initials => "initials",
        Avatars::Auto | Avatars::Pictures if state.draws_pictures() => "pictures",
        Avatars::Auto => "off (no graphics protocol)",
        Avatars::Pictures => "initials (no graphics protocol)",
    };
    rows.push(Row::new(
        format!(
            "Avatars: {avatars} (config: {})",
            state.config.conversations.avatars.name()
        ),
        avatars,
    ));
    rows.push(transcribe_row(state));
    rows
}

fn transcribe_row(state: &State) -> Row {
    match state.config.voice.transcribe {
        TranscribeMode::Off => Row::new("Voice transcription: off", "off"),
        TranscribeMode::Local => {
            let program = state.config.voice.program();
            let (status, detail) = match &state.transcriber {
                Some((checked, true)) if checked == program => ("installed", "local"),
                Some((checked, false)) if checked == program => {
                    ("not found on PATH", "missing command")
                }
                _ => ("not checked yet", "local"),
            };
            Row::new(
                format!(
                    "Voice transcription: local ({program} · model: {} · {status})",
                    state.config.voice.transcribe_model
                ),
                detail,
            )
        }
        TranscribeMode::Remote => Row::new(
            format!(
                "Voice transcription: remote ({} · model: {})",
                state.config.voice.transcribe_url, state.config.voice.transcribe_remote_model
            ),
            "remote",
        ),
    }
}

fn config_and_keymap_rows(state: &State) -> Vec<Row> {
    let mut rows = Vec::new();
    let config_path = if state.config_path.as_os_str().is_empty() {
        "default".to_owned()
    } else {
        state.config_path.display().to_string()
    };
    let changed_count = SettingsGroup::ALL
        .iter()
        .flat_map(|g| g.settings().iter().copied())
        .filter(|s| s.changed(state))
        .count();
    rows.push(Row::new(
        format!("Config file: {config_path} ({changed_count} modified)"),
        if state.config_problems.is_empty() {
            "ok"
        } else {
            "has problems"
        },
    ));

    let active_theme = palette::current_theme(state);
    let configured_theme = &state.config.theme;
    let custom_themes = state.user_themes.themes.len();
    let theme_detail = if state.colors() == ColorDepth::Ansi16 && configured_theme != "system" {
        "16-colour fallback"
    } else if active_theme != configured_theme {
        "fallback"
    } else {
        "ok"
    };
    rows.push(Row::new(
        format!("Theme: {active_theme} (config: {configured_theme}, {custom_themes} custom)"),
        theme_detail,
    ));

    let mut conflicts = Vec::new();
    for problem in &state.config_problems {
        conflicts.push(Row::new(format!("Problem: {problem}"), "config error"));
    }
    for setting in SettingsGroup::ALL
        .iter()
        .flat_map(|g| g.settings().iter().copied())
    {
        if setting.changed(state)
            && let Some(reason) = setting.unavailable(state)
        {
            conflicts.push(Row::new(
                format!(
                    "Conflict: {} = {} has no effect ({reason})",
                    setting.key(),
                    palette::value_title(state, setting)
                ),
                "inactive",
            ));
        }
    }
    if state.env.emoji.from_cli && state.config.ui.emoji != EmojiChoice::Auto {
        conflicts.push(Row::new(
            format!(
                "Conflict: ui.emoji = {} overridden by --emoji / FARO_EMOJI",
                state.config.ui.emoji.name()
            ),
            "overridden",
        ));
    }
    if state.colors() == ColorDepth::Ansi16 && state.config.theme != "system" {
        conflicts.push(Row::new(
            format!(
                "Conflict: theme = \"{}\" drawn as system in 16-colour mode",
                state.config.theme
            ),
            "16 colours",
        ));
    }
    if state.config.security.auto_lock_mins > 0 && state.lock.verifier.is_none() {
        conflicts.push(Row::new(
            format!(
                "Conflict: security.auto_lock_mins = {} needs a passphrase (Alt+L or faro encrypt)",
                state.config.security.auto_lock_mins
            ),
            "no passphrase",
        ));
    }
    if let Some(program) = state.transcriber_missing() {
        conflicts.push(Row::new(
            format!("Conflict: voice.transcribe = local, but \"{program}\" is not on PATH"),
            "missing command",
        ));
    }
    if conflicts.is_empty() {
        rows.push(Row::new(
            "Config problems: none (all settings valid and active)",
            "ok",
        ));
    } else {
        rows.extend(conflicts);
    }

    rows.extend(keymap_rows(state));
    rows
}

fn keymap_rows(state: &State) -> Vec<Row> {
    let defaults = Keymap::default();
    let mut clashes = Vec::new();
    let mut seen: Vec<(Context, KeyChord, String)> = Vec::new();
    let mut displaced: Vec<(Context, KeyChord, Action)> = Vec::new();

    for binding in &state.config.keybindings {
        let (Some(ctx), Some(chord)) = (
            Context::parse(&binding.context),
            KeyChord::parse(&binding.key),
        ) else {
            continue;
        };
        if binding.action != "none" && Action::parse(&binding.action).is_none() {
            continue;
        }
        if let Some((_, _, prev_action)) =
            seen.iter_mut().find(|(c, k, _)| *c == ctx && *k == chord)
        {
            clashes.push(Row::new(
                format!(
                    "Clash: keybindings.{} \"{}\" bound twice (\"{}\" overwritten by \"{}\")",
                    ctx.name(),
                    chord.label(),
                    prev_action,
                    binding.action
                ),
                "clash",
            ));
            binding.action.clone_into(prev_action);
        } else {
            seen.push((ctx, chord, binding.action.clone()));
            if let Some(default_action) = defaults.get(ctx, chord)
                && (binding.action == "none"
                    || Action::parse(&binding.action) != Some(default_action))
                && !displaced
                    .iter()
                    .any(|(c, _, a)| *c == ctx && *a == default_action)
            {
                displaced.push((ctx, chord, default_action));
            }
        }
    }

    for (ctx, chord, action_str) in &seen {
        if *ctx != Context::Global
            && let Some(action) = Action::parse(action_str)
            && let Some(global_action) = state.keymap.get(Context::Global, *chord)
            && global_action != action
            && defaults.get(*ctx, *chord) != Some(action)
        {
            clashes.push(Row::new(
                format!(
                    "Clash: keybindings.{} \"{}\" ({}) shadows global \"{}\"",
                    ctx.name(),
                    chord.label(),
                    action.name().unwrap_or(action_str),
                    global_action.name().unwrap_or("global")
                ),
                "shadows global",
            ));
        }
    }

    for (ctx, chord, old_action) in displaced {
        let none_in_ctx = state.keymap.keys_in(ctx, old_action).is_empty();
        let none_in_global =
            ctx == Context::Global || state.keymap.keys_in(Context::Global, old_action).is_empty();
        if none_in_ctx && none_in_global {
            clashes.push(Row::new(
                format!(
                    "Unbound: {} in {} has no key (\"{}\" was reassigned)",
                    old_action.name().unwrap_or("action"),
                    ctx.name(),
                    chord.label()
                ),
                "unbound",
            ));
        }
    }

    if clashes.is_empty() {
        let custom = state.config.keybindings.len();
        if custom == 0 {
            vec![Row::new(
                "Keybindings: default keymap (no custom bindings or clashes)",
                "ok",
            )]
        } else {
            vec![Row::new(
                format!("Keybindings: {custom} custom binding(s), no clashes"),
                "ok",
            )]
        }
    } else {
        clashes
    }
}
