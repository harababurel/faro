//! Linear, screen-reader transcript mode (`--linear`, #48): plain-text,
//! append-only `time sender: text` output with no alternate screen, no 2D box
//! drawing, stable chronological ordering, and no colour dependency.
//!
//! Bidi ordering (RTL scripts such as Arabic and Hebrew) and wide CJK glyphs
//! are left natively to the terminal or assistive technology without cell-grid
//! truncation or padding.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use faro_core::backend::BackendEvent;
use faro_core::domain::{
    AttachmentRef, BodyPart, ConnectionState, ConversationId, DeliveryStatus, Direction, Message,
    MessageRef, Timestamp, mention_parts,
};
use jiff::tz::TimeZone;

use super::event::{AppEvent, Effect};
use super::input::Input;
use super::keymap::Action;
use super::slash;
use super::state::{EmojiMode, Focus, Notice, Popup, SidebarView, State};
use super::update::{self, navigation, selection, sending};

/// Default number of recent messages printed when a conversation opens or
/// when `/history` is run without a count.
pub const INITIAL_HISTORY: usize = 20;

/// Maximum number of messages `/history <n>` prints at once.
const MAX_HISTORY: usize = 200;

/// What Signal puts in a body where someone is mentioned.
const MENTION_PLACEHOLDER: char = '\u{FFFC}';

/// Clean untrusted text for linear output: C0/C1 control characters (`\x1b`,
/// `\x07`, `\r`, `\n`, etc.) cannot reach the terminal, while every printable
/// codepoint (including RTL Arabic/Hebrew, CJK ideographs, and emoji) is kept
/// in its exact logical order.
pub fn clean_text(state: &State, text: &str) -> String {
    let clean: String = text
        .chars()
        .map(|c| match c {
            '\t' => ' ',
            MENTION_PLACEHOLDER => '@',
            c if c.is_control() => '\u{FFFD}',
            c => c,
        })
        .collect();
    if state.env.emoji.mode == EmojiMode::Compatible {
        crate::ui::text::sanitize(&clean).into_owned()
    } else {
        clean
    }
}

/// Collapse a potentially multi-line string into a single sanitized line.
pub fn clean_one_line(state: &State, text: &str) -> String {
    let joined: String = text
        .split(['\r', '\n'])
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    clean_text(state, &joined)
}

/// Format a message timestamp for linear mode (`HH:MM` in `tz`).
///
/// Linear mode is an append-only stream (lines are never redrawn), so it always
/// uses wall-clock `HH:MM` rather than `"now"`/`"5m"` relative labels or
/// omitting the timestamp.
pub fn format_timestamp(sent_at: Timestamp, tz: &TimeZone) -> String {
    format_hm(sent_at, tz)
}

fn format_hm(ts: Timestamp, tz: &TimeZone) -> String {
    let Ok(ms) = i64::try_from(ts.as_millis()) else {
        return "--:--".to_owned();
    };
    let Ok(jiff_ts) = jiff::Timestamp::from_millisecond(ms) else {
        return "--:--".to_owned();
    };
    jiff_ts.to_zoned(tz.clone()).strftime("%H:%M").to_string()
}

/// Format one [`Message`] into plain-text transcript lines (`time sender: text`).
///
/// When `include_chat` is `true` (for background conversations), the chat title
/// is prefixed as `HH:MM [Chat] Sender: text`.
pub fn format_message(
    state: &State,
    message: &Message,
    tz: &TimeZone,
    _now: Timestamp,
    include_chat: bool,
) -> Vec<String> {
    let time = format_timestamp(message.sent_at, tz);
    let chat_prefix = if include_chat {
        format!(
            "[{}] ",
            clean_one_line(state, &state.conversation_title(message.conversation))
        )
    } else {
        String::new()
    };

    if let Some(event) = &message.system_event {
        let sys = clean_one_line(state, &state.system_event_text(message, event));
        return vec![format!("{time} {chat_prefix}* {sys}")];
    }

    let sender = clean_one_line(state, &state.sender_name(message));
    let reply = message
        .quote
        .as_ref()
        .map(|quote| {
            let who = clean_one_line(state, &state.person_name(quote.target.sender));
            let preview = quote
                .preview
                .as_ref()
                .map_or_else(|| "…".to_owned(), |p| clean_one_line(state, p.expose()));
            format!(" (reply to {who}: \"{preview}\")")
        })
        .unwrap_or_default();

    let mut body_lines: Vec<String> = if message.deleted {
        vec!["(This message was deleted)".to_owned()]
    } else if let Some(body) = &message.body {
        let expanded: String = mention_parts(body.expose(), &message.mentions)
            .into_iter()
            .map(|part| match part {
                BodyPart::Text(t) => t.to_owned(),
                BodyPart::Mention(who) => state.mention_name(who),
            })
            .collect();
        expanded
            .split('\n')
            .map(|line| clean_text(state, line.strip_suffix('\r').unwrap_or(line)))
            .collect()
    } else {
        Vec::new()
    };

    let attachment_tags = format_attachment_tags(state, &message.attachments);
    if !attachment_tags.is_empty() {
        let joined = attachment_tags.join(" ");
        match body_lines.last_mut() {
            Some(last) if !last.is_empty() => {
                last.push(' ');
                last.push_str(&joined);
            }
            Some(last) => {
                *last = joined;
            }
            None => body_lines.push(joined),
        }
    }

    if body_lines.is_empty() {
        body_lines.push("(empty message)".to_owned());
    }

    let mut suffix = String::new();
    if message.edited_at.is_some() && !message.deleted {
        suffix.push_str(" (edited)");
    }
    if message.direction == Direction::Outgoing(DeliveryStatus::Failed) {
        suffix.push_str(" (! not sent)");
    }
    if !message.reactions.is_empty() {
        let _ = write!(suffix, " [reactions: {}]", format_reactions(state, message));
    }
    if let Some(last) = body_lines.last_mut() {
        last.push_str(&suffix);
    }

    body_lines
        .into_iter()
        .enumerate()
        .map(|(i, line)| {
            if i == 0 {
                format!("{time} {chat_prefix}{sender}{reply}: {line}")
            } else {
                format!("{time} {chat_prefix}{sender}: {line}")
            }
        })
        .collect()
}

fn format_attachment_tags(state: &State, attachments: &[AttachmentRef]) -> Vec<String> {
    attachments
        .iter()
        .map(|att| {
            if att.is_voice_note() {
                let duration = att
                    .duration_secs
                    .map_or_else(|| "0:00".to_owned(), faro_core::audio::format_duration);
                if let Some(transcript) = att.transcript.as_deref().filter(|t| !t.is_empty()) {
                    let paragraph =
                        clean_one_line(state, &faro_core::domain::transcript_paragraph(transcript));
                    format!("[voice note {duration}: \"{paragraph}\"]")
                } else {
                    format!("[voice note {duration}]")
                }
            } else {
                let name = att.file_name.as_deref().map_or_else(
                    || clean_one_line(state, &att.content_type),
                    |n| clean_one_line(state, n),
                );
                let size = att.size.map(format_size).unwrap_or_default();
                format!("[attachment: {name}{size}]")
            }
        })
        .collect()
}

/// Format the newest `limit` messages from `messages` in stable chronological
/// `(sent_at, sender)` order.
pub fn format_history(
    state: &State,
    messages: &[Message],
    limit: usize,
    tz: &TimeZone,
    now: Timestamp,
) -> Vec<String> {
    let mut ordered: Vec<&Message> = messages.iter().collect();
    ordered.sort_by(|a, b| {
        a.sent_at
            .cmp(&b.sent_at)
            .then_with(|| a.sender.to_string().cmp(&b.sender.to_string()))
    });
    let start = ordered.len().saturating_sub(limit);
    ordered
        .get(start..)
        .unwrap_or(&ordered)
        .iter()
        .flat_map(|m| format_message(state, m, tz, now, false))
        .collect()
}

/// Format a live [`BackendEvent`] after it has been applied to `state`.
#[expect(
    clippy::too_many_lines,
    reason = "flat dispatch table over every BackendEvent variant"
)]
pub fn format_backend_event(
    state: &State,
    event: &BackendEvent,
    prev_connection: ConnectionState,
    tz: &TimeZone,
    now: Timestamp,
) -> Vec<String> {
    match event {
        BackendEvent::Message(message) => {
            if state
                .summary(message.conversation)
                .is_some_and(|s| s.state.blocked)
            {
                return Vec::new();
            }
            let is_open = state.open.as_ref().map(|o| o.id) == Some(message.conversation);
            if state
                .summary(message.conversation)
                .is_some_and(|s| s.state.request)
                && !is_open
            {
                let time = format_timestamp(message.sent_at, tz);
                let who = clean_one_line(state, &state.sender_name(message));
                return vec![format!(
                    "{time} * Message request from {who} (use `/chat {who}` to view)"
                )];
            }
            format_message(state, message, tz, now, !is_open)
        }
        BackendEvent::Reaction {
            conversation,
            target,
            sender,
            emoji,
        } => {
            if state.summary(*conversation).is_some_and(|s| s.state.blocked) {
                return Vec::new();
            }
            let chat_prefix = background_prefix(state, *conversation);
            let who = clean_one_line(state, &state.person_name(*sender));
            let author = clean_one_line(state, &state.person_name(target.sender));
            let target_time = format_hm(target.sent_at, tz);
            match emoji {
                Some(e) => vec![format!(
                    "{chat_prefix}* {who} reacted {} to {author} ({target_time})",
                    clean_one_line(state, e)
                )],
                None => vec![format!(
                    "{chat_prefix}* {who} removed reaction from {author} ({target_time})"
                )],
            }
        }
        BackendEvent::Edited {
            conversation,
            target,
            body,
            mentions,
            edited_at,
        } => {
            if state.summary(*conversation).is_some_and(|s| s.state.blocked) {
                return Vec::new();
            }
            let time = format_timestamp(*edited_at, tz);
            let chat_prefix = background_prefix(state, *conversation);
            let who = clean_one_line(state, &state.person_name(target.sender));
            let expanded: String = mention_parts(body.expose(), mentions)
                .into_iter()
                .map(|part| match part {
                    BodyPart::Text(t) => t.to_owned(),
                    BodyPart::Mention(m) => state.mention_name(m),
                })
                .collect();
            let text = clean_one_line(state, &expanded);
            vec![format!("{time} {chat_prefix}{who} (edited): {text}")]
        }
        BackendEvent::Deleted {
            conversation,
            target,
        } => {
            if state.summary(*conversation).is_some_and(|s| s.state.blocked) {
                return Vec::new();
            }
            let chat_prefix = background_prefix(state, *conversation);
            let who = clean_one_line(state, &state.person_name(target.sender));
            let target_time = format_hm(target.sent_at, tz);
            vec![format!(
                "{chat_prefix}* {who} deleted message ({target_time})"
            )]
        }
        BackendEvent::DeliveryUpdate {
            message,
            status: DeliveryStatus::Failed,
            ..
        } => {
            let time = format_hm(message.sent_at, tz);
            vec![format!("* Message at {time} failed to send")]
        }
        BackendEvent::Connection(conn) => match conn {
            ConnectionState::Disconnected => {
                vec!["* Disconnected".to_owned()]
            }
            ConnectionState::Unlinked => {
                vec!["* Device unlinked — run `faro link` to link again".to_owned()]
            }
            ConnectionState::Connected if prev_connection == ConnectionState::Disconnected => {
                vec!["* Connected".to_owned()]
            }
            ConnectionState::Connecting | ConnectionState::Connected => Vec::new(),
        },
        BackendEvent::DeliveryUpdate {
            status:
                DeliveryStatus::Sending
                | DeliveryStatus::Sent
                | DeliveryStatus::Delivered
                | DeliveryStatus::Read,
            ..
        }
        | BackendEvent::Typing { .. }
        | BackendEvent::ReadElsewhere(_)
        | BackendEvent::ContactUpdated(_)
        | BackendEvent::GroupUpdated(_)
        | BackendEvent::IdentityUpdated(_)
        | BackendEvent::DisappearingTimer { .. }
        | BackendEvent::BlockedSynced { .. }
        | BackendEvent::MessageRequestResponded { .. }
        | BackendEvent::Error(_) => Vec::new(),
    }
}

fn background_prefix(state: &State, conversation: ConversationId) -> String {
    if state.open.as_ref().map(|o| o.id) == Some(conversation) {
        String::new()
    } else {
        format!(
            "[{}] ",
            clean_one_line(state, &state.conversation_title(conversation))
        )
    }
}

/// Format the visible conversation list with 1-based indices, active marker,
/// unread counts, and status tags.
pub fn format_conversations(state: &State, now: Timestamp) -> Vec<String> {
    let open_id = state.open.as_ref().map(|o| o.id);
    let visible: Vec<_> = state.listed_in(SidebarView::Chats).collect();
    if visible.is_empty() {
        return vec!["* No conversations".to_owned()];
    }
    let mut lines = vec!["* Conversations:".to_owned()];
    for (idx, summary) in visible.iter().enumerate() {
        let marker = if Some(summary.id) == open_id {
            "*"
        } else {
            " "
        };
        let title = clean_one_line(state, &state.conversation_title(summary.id));
        let mut tags = Vec::new();
        if summary.unread > 0 {
            tags.push(format!("{} unread", summary.unread));
        }
        if summary.state.pinned.is_some() {
            tags.push("pinned".to_owned());
        }
        if summary.state.muted_at(now) {
            tags.push("muted".to_owned());
        }
        let suffix = if tags.is_empty() {
            String::new()
        } else {
            format!(" ({})", tags.join(", "))
        };
        lines.push(format!("  {}. {marker} {title}{suffix}", idx + 1));
    }
    let requests = state
        .listed_in(SidebarView::Archive)
        .filter(|s| s.state.request)
        .count();
    if requests > 0 {
        let noun = if requests == 1 { "request" } else { "requests" };
        lines.push(format!("  ({requests} pending message {noun})"));
    }
    lines
}

/// Format a summary of the active conversation and connection state.
pub fn format_status(state: &State, now: Timestamp) -> Vec<String> {
    let conn = match state.connection {
        ConnectionState::Connecting => "connecting",
        ConnectionState::Connected => "connected",
        ConnectionState::Disconnected => "disconnected",
        ConnectionState::Unlinked => "unlinked",
    };
    let unread_total = state.unread_total(now);
    let Some(open) = &state.open else {
        return vec![format!(
            "* Status: {conn} · no open conversation · {unread_total} unread total"
        )];
    };
    let title = clean_one_line(state, &state.conversation_title(open.id));
    let mut details = vec![format!("chat: {title}"), format!("connection: {conn}")];
    if let Some(summary) = state.summary(open.id) {
        if let Some(secs) = summary.disappearing_timer {
            details.push(format!(
                "disappearing: {}",
                faro_core::domain::format_disappearing_timer(secs)
            ));
        }
        if summary.state.muted_at(now) {
            details.push("muted".to_owned());
        }
        if summary.state.request {
            details.push("message request".to_owned());
        }
    }
    if unread_total > 0 {
        details.push(format!("{unread_total} unread total"));
    }
    let mut lines = vec![format!("* Status: {}", details.join(" · "))];
    for who in state.changed_identities(open.id) {
        let person = clean_one_line(state, &state.person_name(who));
        lines.push(format!(
            "* Warning: safety number with {person} changed — verify before sending"
        ));
    }
    lines
}

/// Format plain-text help for linear mode.
pub fn format_help() -> Vec<String> {
    vec![
        "* Linear mode commands:".to_owned(),
        "  <text>           Send message to the active chat (use // for a leading /)".to_owned(),
        "  /chats           List conversations with numbers and unread counts".to_owned(),
        "  /chat <#|name>   Switch to a conversation by number or name".to_owned(),
        "  /history [n]     Print the last n messages (default: 20)".to_owned(),
        "  /reply <text>    Reply to the latest message in the active chat".to_owned(),
        "  /react <emoji>   React to the latest message in the active chat".to_owned(),
        "  /shrug [text]    Send ¯\\_(ツ)_/¯".to_owned(),
        "  /me <action>     Send an italicized action message".to_owned(),
        "  /mute [2h|off]   Mute or unmute the active chat".to_owned(),
        "  /disappear [1h]  Set disappearing messages timer (direct chats)".to_owned(),
        "  /pin, /unpin     Pin or unpin the active chat".to_owned(),
        "  /archive         Archive the active chat".to_owned(),
        "  /status          Show active chat and connection status".to_owned(),
        "  /quit            Quit faro".to_owned(),
    ]
}

/// Take and format any pending notice on `state`.
pub fn drain_notice(state: &mut State) -> Option<String> {
    let Notice { text, .. } = state.notice.take()?;
    Some(format!("* {}", clean_one_line(state, &text)))
}

/// Whether initial conversation/message loading is still in progress.
pub fn is_loading(state: &State) -> bool {
    state.loading || state.open.as_ref().is_some_and(|o| o.loading)
}

/// Whether the open conversation has an outgoing message currently in flight (`Sending`).
pub fn has_sending(state: &State) -> bool {
    state.open.as_ref().is_some_and(|open| {
        open.messages
            .iter()
            .any(|m| m.direction == Direction::Outgoing(DeliveryStatus::Sending))
    })
}

/// Pick the initial conversation to open when `AppEvent::Loaded` (or initial
/// backlog sync) completes and none is open yet: the remembered cursor if
/// listed/non-empty, else the first visible chat in `SidebarView::Chats`.
pub fn choose_initial_conversation(state: &State) -> Option<ConversationId> {
    if state.open.is_some() {
        return None;
    }
    state
        .list
        .cursor
        .filter(|id| state.summary(*id).is_some_and(|s| s.last_activity.is_some()))
        .or_else(|| state.listed_in(SidebarView::Chats).next().map(|c| c.id))
}

/// Process one [`AppEvent`] through the reducer and return both the resulting
/// [`Effect`]s and the transcript lines to emit.
pub fn on_app_event(
    state: &mut State,
    event: AppEvent,
    printed_banner: &mut bool,
    tz: &TimeZone,
    now: Timestamp,
) -> (Vec<Effect>, Vec<String>) {
    let prev_connection = state.connection;
    let backend_snapshot = if let AppEvent::Backend(b) = &event
        && !state.loading
    {
        Some(b.clone())
    } else {
        None
    };
    let loaded_event = matches!(event, AppEvent::Loaded { .. });
    let sync_checkpoint = matches!(
        backend_snapshot,
        Some(
            BackendEvent::ReadElsewhere(_)
                | BackendEvent::Connection(
                    ConnectionState::Disconnected | ConnectionState::Unlinked
                )
        )
    ) || matches!(event, AppEvent::Tick);
    let messages_loaded_for = if let AppEvent::MessagesLoaded { conversation, .. } = &event {
        Some(*conversation)
    } else {
        None
    };

    let mut effects = update::update(state, event);
    let mut lines = Vec::new();

    if loaded_event || (!state.loading && state.open.is_none() && !*printed_banner && sync_checkpoint) {
        if let Some(id) = choose_initial_conversation(state) {
            effects.extend(update::open_conversation(state, id));
        } else if state.open.is_none()
            && !*printed_banner
            && state.connection != ConnectionState::Connecting
        {
            *printed_banner = true;
            lines.push("# faro linear — no open conversation (type /help for commands)".to_owned());
        }
    }

    if let Some(conv_id) = messages_loaded_for
        && state
            .open
            .as_ref()
            .is_some_and(|o| o.id == conv_id && !o.loading)
    {
        let title = clean_one_line(state, &state.conversation_title(conv_id));
        if *printed_banner {
            lines.push(format!("# Chat: {title}"));
        } else {
            *printed_banner = true;
            lines.push(format!(
                "# faro linear — Chat: {title} (type /help for commands)"
            ));
        }
        if let Some(open) = &state.open {
            lines.extend(format_history(
                state,
                &open.messages,
                INITIAL_HISTORY,
                tz,
                now,
            ));
        }
    }

    if *printed_banner && let Some(backend_event) = &backend_snapshot {
        lines.extend(format_backend_event(
            state,
            backend_event,
            prev_connection,
            tz,
            now,
        ));
    }

    if let Some(notice_line) = drain_notice(state) {
        lines.push(notice_line);
    }

    // Keep linear mode free of modal TUI overlays so `seeing_open()` stays true.
    state.palette = None;
    state.popup = None;
    state.focus = Focus::Input;

    (effects, lines)
}

/// Process one line of user input from stdin in linear mode, returning the
/// [`Effect`]s to dispatch and the plain-text lines to print.
#[expect(
    clippy::too_many_lines,
    reason = "flat dispatch over linear mode commands and plain-text message sending"
)]
pub fn handle_line(
    state: &mut State,
    raw_line: &str,
    tz: &TimeZone,
    now: Timestamp,
) -> (Vec<Effect>, Vec<String>) {
    let trimmed = raw_line.trim();
    if trimmed.is_empty() {
        return (Vec::new(), Vec::new());
    }

    let prev_sent_at = state.last_sent_at;
    let mut effects = Vec::new();
    let mut lines = Vec::new();

    if let Some(after_slash) = trimmed.strip_prefix('/')
        && !after_slash.starts_with('/')
    {
        let (cmd_raw, arg_opt) = match after_slash.find(char::is_whitespace) {
            Some(idx) => (
                after_slash.get(..idx).unwrap_or(""),
                after_slash.get(idx..).map(str::trim),
            ),
            None => (after_slash, None),
        };
        let cmd = cmd_raw.to_ascii_lowercase();
        let arg = arg_opt.unwrap_or("");

        match cmd.as_str() {
            "" | "help" | "?" | "keys" | "commands" => {
                lines.extend(format_help());
                return (effects, lines);
            }
            "chats" | "list" | "ls" => {
                lines.extend(format_conversations(state, now));
                return (effects, lines);
            }
            "status" | "info" | "who" => {
                lines.extend(format_status(state, now));
                return (effects, lines);
            }
            "history" | "h" => {
                let count = if arg.is_empty() {
                    INITIAL_HISTORY
                } else {
                    match arg.parse::<usize>() {
                        Ok(n) if n > 0 => n.min(MAX_HISTORY),
                        _ => {
                            lines.push("* Usage: /history [count]".to_owned());
                            return (effects, lines);
                        }
                    }
                };
                if let Some(open) = &state.open {
                    let history = format_history(state, &open.messages, count, tz, now);
                    if history.is_empty() {
                        lines.push("* (no messages in this conversation)".to_owned());
                    } else {
                        lines.extend(history);
                    }
                } else {
                    lines.push("* No conversation open (use /chats and /chat <name>)".to_owned());
                }
                return (effects, lines);
            }
            "chat" | "open" | "goto" | "c" => {
                if arg.is_empty() {
                    lines.extend(format_conversations(state, now));
                    return (effects, lines);
                }
                if let Some(target_id) = resolve_chat_target(state, arg) {
                    if state.open.as_ref().is_some_and(|o| o.id == target_id && !o.loading) {
                        let title = clean_one_line(state, &state.conversation_title(target_id));
                        lines.push(format!("# Chat: {title}"));
                        if let Some(open) = &state.open {
                            lines.extend(format_history(
                                state,
                                &open.messages,
                                INITIAL_HISTORY,
                                tz,
                                now,
                            ));
                        }
                    } else {
                        effects.extend(update::open_conversation(state, target_id));
                    }
                } else {
                    lines.push(format!(
                        "* No conversation matching \"{}\" (use /chats to list)",
                        clean_one_line(state, arg)
                    ));
                }
                return (effects, lines);
            }
            "reply" | "r" => {
                if arg.is_empty() {
                    lines.push("* Usage: /reply <message text>".to_owned());
                    return (effects, lines);
                }
                let Some(target) = latest_message_ref(state) else {
                    lines.push("* No message to reply to in the active chat".to_owned());
                    return (effects, lines);
                };
                selection::reply(state, target);
                if let Some(open) = state.open.as_mut() {
                    open.input = Input::new(arg);
                }
                effects.extend(sending::send_message(state));
            }
            "react" | "+" => {
                if arg.is_empty() {
                    lines.push("* Usage: /react <emoji>".to_owned());
                    return (effects, lines);
                }
                let Some(target) = latest_message_ref(state) else {
                    lines.push("* No message to react to in the active chat".to_owned());
                    return (effects, lines);
                };
                let emoji = resolve_emoji(arg);
                let author = clean_one_line(state, &state.person_name(target.sender));
                let target_time = format_hm(target.sent_at, tz);
                effects.extend(selection::react(state, target, &emoji));
                let still_set = state
                    .open
                    .as_ref()
                    .and_then(|o| o.messages.iter().rfind(|m| m.reference() == target))
                    .is_some_and(|m| m.reactions.iter().any(|r| r.sender == state.me));
                if still_set {
                    lines.push(format!(
                        "* You reacted {} to {author} ({target_time})",
                        clean_one_line(state, &emoji)
                    ));
                } else {
                    lines.push(format!(
                        "* You removed reaction from {author} ({target_time})"
                    ));
                }
            }
            "quit" | "q" | "exit" => {
                effects.extend(navigation::on_action(state, Action::Quit));
                return (effects, lines);
            }
            _ => {
                if state.open.is_none() {
                    lines.push(
                        "* No conversation open (use /chats and /chat <name> to open one)"
                            .to_owned(),
                    );
                    return (effects, lines);
                }
                effects.extend(slash::execute(state, trimmed));
                if matches!(state.popup, Some(Popup::About)) {
                    lines.push(format!(
                        "* faro {} — linear mode",
                        crate::version::shown()
                    ));
                }
            }
        }
    } else {
        let Some(open) = state.open.as_mut() else {
            lines.push(
                "* No conversation open (use /chats and /chat <name> to open one)".to_owned(),
            );
            return (effects, lines);
        };
        open.input = Input::new(trimmed);
        effects.extend(sending::send_message(state));
    }

    if state.last_sent_at > prev_sent_at {
        let sent_at = state.last_sent_at;
        let me = state.me;
        if let Some(sent_msg) = state
            .open
            .as_ref()
            .and_then(|o| {
                o.messages
                    .iter()
                    .rfind(|m| m.sent_at == sent_at && m.sender == me)
            })
            .cloned()
        {
            lines.extend(format_message(state, &sent_msg, tz, now, false));
        }
    }

    if let Some(notice_line) = drain_notice(state) {
        lines.push(notice_line);
    }

    state.palette = None;
    state.popup = None;
    state.focus = Focus::Input;

    (effects, lines)
}

fn latest_message_ref(state: &State) -> Option<MessageRef> {
    state
        .open
        .as_ref()?
        .messages
        .iter()
        .rev()
        .find(|m| !m.deleted && !m.is_system())
        .map(Message::reference)
}

fn resolve_emoji(arg: &str) -> String {
    let trimmed = arg.trim();
    let shortcode = trimmed
        .strip_prefix(':')
        .and_then(|s| s.strip_suffix(':'))
        .unwrap_or(trimmed);
    if shortcode
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '+' | '-'))
        && let Some(&(_, emoji)) = crate::ui::shortcodes::search_shortcodes(shortcode, 1).first()
    {
        return emoji.to_owned();
    }
    trimmed.to_owned()
}

fn resolve_chat_target(state: &State, query: &str) -> Option<ConversationId> {
    let q = query.trim();
    if let Ok(index) = q.parse::<usize>()
        && index >= 1
    {
        let visible: Vec<_> = state.listed_in(SidebarView::Chats).collect();
        if let Some(summary) = visible.get(index - 1) {
            return Some(summary.id);
        }
    }
    let lower = q.to_lowercase();
    let mut exact = None;
    let mut prefix = None;
    let mut contains = None;
    for summary in &state.conversations {
        let title = state.conversation_title(summary.id).to_lowercase();
        if title == lower {
            exact = Some(summary.id);
            break;
        } else if prefix.is_none() && title.starts_with(&lower) {
            prefix = Some(summary.id);
        } else if contains.is_none() && title.contains(&lower) {
            contains = Some(summary.id);
        }
    }
    exact.or(prefix).or(contains)
}

fn format_reactions(state: &State, message: &Message) -> String {
    let mut order: Vec<&str> = Vec::new();
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for reaction in &message.reactions {
        let emoji = reaction.emoji.as_str();
        let count = counts.entry(emoji).or_insert(0);
        if *count == 0 {
            order.push(emoji);
        }
        *count += 1;
    }
    order
        .iter()
        .map(|emoji| {
            let n = counts.get(emoji).copied().unwrap_or(0);
            let emoji = clean_one_line(state, emoji);
            if n > 1 {
                format!("{emoji} {n}")
            } else {
                emoji
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

#[expect(
    clippy::cast_precision_loss,
    reason = "sizes are shown to one decimal; precision beyond 2^52 bytes doesn't matter"
)]
fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["KB", "MB", "GB", "TB"];
    if bytes < 1024 {
        return format!(" · {bytes} B");
    }
    let mut size = bytes as f64 / 1024.0;
    let mut unit = 0;
    while size >= 1024.0 && unit + 1 < UNITS.len() {
        size /= 1024.0;
        unit += 1;
    }
    format!(" · {size:.1} {}", UNITS.get(unit).copied().unwrap_or("TB"))
}

#[cfg(test)]
mod tests {
    use faro_core::domain::{
        AttachmentRef, CallKind, CallState, Mention, MessageBody, Quote, Reaction, SystemEvent,
        TranscriptSource,
    };

    use super::*;
    use crate::fixtures::{ALICE, BOB, ME, alice, bob, climbing, loaded_state, text};

    fn utc() -> TimeZone {
        TimeZone::UTC
    }

    #[test]
    fn formats_time_sender_and_text_with_no_ansi_or_boxes() {
        let state = loaded_state();
        let tz = utc();
        let now = Timestamp::from_millis(1_700_000_060_000);
        // 2023-11-14T22:13:20Z -> 22:13 in UTC
        let message = text(bob(), BOB, 1_700_000_000_000, "on my way");
        let lines = format_message(&state, &message, &tz, now, false);
        assert_eq!(lines, vec!["22:13 Bob: on my way"]);
        assert!(!lines[0].contains('\x1b'), "no ANSI escapes");
    }

    #[test]
    fn preserves_rtl_and_cjk_codepoints_in_logical_order() {
        let state = loaded_state();
        let tz = utc();
        let now = Timestamp::from_millis(1_700_000_060_000);

        let arabic_hebrew_cjk = "مرحبا بالعالم — שלום עולם — 日本語のテスト — 한국어 — 🚀";
        let message = text(bob(), BOB, 1_700_000_000_000, arabic_hebrew_cjk);
        let lines = format_message(&state, &message, &tz, now, false);
        assert_eq!(lines, vec![format!("22:13 Bob: {arabic_hebrew_cjk}")]);
        // Every codepoint in the body appears in the exact same order.
        let (_, body_part) = lines[0].split_once("Bob: ").unwrap();
        assert_eq!(body_part, arabic_hebrew_cjk);
    }

    #[test]
    fn sanitizes_control_characters_against_terminal_injection() {
        let state = loaded_state();
        let tz = utc();
        let now = Timestamp::from_millis(1_700_000_060_000);

        let malicious = "hello\x1b[31mred\x07\tworld";
        let message = text(bob(), BOB, 1_700_000_000_000, malicious);
        let lines = format_message(&state, &message, &tz, now, false);
        assert_eq!(lines, vec!["22:13 Bob: hello\u{FFFD}[31mred\u{FFFD} world"]);
        assert!(!lines[0].contains('\x1b'));
        assert!(!lines[0].contains('\x07'));
    }

    #[test]
    fn formats_replies_mentions_attachments_reactions_and_system_events() {
        let state = loaded_state();
        let tz = utc();
        let now = Timestamp::from_millis(1_700_000_060_000);

        // Message with quote, mention, attachment, reaction, and edited badge.
        let mut msg = text(bob(), BOB, 1_700_000_000_000, "hi \u{FFFC}!");
        msg.mentions = vec![Mention {
            who: ALICE,
            start: 3,
            len: 3,
        }];
        msg.quote = Some(Quote {
            target: MessageRef {
                sender: state.me,
                sent_at: Timestamp::from_millis(1_699_999_900_000),
            },
            preview: Some(MessageBody::new("are you there?")),
        });
        let mut att = AttachmentRef::remote("a1".to_owned(), "application/pdf".to_owned());
        att.file_name = Some("notes.pdf".to_owned());
        att.size = Some(2048);
        msg.attachments = vec![att];
        msg.reactions = vec![
            Reaction {
                sender: state.me,
                emoji: "👍".to_owned(),
            },
            Reaction {
                sender: ALICE,
                emoji: "👍".to_owned(),
            },
        ];
        msg.edited_at = Some(Timestamp::from_millis(1_700_000_010_000));

        let lines = format_message(&state, &msg, &tz, now, true);
        assert_eq!(
            lines,
            vec![
                "22:13 [Bob] Bob (reply to You: \"are you there?\"): hi @Alice 🌸! [attachment: notes.pdf · 2.0 KB] (edited) [reactions: 👍 2]"
            ]
        );

        // Voice note with transcript.
        let mut vn_msg = text(bob(), BOB, 1_700_000_000_000, "");
        vn_msg.body = None;
        let mut vn = AttachmentRef::remote("v1".to_owned(), "audio/aac".to_owned());
        vn.voice_note = true;
        vn.duration_secs = Some(14);
        vn.transcript = Some("let us meet at noon".to_owned());
        vn.transcript_source = Some(TranscriptSource::Local);
        vn_msg.attachments = vec![vn];
        let vn_lines = format_message(&state, &vn_msg, &tz, now, false);
        assert_eq!(
            vn_lines,
            vec!["22:13 Bob: [voice note 0:14: \"let us meet at noon\"]"]
        );

        // System event.
        let sys_msg = Message::system(
            bob(),
            BOB,
            Timestamp::from_millis(1_700_000_000_000),
            Direction::Incoming,
            SystemEvent::Call {
                kind: CallKind::Voice,
                state: CallState::Missed,
            },
        );
        let sys_lines = format_message(&state, &sys_msg, &tz, now, false);
        assert_eq!(sys_lines, vec!["22:13 * ☎ Missed voice call"]);
    }

    #[test]
    fn format_history_orders_messages_deterministically_by_timestamp_and_sender() {
        let state = loaded_state();
        let tz = utc();
        let now = Timestamp::from_millis(1_700_000_100_000);
        let m1 = text(bob(), BOB, 1_700_000_000_000, "first");
        let m2 = text(bob(), ME, 1_700_000_060_000, "second");
        let m3 = text(bob(), BOB, 1_700_000_120_000, "third");

        // Even if passed out of order, format_history emits oldest first and respects limit.
        let lines = format_history(
            &state,
            &[m3.clone(), m1.clone(), m2.clone()],
            2,
            &tz,
            now,
        );
        assert_eq!(
            lines,
            vec!["22:14 You: second", "22:15 Bob: third"]
        );
    }

    #[test]
    fn live_backend_events_stream_messages_reactions_edits_and_deletions() {
        let mut state = loaded_state();
        let tz = utc();
        let now = Timestamp::from_millis(1_700_000_100_000);
        let mut printed_banner = true;

        // `loaded_state()` has `climbing()` open; an incoming message in `bob()` includes `[Bob]`.
        let bob_msg = text(bob(), BOB, 1_700_000_000_000, "deploying now");
        let (_, lines) = on_app_event(
            &mut state,
            AppEvent::Backend(BackendEvent::Message(bob_msg)),
            &mut printed_banner,
            &tz,
            now,
        );
        assert_eq!(lines, vec!["22:13 [Bob] Bob: deploying now"]);

        // Reaction in active chat (`climbing()`).
        let target = state.open.as_ref().unwrap().messages.last().unwrap().reference();
        let target_hm = format_hm(target.sent_at, &tz);
        let target_author = state.person_name(target.sender);
        let (_, lines) = on_app_event(
            &mut state,
            AppEvent::Backend(BackendEvent::Reaction {
                conversation: climbing(),
                target,
                sender: ALICE,
                emoji: Some("❤️".to_owned()),
            }),
            &mut printed_banner,
            &tz,
            now,
        );
        assert_eq!(
            lines,
            vec![format!(
                "* Alice 🌸 reacted ❤️ to {target_author} ({target_hm})"
            )]
        );

        // Edit in active chat.
        let (_, lines) = on_app_event(
            &mut state,
            AppEvent::Backend(BackendEvent::Edited {
                conversation: climbing(),
                target,
                body: MessageBody::new("updated text"),
                mentions: Vec::new(),
                edited_at: Timestamp::from_millis(1_700_000_060_000),
            }),
            &mut printed_banner,
            &tz,
            now,
        );
        assert_eq!(lines, vec![format!("22:14 {target_author} (edited): updated text")]);

        // Deletion in active chat.
        let (_, lines) = on_app_event(
            &mut state,
            AppEvent::Backend(BackendEvent::Deleted {
                conversation: climbing(),
                target,
            }),
            &mut printed_banner,
            &tz,
            now,
        );
        assert_eq!(
            lines,
            vec![format!("* {target_author} deleted message ({target_hm})")]
        );
    }

    #[test]
    fn handle_line_supports_sending_and_linear_commands() {
        let mut state = loaded_state();
        let tz = utc();
        let now = Timestamp::from_millis(1_700_000_000_000);

        // `/chats` lists visible conversations.
        let (_, lines) = handle_line(&mut state, "/chats", &tz, now);
        assert_eq!(lines[0], "* Conversations:");
        assert!(lines.iter().any(|l| l.contains("* Climbing 🧗")), "{lines:?}");

        // Sending plain text appends and prints the outgoing message line.
        let (effects, lines) = handle_line(&mut state, "hello screen reader", &tz, now);
        assert_ne!(effects, []);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].ends_with("You: hello screen reader"), "{lines:?}");

        // `/reply` quotes the latest message and sends.
        let (effects, lines) = handle_line(&mut state, "/reply got it", &tz, now);
        assert_ne!(effects, []);
        assert_eq!(lines.len(), 1);
        assert!(
            lines[0].contains("You (reply to You: \"hello screen reader\"): got it"),
            "{lines:?}"
        );

        // `/react` reacts to the latest message.
        let (effects, lines) = handle_line(&mut state, "/react :thumbsup:", &tz, now);
        assert_ne!(effects, []);
        assert!(lines[0].contains("* You reacted 👍 to You"), "{lines:?}");

        // `/mute 2h` mutes the active conversation and emits the notice.
        let (_, lines) = handle_line(&mut state, "/mute 2h", &tz, now);
        assert_eq!(lines, vec!["* Muted for 2 hours"]);

        // `/chat 3` switches to Alice (`alice()`).
        let (effects, _) = handle_line(&mut state, "/chat Alice", &tz, now);
        assert!(
            effects
                .iter()
                .any(|e| matches!(e, Effect::LoadMessages(id) if *id == alice())),
            "switching chats loads messages"
        );

        // `/quit` sets `state.quit`.
        let _ = handle_line(&mut state, "/quit", &tz, now);
        assert!(state.quit);
    }
}
