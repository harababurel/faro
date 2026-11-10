//! Slash commands in the composer (`#25`): `/mute 2h`, `/shrug`, `/theme dusk`,
//! `/pin`, `/archive`, `/disappear 1h`, `/nick Alice`, `/search …`, and the
//! rest of the palette's actions, typed where you type with inline completion.
//! No protocol invention and no `/call`.

use std::time::Duration;

use faro_core::config::MessageLayout;
use faro_core::domain::ConversationId;
use faro_core::store::NotifyLevel;

use super::event::Effect;
use super::input::Input;
use super::keymap::Action;
use super::palette::{INACTIVE_CHOICES, Level, SettingChoice, current_theme};
use super::state::{ConversationAction, DISAPPEARING_CHOICES, MUTE_CHOICES, Notice, Popup, State};
use super::update::{curation, navigation, palette, presence, selection, sending, settings};
use crate::theme::Theme;

/// ASCII shrug emoticon appended or sent by `/shrug`.
pub const SHRUG: &str = r"¯\_(ツ)_/¯";

/// Maximum number of inline completion candidates shown at once.
const MAX_CANDIDATES: usize = 8;

/// One row in the inline `/command` completion popup above the composer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlashCandidate {
    /// Command (or command + argument) shown on the left (e.g. `"/mute 2h"`).
    pub command: String,
    /// Brief description or argument hint shown on the right.
    pub description: String,
    /// What `Tab` replaces the composer input with.
    pub completion: String,
    /// What `Enter` executes (or `None` if `Enter` should complete into the
    /// composer instead because a required argument is missing).
    pub execute: Option<String>,
}

struct CommandSpec {
    name: &'static str,
    aliases: &'static [&'static str],
    arg_hint: &'static str,
    description: &'static str,
    takes_arg: bool,
    requires_arg: bool,
}

const COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        name: "mute",
        aliases: &[],
        arg_hint: "[duration|forever|off]",
        description: "Mute this conversation (e.g. 2h, 1d, forever)",
        takes_arg: true,
        requires_arg: false,
    },
    CommandSpec {
        name: "unmute",
        aliases: &[],
        arg_hint: "",
        description: "Unmute this conversation",
        takes_arg: false,
        requires_arg: false,
    },
    CommandSpec {
        name: "shrug",
        aliases: &[],
        arg_hint: "[text]",
        description: r"Send ¯\_(ツ)_/¯",
        takes_arg: true,
        requires_arg: false,
    },
    CommandSpec {
        name: "theme",
        aliases: &[],
        arg_hint: "[name]",
        description: "Switch colour theme (e.g. dusk, dark, light)",
        takes_arg: true,
        requires_arg: false,
    },
    CommandSpec {
        name: "pin",
        aliases: &[],
        arg_hint: "",
        description: "Pin this conversation",
        takes_arg: false,
        requires_arg: false,
    },
    CommandSpec {
        name: "unpin",
        aliases: &[],
        arg_hint: "",
        description: "Unpin this conversation",
        takes_arg: false,
        requires_arg: false,
    },
    CommandSpec {
        name: "archive",
        aliases: &[],
        arg_hint: "",
        description: "Archive this conversation",
        takes_arg: false,
        requires_arg: false,
    },
    CommandSpec {
        name: "unarchive",
        aliases: &[],
        arg_hint: "",
        description: "Unarchive this conversation",
        takes_arg: false,
        requires_arg: false,
    },
    CommandSpec {
        name: "hide",
        aliases: &[],
        arg_hint: "[1w|1m|3m|1y]",
        description: "Hide this conversation (or inactive ones)",
        takes_arg: true,
        requires_arg: false,
    },
    CommandSpec {
        name: "unhide",
        aliases: &[],
        arg_hint: "",
        description: "Unhide this conversation",
        takes_arg: false,
        requires_arg: false,
    },
    CommandSpec {
        name: "mentions",
        aliases: &[],
        arg_hint: "[on|off]",
        description: "Notify for mentions only or every message",
        takes_arg: true,
        requires_arg: false,
    },
    CommandSpec {
        name: "disappear",
        aliases: &["timer"],
        arg_hint: "[off|5m|1h|8h|1d|1w|4w]",
        description: "Set disappearing messages timer",
        takes_arg: true,
        requires_arg: false,
    },
    CommandSpec {
        name: "nick",
        aliases: &["nickname"],
        arg_hint: "[name|off]",
        description: "Set or clear a nickname for this person",
        takes_arg: true,
        requires_arg: false,
    },
    CommandSpec {
        name: "search",
        aliases: &["find"],
        arg_hint: "[query]",
        description: "Search messages in this conversation",
        takes_arg: true,
        requires_arg: false,
    },
    CommandSpec {
        name: "attach",
        aliases: &["file"],
        arg_hint: "[path]",
        description: "Attach a file to send",
        takes_arg: true,
        requires_arg: false,
    },
    CommandSpec {
        name: "paste",
        aliases: &[],
        arg_hint: "",
        description: "Paste an image from the clipboard",
        takes_arg: false,
        requires_arg: false,
    },
    CommandSpec {
        name: "links",
        aliases: &["files"],
        arg_hint: "",
        description: "Links & files shared in this chat",
        takes_arg: false,
        requires_arg: false,
    },
    CommandSpec {
        name: "gallery",
        aliases: &["media"],
        arg_hint: "",
        description: "Browse images, stickers, and videos in this chat",
        takes_arg: false,
        requires_arg: false,
    },
    CommandSpec {
        name: "unread",
        aliases: &[],
        arg_hint: "",
        description: "Jump to the unread marker",
        takes_arg: false,
        requires_arg: false,
    },
    CommandSpec {
        name: "verify",
        aliases: &["safety"],
        arg_hint: "",
        description: "Verify safety number",
        takes_arg: false,
        requires_arg: false,
    },
    CommandSpec {
        name: "chat",
        aliases: &["open", "goto"],
        arg_hint: "[name]",
        description: "Switch to another conversation",
        takes_arg: true,
        requires_arg: false,
    },
    CommandSpec {
        name: "me",
        aliases: &[],
        arg_hint: "<text>",
        description: "Send an italicized action message",
        takes_arg: true,
        requires_arg: true,
    },
    CommandSpec {
        name: "dnd",
        aliases: &["bell"],
        arg_hint: "[on|off]",
        description: "Toggle do not disturb (terminal bell)",
        takes_arg: true,
        requires_arg: false,
    },
    CommandSpec {
        name: "layout",
        aliases: &[],
        arg_hint: "[bubbles|irc]",
        description: "Switch message layout",
        takes_arg: true,
        requires_arg: false,
    },
    CommandSpec {
        name: "allow",
        aliases: &[],
        arg_hint: "",
        description: "Add this conversation to the allowlist",
        takes_arg: false,
        requires_arg: false,
    },
    CommandSpec {
        name: "disallow",
        aliases: &[],
        arg_hint: "",
        description: "Remove this conversation from the allowlist",
        takes_arg: false,
        requires_arg: false,
    },
    CommandSpec {
        name: "allowlist",
        aliases: &[],
        arg_hint: "[on|off]",
        description: "Toggle allowlist mode",
        takes_arg: true,
        requires_arg: false,
    },
    CommandSpec {
        name: "info",
        aliases: &[],
        arg_hint: "",
        description: "Toggle conversation info panel",
        takes_arg: false,
        requires_arg: false,
    },
    CommandSpec {
        name: "sidebar",
        aliases: &[],
        arg_hint: "",
        description: "Toggle conversation list panel",
        takes_arg: false,
        requires_arg: false,
    },
    CommandSpec {
        name: "clear",
        aliases: &[],
        arg_hint: "",
        description: "Clear the composer draft",
        takes_arg: false,
        requires_arg: false,
    },
    CommandSpec {
        name: "settings",
        aliases: &[],
        arg_hint: "",
        description: "Open settings",
        takes_arg: false,
        requires_arg: false,
    },
    CommandSpec {
        name: "commands",
        aliases: &[],
        arg_hint: "",
        description: "Open the command palette",
        takes_arg: false,
        requires_arg: false,
    },
    CommandSpec {
        name: "keys",
        aliases: &["help"],
        arg_hint: "",
        description: "Show keybindings",
        takes_arg: false,
        requires_arg: false,
    },
    CommandSpec {
        name: "about",
        aliases: &[],
        arg_hint: "",
        description: "About faro",
        takes_arg: false,
        requires_arg: false,
    },
    CommandSpec {
        name: "quit",
        aliases: &["q"],
        arg_hint: "",
        description: "Quit faro",
        takes_arg: false,
        requires_arg: false,
    },
];

/// Whether `text` is a slash command (or prefix) rather than a normal chat
/// message or Unix file path.
///
/// A slash input starts with a single `/` (not `//`, which escapes a literal
/// leading slash), stays on one line, and its command word contains only ASCII
/// letters or `-` (so `/tmp/file.png` is treated as normal text/path).
pub fn is_slash_input(text: &str) -> bool {
    let Some(after_slash) = text.strip_prefix('/') else {
        return false;
    };
    if after_slash.starts_with('/') || after_slash.contains('\n') {
        return false;
    }
    let cmd_word = after_slash.split_whitespace().next().unwrap_or("");
    cmd_word
        .chars()
        .all(|c| c.is_ascii_alphabetic() || c == '-')
}

/// Split a slash input (without the leading `/`) into `(cmd, Option<arg>)`.
fn split_slash(after_slash: &str) -> (&str, Option<&str>) {
    match after_slash.find(char::is_whitespace) {
        Some(idx) => (&after_slash[..idx], Some(after_slash[idx..].trim_start())),
        None => (after_slash, None),
    }
}

fn find_spec(cmd: &str) -> Option<&'static CommandSpec> {
    let lower = cmd.to_ascii_lowercase();
    COMMANDS
        .iter()
        .find(|spec| spec.name == lower || spec.aliases.iter().any(|&alias| alias == lower))
}

/// Inline completion candidates for `after_slash` (the text after leading `/`).
pub fn candidates(state: &State, after_slash: &str) -> Vec<SlashCandidate> {
    let (cmd_raw, arg_opt) = split_slash(after_slash);
    let cmd = cmd_raw.to_ascii_lowercase();

    if let Some(arg) = arg_opt {
        let Some(spec) = find_spec(&cmd) else {
            return Vec::new();
        };
        return arg_candidates(state, spec.name, arg);
    }

    let mut ranked: Vec<(u8, usize, SlashCandidate)> = Vec::new();
    for (idx, spec) in COMMANDS.iter().enumerate() {
        if !command_available(state, spec.name) {
            continue;
        }
        let tier = if cmd.is_empty() || spec.name == cmd {
            Some(0)
        } else if spec.aliases.iter().any(|&a| a == cmd) {
            Some(1)
        } else if spec.name.starts_with(&cmd) {
            Some(2)
        } else if spec.aliases.iter().any(|a| a.starts_with(&cmd)) {
            Some(3)
        } else if spec.name.contains(&cmd) || spec.description.to_ascii_lowercase().contains(&cmd) {
            Some(4)
        } else {
            None
        };
        let Some(tier) = tier else {
            continue;
        };
        let command = format!("/{}", spec.name);
        let description = if spec.arg_hint.is_empty() {
            spec.description.to_owned()
        } else {
            format!("{} · {}", spec.arg_hint, spec.description)
        };
        let completion = if spec.takes_arg {
            format!("/{} ", spec.name)
        } else {
            format!("/{}", spec.name)
        };
        let execute = (!spec.requires_arg).then(|| format!("/{}", spec.name));
        ranked.push((
            tier,
            idx,
            SlashCandidate {
                command,
                description,
                completion,
                execute,
            },
        ));
    }
    ranked.sort_by_key(|(tier, idx, _)| (*tier, *idx));
    ranked
        .into_iter()
        .take(MAX_CANDIDATES)
        .map(|(_, _, c)| c)
        .collect()
}

fn command_available(state: &State, name: &str) -> bool {
    let Some(open) = &state.open else {
        return false;
    };
    match name {
        "disappear" => matches!(open.id, ConversationId::Direct(peer) if peer != state.me),
        _ => true,
    }
}

fn arg_candidates(state: &State, cmd: &str, arg: &str) -> Vec<SlashCandidate> {
    let q = arg.trim().to_ascii_lowercase();
    match cmd {
        "mute" => mute_arg_candidates(state, &q),
        "theme" => theme_arg_candidates(state, &q),
        "disappear" => disappear_arg_candidates(state, &q),
        "mentions" => toggle_arg_candidates(
            "mentions",
            &q,
            &[
                ("on", "Notify for mentions only"),
                ("off", "Notify for every message"),
            ],
        ),
        "dnd" => toggle_arg_candidates(
            "dnd",
            &q,
            &[
                ("on", "Turn do not disturb on (bell off)"),
                ("off", "Turn do not disturb off (bell on)"),
            ],
        ),
        "allowlist" => toggle_arg_candidates(
            "allowlist",
            &q,
            &[
                ("on", "Turn allowlist mode on"),
                ("off", "Turn allowlist mode off"),
            ],
        ),
        "layout" => toggle_arg_candidates(
            "layout",
            &q,
            &[
                ("bubbles", "Bubbles message layout"),
                ("irc", "IRC message layout"),
            ],
        ),
        "hide" => hide_arg_candidates(&q),
        "chat" => chat_arg_candidates(state, &q),
        _ => Vec::new(),
    }
}

fn mute_arg_candidates(state: &State, q: &str) -> Vec<SlashCandidate> {
    const PRESETS: &[(&str, &str)] = &[
        ("1h", "Mute for 1 hour"),
        ("2h", "Mute for 2 hours"),
        ("8h", "Mute for 8 hours"),
        ("1d", "Mute for 1 day"),
        ("1w", "Mute for 1 week"),
        ("forever", "Mute until unmuted"),
        ("off", "Unmute this conversation"),
    ];
    let mut out = Vec::new();
    if !q.is_empty()
        && !PRESETS.iter().any(|(token, _)| *token == q)
        && let Some((_, label)) = parse_duration(q)
    {
        let full = format!("/mute {q}");
        out.push(SlashCandidate {
            command: full.clone(),
            description: format!("Mute for {label}"),
            completion: full.clone(),
            execute: Some(full),
        });
    }
    let muted = state
        .open
        .as_ref()
        .and_then(|o| state.summary(o.id))
        .is_some_and(|s| s.state.muted_at(sending::wall_clock()));
    for &(token, desc) in PRESETS {
        if token == "off" && !muted && q.is_empty() {
            continue;
        }
        if q.is_empty() || token.starts_with(q) || desc.to_ascii_lowercase().contains(q) {
            let full = format!("/mute {token}");
            out.push(SlashCandidate {
                command: full.clone(),
                description: desc.to_owned(),
                completion: full.clone(),
                execute: Some(full),
            });
        }
    }
    out.truncate(MAX_CANDIDATES);
    out
}

fn theme_arg_candidates(state: &State, q: &str) -> Vec<SlashCandidate> {
    let current = current_theme(state);
    let all = Theme::all(&state.user_themes);
    let mut ranked: Vec<(u8, usize, SlashCandidate)> = Vec::new();
    for (idx, theme) in all.into_iter().enumerate() {
        let name = theme.name.as_str();
        let lower = name.to_ascii_lowercase();
        let short = lower.strip_prefix("faro-");
        let tier = if q.is_empty() {
            Some(1)
        } else if lower == q || short == Some(q) {
            Some(0)
        } else if lower.starts_with(q) || short.is_some_and(|s| s.starts_with(q)) {
            Some(1)
        } else if lower.contains(q) {
            Some(2)
        } else {
            None
        };
        let Some(tier) = tier else {
            continue;
        };
        let mut details = Vec::new();
        if name == current {
            details.push("current");
        }
        if let Some(src) = theme.source.label() {
            details.push(src);
        }
        let full = format!("/theme {name}");
        ranked.push((
            tier,
            idx,
            SlashCandidate {
                command: full.clone(),
                description: details.join(" · "),
                completion: full.clone(),
                execute: Some(full),
            },
        ));
    }
    ranked.sort_by_key(|(tier, idx, _)| (*tier, *idx));
    ranked
        .into_iter()
        .take(MAX_CANDIDATES)
        .map(|(_, _, c)| c)
        .collect()
}

fn disappear_arg_candidates(state: &State, q: &str) -> Vec<SlashCandidate> {
    const PRESETS: &[(&str, &str, Option<u32>)] = &[
        ("off", "Turn off disappearing messages", None),
        ("5m", "Disappear after 5 minutes", Some(5 * 60)),
        ("1h", "Disappear after 1 hour", Some(60 * 60)),
        ("8h", "Disappear after 8 hours", Some(8 * 60 * 60)),
        ("1d", "Disappear after 1 day", Some(24 * 60 * 60)),
        ("1w", "Disappear after 1 week", Some(7 * 24 * 60 * 60)),
        ("4w", "Disappear after 4 weeks", Some(28 * 24 * 60 * 60)),
    ];
    let current = state
        .open
        .as_ref()
        .and_then(|o| state.summary(o.id))
        .and_then(|s| s.disappearing_timer);
    PRESETS
        .iter()
        .filter(|&&(token, desc, _)| {
            q.is_empty() || token.starts_with(q) || desc.to_ascii_lowercase().contains(q)
        })
        .map(|&(token, desc, secs)| {
            let full = format!("/disappear {token}");
            let description = if secs == current {
                format!("{desc} · current")
            } else {
                desc.to_owned()
            };
            SlashCandidate {
                command: full.clone(),
                description,
                completion: full.clone(),
                execute: Some(full),
            }
        })
        .collect()
}

fn toggle_arg_candidates(
    cmd: &str,
    q: &str,
    options: &[(&'static str, &'static str)],
) -> Vec<SlashCandidate> {
    options
        .iter()
        .filter(|&&(token, desc)| {
            q.is_empty() || token.starts_with(q) || desc.to_ascii_lowercase().contains(q)
        })
        .map(|&(token, desc)| {
            let full = format!("/{cmd} {token}");
            SlashCandidate {
                command: full.clone(),
                description: desc.to_owned(),
                completion: full.clone(),
                execute: Some(full),
            }
        })
        .collect()
}

fn hide_arg_candidates(q: &str) -> Vec<SlashCandidate> {
    const OPTIONS: &[(&str, &str)] = &[
        ("1w", "Hide conversations inactive for 1 week"),
        ("1m", "Hide conversations inactive for 1 month"),
        ("3m", "Hide conversations inactive for 3 months"),
        ("1y", "Hide conversations inactive for 1 year"),
    ];
    let mut out = Vec::new();
    if q.is_empty() {
        out.push(SlashCandidate {
            command: "/hide".to_owned(),
            description: "Hide this conversation".to_owned(),
            completion: "/hide".to_owned(),
            execute: Some("/hide".to_owned()),
        });
    }
    for &(token, desc) in OPTIONS {
        if q.is_empty() || token.starts_with(q) || desc.to_ascii_lowercase().contains(q) {
            let full = format!("/hide {token}");
            out.push(SlashCandidate {
                command: full.clone(),
                description: desc.to_owned(),
                completion: full.clone(),
                execute: Some(full),
            });
        }
    }
    out
}

fn chat_arg_candidates(state: &State, q: &str) -> Vec<SlashCandidate> {
    let open_id = state.open.as_ref().map(|o| o.id);
    state
        .conversations
        .iter()
        .filter(|c| Some(c.id) != open_id)
        .filter_map(|c| {
            let title = state.conversation_title(c.id);
            let lower = title.to_lowercase();
            (q.is_empty() || lower.contains(q)).then(|| {
                let full = format!("/chat {title}");
                let detail = if c.unread > 0 {
                    format!("Open {title} ({} unread)", c.unread)
                } else {
                    format!("Open {title}")
                };
                SlashCandidate {
                    command: full.clone(),
                    description: detail,
                    completion: full.clone(),
                    execute: Some(full),
                }
            })
        })
        .take(MAX_CANDIDATES)
        .collect()
}

/// Parse a compact or human duration (`"2h"`, `"30m"`, `"1d"`, `"1w"`,
/// `"2 hours"`, `"15 min"`) into a [`Duration`] and display label.
pub fn parse_duration(raw: &str) -> Option<(Duration, String)> {
    let trimmed = raw.trim().to_ascii_lowercase();
    if trimmed.is_empty() {
        return None;
    }
    let split = trimmed
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(trimmed.len());
    let (num_str, unit_raw) = trimmed.split_at(split);
    let count: u64 = num_str.parse().ok()?;
    if count == 0 || count > 3650 {
        return None;
    }
    let unit = unit_raw.trim();
    match unit {
        "m" | "min" | "mins" | "minute" | "minutes" => {
            let label = if count == 1 {
                "1 minute".to_owned()
            } else if count == 60 {
                "1 hour".to_owned()
            } else {
                format!("{count} minutes")
            };
            Some((Duration::from_secs(count.saturating_mul(60)), label))
        }
        "" | "h" | "hr" | "hrs" | "hour" | "hours" => {
            let label = if count == 1 {
                "1 hour".to_owned()
            } else {
                format!("{count} hours")
            };
            Some((Duration::from_secs(count.saturating_mul(3600)), label))
        }
        "d" | "day" | "days" => {
            let label = if count == 1 {
                "1 day".to_owned()
            } else if count == 7 {
                "1 week".to_owned()
            } else {
                format!("{count} days")
            };
            Some((Duration::from_secs(count.saturating_mul(86_400)), label))
        }
        "w" | "wk" | "wks" | "week" | "weeks" => {
            let label = if count == 1 {
                "1 week".to_owned()
            } else {
                format!("{count} weeks")
            };
            Some((Duration::from_secs(count.saturating_mul(7 * 86_400)), label))
        }
        _ => None,
    }
}

/// Clear the composer's slash command input and any saved draft or typing state.
fn consume_input(state: &mut State) -> Vec<Effect> {
    let Some(open) = state.open.as_mut() else {
        return vec![];
    };
    let id = open.id;
    open.input = Input::default();
    open.draft_edited = None;
    open.slash_selected = 0;
    open.slash_dismissed_for = None;
    open.mention_selected = 0;
    open.mention_dismissed_at = None;
    open.shortcode_selected = 0;
    open.shortcode_dismissed_at = None;
    let mut effects = presence::stop_typing(state);
    effects.extend(sending::clear_draft(state, id));
    effects
}

/// Execute the slash command currently in `open.input`.
pub fn execute_from_input(state: &mut State) -> Vec<Effect> {
    let Some(raw) = state
        .open
        .as_ref()
        .map(|o| o.input.text().trim().to_owned())
    else {
        return vec![];
    };
    execute(state, &raw)
}

/// Parse and execute `raw` (a string starting with `/`).
#[expect(
    clippy::too_many_lines,
    reason = "flat dispatch table over every composer slash command"
)]
pub fn execute(state: &mut State, raw: &str) -> Vec<Effect> {
    let Some(after_slash) = raw.trim().strip_prefix('/') else {
        return vec![];
    };
    let (cmd_raw, arg_opt) = split_slash(after_slash);
    let arg = arg_opt.map_or("", str::trim);
    let Some(spec) = find_spec(cmd_raw) else {
        state.notice = Some(Notice::warning(format!(
            "Unknown command \"/{cmd_raw}\" (type / for commands, or // to send a message starting with /)"
        )));
        return vec![];
    };

    let Some(conversation) = state.open.as_ref().map(|o| o.id) else {
        return vec![];
    };

    match spec.name {
        "shrug" => {
            let body = if arg.is_empty() {
                SHRUG.to_owned()
            } else {
                format!("{arg} {SHRUG}")
            };
            if let Some(open) = state.open.as_mut() {
                open.input = Input::new(&body);
                open.slash_selected = 0;
                open.slash_dismissed_for = None;
            }
            sending::send_message(state)
        }
        "me" => {
            if arg.is_empty() {
                if let Some(open) = state.open.as_mut() {
                    open.input = Input::new("/me ");
                    open.slash_selected = 0;
                    open.slash_dismissed_for = None;
                }
                return vec![];
            }
            let body = format!("*{arg}*");
            if let Some(open) = state.open.as_mut() {
                open.input = Input::new(&body);
                open.slash_selected = 0;
                open.slash_dismissed_for = None;
            }
            sending::send_message(state)
        }
        "clear" => {
            let effects = consume_input(state);
            if let Some(open) = state.open.as_mut() {
                open.reply_to = None;
                open.staged_attachment = None;
                open.mentions.clear();
            }
            state.notice = Some(Notice::info("Draft cleared"));
            effects
        }
        "mute" => exec_mute(state, conversation, arg),
        "unmute" => {
            let mut effects = consume_input(state);
            effects.extend(curation::perform(
                state,
                conversation,
                ConversationAction::Unmute,
            ));
            effects
        }
        "pin" => {
            let mut effects = consume_input(state);
            effects.extend(curation::perform(
                state,
                conversation,
                ConversationAction::Pin,
            ));
            effects
        }
        "unpin" => {
            let mut effects = consume_input(state);
            effects.extend(curation::perform(
                state,
                conversation,
                ConversationAction::Unpin,
            ));
            effects
        }
        "archive" => {
            let mut effects = consume_input(state);
            effects.extend(curation::perform(
                state,
                conversation,
                ConversationAction::Archive,
            ));
            effects
        }
        "unarchive" => {
            let mut effects = consume_input(state);
            effects.extend(curation::perform(
                state,
                conversation,
                ConversationAction::Unarchive,
            ));
            effects
        }
        "hide" => exec_hide(state, conversation, arg),
        "unhide" => {
            let mut effects = consume_input(state);
            effects.extend(curation::perform(
                state,
                conversation,
                ConversationAction::Unhide,
            ));
            effects
        }
        "mentions" => exec_mentions(state, conversation, arg),
        "disappear" => exec_disappear(state, conversation, arg),
        "nick" => exec_nick(state, conversation, arg),
        "verify" => {
            let mut effects = consume_input(state);
            let available = match conversation {
                ConversationId::Direct(peer) => {
                    peer != state.me
                        && state
                            .identities
                            .get(&peer)
                            .and_then(|i| i.safety_number.as_ref())
                            .is_some()
                }
                ConversationId::Group(_) => !state.changed_identities(conversation).is_empty(),
            };
            if available {
                effects.extend(curation::perform(
                    state,
                    conversation,
                    ConversationAction::SafetyNumber,
                ));
            } else {
                state.notice = Some(Notice::info(
                    "No safety number is recorded for this conversation yet",
                ));
            }
            effects
        }
        "theme" => exec_theme(state, arg),
        "search" => {
            let mut effects = consume_input(state);
            effects.extend(palette::open_with_query(
                state,
                Level::Search(Some(conversation)),
                arg,
            ));
            effects
        }
        "attach" => exec_attach(state, arg),
        "paste" => {
            let mut effects = consume_input(state);
            effects.extend(sending::paste_image(state));
            effects
        }
        "links" => {
            let effects = consume_input(state);
            palette::open(state, Level::ChatLinks);
            effects
        }
        "gallery" => {
            let mut effects = consume_input(state);
            effects.extend(selection::open_media_gallery(state));
            effects
        }
        "unread" => {
            let mut effects = consume_input(state);
            effects.extend(selection::jump_unread(state));
            effects
        }
        "chat" => exec_chat(state, arg),
        "dnd" => exec_dnd(state, arg),
        "layout" => exec_layout(state, arg),
        "allow" => {
            let mut effects = consume_input(state);
            effects.extend(curation::perform(
                state,
                conversation,
                ConversationAction::Allow,
            ));
            effects
        }
        "disallow" => {
            let mut effects = consume_input(state);
            effects.extend(curation::perform(
                state,
                conversation,
                ConversationAction::Disallow,
            ));
            effects
        }
        "allowlist" => exec_allowlist(state, arg),
        "info" => {
            let mut effects = consume_input(state);
            effects.extend(navigation::on_action(state, Action::ToggleRightSidebar));
            effects
        }
        "sidebar" => {
            let mut effects = consume_input(state);
            effects.extend(navigation::on_action(state, Action::ToggleLeftSidebar));
            effects
        }
        "settings" => {
            let effects = consume_input(state);
            palette::open(state, Level::Settings);
            effects
        }
        "commands" => {
            let effects = consume_input(state);
            palette::open(state, Level::Commands);
            effects
        }
        "keys" => {
            let effects = consume_input(state);
            palette::open(state, Level::Keys);
            effects
        }
        "about" => {
            let effects = consume_input(state);
            state.popup = Some(Popup::About);
            effects
        }
        "quit" => {
            let mut effects = consume_input(state);
            effects.extend(navigation::on_action(state, Action::Quit));
            effects
        }
        _ => vec![],
    }
}

fn exec_mute(state: &mut State, conversation: ConversationId, arg: &str) -> Vec<Effect> {
    let lower = arg.trim().to_ascii_lowercase();
    if lower.is_empty() {
        let mut effects = consume_input(state);
        let muted = state
            .summary(conversation)
            .is_some_and(|s| s.state.muted_at(sending::wall_clock()));
        let action = if muted {
            ConversationAction::Unmute
        } else {
            ConversationAction::Mute
        };
        effects.extend(curation::perform(state, conversation, action));
        return effects;
    }
    if matches!(lower.as_str(), "off" | "none" | "unmute" | "0") {
        let mut effects = consume_input(state);
        effects.extend(curation::perform(
            state,
            conversation,
            ConversationAction::Unmute,
        ));
        return effects;
    }
    if matches!(
        lower.as_str(),
        "forever" | "always" | "indefinitely" | "until unmuted"
    ) {
        let mut effects = consume_input(state);
        effects.extend(curation::mute_for_duration(state, conversation, None, ""));
        return effects;
    }
    if let Some((duration, label)) = parse_duration(&lower) {
        let mut effects = consume_input(state);
        effects.extend(curation::mute_for_duration(
            state,
            conversation,
            Some(duration),
            &label,
        ));
        return effects;
    }
    for (i, &(label, _)) in MUTE_CHOICES.iter().enumerate() {
        if label.eq_ignore_ascii_case(&lower) {
            let mut effects = consume_input(state);
            effects.extend(curation::mute_for(state, conversation, i));
            return effects;
        }
    }
    state.notice = Some(Notice::warning(format!(
        "Unknown mute duration \"{arg}\" (try 2h, 8h, 1d, 1w, forever, or off)"
    )));
    vec![]
}

fn exec_hide(state: &mut State, conversation: ConversationId, arg: &str) -> Vec<Effect> {
    let lower = arg.trim().to_ascii_lowercase();
    if lower.is_empty() {
        let mut effects = consume_input(state);
        effects.extend(curation::perform(
            state,
            conversation,
            ConversationAction::Hide,
        ));
        return effects;
    }
    let choice = match lower.as_str() {
        "1w" | "7d" | "1 week" => Some(0),
        "1m" | "30d" | "1 month" => Some(1),
        "3m" | "90d" | "3 months" => Some(2),
        "1y" | "365d" | "1 year" => Some(3),
        _ => INACTIVE_CHOICES
            .iter()
            .position(|(_, label)| label.eq_ignore_ascii_case(&lower)),
    };
    let Some(choice) = choice else {
        state.notice = Some(Notice::warning(format!(
            "Unknown /hide argument \"{arg}\" (use /hide, or /hide 1w, 1m, 3m, 1y)"
        )));
        return vec![];
    };
    let mut effects = consume_input(state);
    effects.extend(curation::hide_inactive(state, choice));
    effects
}

fn exec_mentions(state: &mut State, conversation: ConversationId, arg: &str) -> Vec<Effect> {
    let lower = arg.trim().to_ascii_lowercase();
    let action = match lower.as_str() {
        "" => state.summary(conversation).map(|s| match s.state.notify {
            NotifyLevel::All => ConversationAction::MentionsOnly,
            NotifyLevel::Mentions => ConversationAction::AllMessages,
        }),
        "on" | "only" | "mentions" | "true" => Some(ConversationAction::MentionsOnly),
        "off" | "all" | "every" | "false" => Some(ConversationAction::AllMessages),
        _ => {
            state.notice = Some(Notice::warning(format!(
                "Unknown /mentions option \"{arg}\" (use on or off)"
            )));
            return vec![];
        }
    };
    let mut effects = consume_input(state);
    if let Some(action) = action {
        effects.extend(curation::perform(state, conversation, action));
    }
    effects
}

fn exec_disappear(state: &mut State, conversation: ConversationId, arg: &str) -> Vec<Effect> {
    if !matches!(conversation, ConversationId::Direct(peer) if peer != state.me) {
        state.notice = Some(Notice::info(
            "Disappearing messages can only be changed in direct chats",
        ));
        return vec![];
    }
    let lower = arg.trim().to_ascii_lowercase();
    if lower.is_empty() {
        let mut effects = consume_input(state);
        effects.extend(curation::perform(
            state,
            conversation,
            ConversationAction::DisappearingTimer,
        ));
        return effects;
    }
    let choice = match lower.as_str() {
        "off" | "0" | "none" => Some(0),
        "5m" | "5 min" | "5 minutes" => Some(1),
        "1h" | "60m" | "1 hour" => Some(2),
        "8h" | "8 hours" => Some(3),
        "1d" | "24h" | "1 day" => Some(4),
        "1w" | "7d" | "1 week" => Some(5),
        "4w" | "28d" | "4 weeks" => Some(6),
        _ => DISAPPEARING_CHOICES
            .iter()
            .position(|(label, _)| label.eq_ignore_ascii_case(&lower)),
    };
    let Some(choice) = choice else {
        state.notice = Some(Notice::warning(format!(
            "Unknown timer \"{arg}\" (use off, 5m, 1h, 8h, 1d, 1w, or 4w)"
        )));
        return vec![];
    };
    let mut effects = consume_input(state);
    effects.extend(curation::set_disappearing_timer(
        state,
        conversation,
        choice,
    ));
    effects
}

fn exec_nick(state: &mut State, conversation: ConversationId, arg: &str) -> Vec<Effect> {
    let ConversationId::Direct(peer) = conversation else {
        let effects = consume_input(state);
        palette::open(state, Level::People);
        return effects;
    };
    if peer == state.me {
        let effects = consume_input(state);
        palette::open(state, Level::People);
        return effects;
    }
    let trimmed = arg.trim();
    if trimmed.is_empty() {
        let effects = consume_input(state);
        palette::open_nickname(state, peer);
        return effects;
    }
    let mut effects = consume_input(state);
    let nickname = (!matches!(
        trimmed.to_ascii_lowercase().as_str(),
        "off" | "clear" | "none"
    ))
    .then(|| trimmed.to_owned());
    effects.extend(palette::set_nickname(state, peer, nickname));
    effects
}

fn exec_theme(state: &mut State, arg: &str) -> Vec<Effect> {
    let query = arg.trim();
    if query.is_empty() {
        let effects = consume_input(state);
        palette::open(state, Level::Themes);
        return effects;
    }
    let lower = query.to_ascii_lowercase();
    let faro_prefixed = format!("faro-{lower}");
    let all = Theme::all(&state.user_themes);
    let matched = all
        .iter()
        .find(|t| t.name.eq_ignore_ascii_case(&lower))
        .or_else(|| {
            all.iter()
                .find(|t| t.name.eq_ignore_ascii_case(&faro_prefixed))
        })
        .or_else(|| {
            let prefixes: Vec<_> = all
                .iter()
                .filter(|t| {
                    let n = t.name.to_ascii_lowercase();
                    n.starts_with(&lower)
                        || n.strip_prefix("faro-")
                            .is_some_and(|s| s.starts_with(&lower))
                })
                .collect();
            if let [only] = prefixes.as_slice() {
                Some(*only)
            } else {
                None
            }
        })
        .map(|t| t.name.clone());

    let Some(name) = matched else {
        state.notice = Some(Notice::warning(format!(
            "No theme matching \"{query}\" (try /theme dusk, dark, light, or system)"
        )));
        return vec![];
    };
    let mut effects = consume_input(state);
    effects.extend(settings::choose_theme(state, &name));
    effects
}

fn exec_attach(state: &mut State, arg: &str) -> Vec<Effect> {
    let trimmed = arg.trim();
    if trimmed.is_empty() {
        let mut effects = consume_input(state);
        effects.extend(palette::open_with_query(state, Level::AttachFile, "~/"));
        return effects;
    }
    let mut effects = consume_input(state);
    let path = super::files::expand_tilde(trimmed);
    effects.push(Effect::StageFile(path));
    effects
}

fn exec_chat(state: &mut State, arg: &str) -> Vec<Effect> {
    let query = arg.trim();
    if query.is_empty() {
        let effects = consume_input(state);
        palette::open(state, Level::Chats);
        return effects;
    }
    let lower = query.to_lowercase();
    let exact = state
        .conversations
        .iter()
        .find(|c| state.conversation_title(c.id).to_lowercase() == lower)
        .map(|c| c.id);
    let target = exact
        .or_else(|| {
            state
                .conversations
                .iter()
                .find(|c| {
                    state
                        .conversation_title(c.id)
                        .to_lowercase()
                        .starts_with(&lower)
                })
                .map(|c| c.id)
        })
        .or_else(|| {
            state
                .conversations
                .iter()
                .find(|c| {
                    state
                        .conversation_title(c.id)
                        .to_lowercase()
                        .contains(&lower)
                })
                .map(|c| c.id)
        });
    let Some(id) = target else {
        state.notice = Some(Notice::warning(format!(
            "No conversation matching \"{query}\""
        )));
        return vec![];
    };
    let mut effects = consume_input(state);
    effects.extend(palette::show_conversation(state, id));
    effects
}

fn exec_dnd(state: &mut State, arg: &str) -> Vec<Effect> {
    let lower = arg.trim().to_ascii_lowercase();
    let bell_on = match lower.as_str() {
        "" => !state.config.notifications.bell,
        "on" | "true" => false,
        "off" | "false" => true,
        _ => {
            state.notice = Some(Notice::warning(format!(
                "Unknown /dnd option \"{arg}\" (use on or off)"
            )));
            return vec![];
        }
    };
    let mut effects = consume_input(state);
    effects.extend(settings::choose(state, SettingChoice::Bell(bell_on)));
    effects
}

fn exec_layout(state: &mut State, arg: &str) -> Vec<Effect> {
    let lower = arg.trim().to_ascii_lowercase();
    let layout = match lower.as_str() {
        "" => match state.layout {
            MessageLayout::Bubbles => MessageLayout::Irc,
            MessageLayout::Irc => MessageLayout::Bubbles,
        },
        "bubbles" | "bubble" => MessageLayout::Bubbles,
        "irc" => MessageLayout::Irc,
        _ => {
            state.notice = Some(Notice::warning(format!(
                "Unknown /layout \"{arg}\" (use bubbles or irc)"
            )));
            return vec![];
        }
    };
    let mut effects = consume_input(state);
    effects.extend(settings::choose(state, SettingChoice::Layout(layout)));
    effects
}

fn exec_allowlist(state: &mut State, arg: &str) -> Vec<Effect> {
    let lower = arg.trim().to_ascii_lowercase();
    let on = match lower.as_str() {
        "" => !state.list.allowlist,
        "on" | "true" => true,
        "off" | "false" => false,
        _ => {
            state.notice = Some(Notice::warning(format!(
                "Unknown /allowlist option \"{arg}\" (use on or off)"
            )));
            return vec![];
        }
    };
    let mut effects = consume_input(state);
    effects.extend(curation::set_allowlist_mode(state, on));
    effects
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distinguishes_slash_commands_from_paths_and_escaped_slashes() {
        assert!(is_slash_input("/"));
        assert!(is_slash_input("/mute"));
        assert!(is_slash_input("/mute 2h"));
        assert!(is_slash_input("/theme dusk"));
        assert!(is_slash_input("/shrug hello / world"));
        assert!(!is_slash_input("//mute"));
        assert!(!is_slash_input("/tmp/photo.jpg"));
        assert!(!is_slash_input("/usr/local/bin check"));
        assert!(!is_slash_input("hello /mute"));
        assert!(!is_slash_input("/mute\n2h"));
    }

    #[test]
    fn parses_compact_and_human_durations() {
        assert_eq!(
            parse_duration("30m"),
            Some((Duration::from_mins(30), "30 minutes".to_owned()))
        );
        assert_eq!(
            parse_duration("2h"),
            Some((Duration::from_hours(2), "2 hours".to_owned()))
        );
        assert_eq!(
            parse_duration("1d"),
            Some((Duration::from_hours(24), "1 day".to_owned()))
        );
        assert_eq!(
            parse_duration("1w"),
            Some((Duration::from_hours(168), "1 week".to_owned()))
        );
        assert_eq!(parse_duration("0h"), None);
        assert_eq!(parse_duration("abc"), None);
    }
}
