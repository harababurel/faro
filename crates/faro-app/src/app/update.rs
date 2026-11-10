//! The reducer (D22): `update(state, event) -> effects`. Synchronous and free of
//! I/O, so the UI can never wait on the network or disk here. Store work is
//! returned as `Effect`s.

use crate::app::term_event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use faro_core::backend::BackendEvent;
use faro_core::config::{BubbleStyle, InlineImages};
use faro_core::domain::{
    CallState, ConnectionState, ConversationId, Direction, Message, MessageBody, MessageRef,
    ServiceId, SystemEvent, TypingState,
};
use faro_core::store::{ConversationState, ConversationSummary};
use std::time::Duration;

use faro_core::rt::Instant;

use super::event::{AppEvent, Effect};
use super::input::Input;
use super::keymap;
use super::state::Terminal;
use super::state::{
    Anchor, DRAFT_SAVE_DELAY, EmojiMode, Focus, Geometry, LockScreen, Notice, OpenConversation,
    Popup, RestoringSession, Selection, SessionRecord, SidebarScroll, SinceYouLeft, State,
    TYPING_TIMEOUT, Typist, sort_conversations,
};
use super::timeline;

pub(crate) mod curation;
pub(crate) mod navigation;
mod notify;
pub(crate) mod palette;
mod pictures;
pub(crate) mod presence;
mod resize;
pub(crate) mod selection;
pub(crate) mod sending;
pub(crate) mod settings;
mod updates;

pub use curation::mentions_toggle;
pub use navigation::mouse_affects_view;
pub use palette::open as open_palette;
pub use sending::wall_clock;
pub use settings::{choose, choose_theme, configure};

/// How many messages to load when a conversation opens.
pub const PAGE: u32 = 200;

/// Whether an event can change what's on screen; the rest don't cost a frame.
pub fn affects_view(event: &AppEvent) -> bool {
    match event {
        AppEvent::Terminal(Event::Mouse(mouse)) => mouse_affects_view(*mouse),
        AppEvent::Drawn(_) => false,
        // Focus changes can clear unread counts (D42).
        AppEvent::Terminal(
            Event::Key(_)
            | Event::Paste(_)
            | Event::Resize(..)
            | Event::FocusGained
            | Event::FocusLost,
        )
        | AppEvent::Backend(_)
        | AppEvent::Loaded { .. }
        | AppEvent::SessionLoaded(_)
        | AppEvent::MessagesLoaded { .. }
        | AppEvent::OlderLoaded { .. }
        | AppEvent::MessagesAroundLoaded { .. }
        | AppEvent::SearchResults { .. }
        | AppEvent::MessageReceiptsLoaded { .. }
        | AppEvent::QuoteThreadLoaded { .. }
        | AppEvent::SendResult { .. }
        | AppEvent::ReactResult { .. }
        | AppEvent::EditResult { .. }
        | AppEvent::DeleteResult { .. }
        | AppEvent::DevicesListed(_)
        | AppEvent::DeviceUnlinked { .. }
        | AppEvent::Fetched { .. }
        | AppEvent::LongTextFetched { .. }
        | AppEvent::SavedToDownloads(_)
        | AppEvent::FilesListed { .. }
        | AppEvent::CommandChecked { .. }
        | AppEvent::FileStaged(_)
        | AppEvent::PasteChecked { .. }
        | AppEvent::ClipboardImage { .. }
        | AppEvent::ClipboardPrompt(_)
        | AppEvent::ClipboardDrained
        | AppEvent::PlaybackFinished { .. }
        | AppEvent::Transcribed { .. }
        | AppEvent::PicturesListed(_)
        | AppEvent::PictureFetched { .. }
        | AppEvent::OpenFailed(_)
        | AppEvent::Unavailable(_)
        | AppEvent::TerminalChecked { .. }
        | AppEvent::ConfigRead(_)
        | AppEvent::ThemesRead(_)
        | AppEvent::SettingNotSaved(_)
        | AppEvent::StoreFailed(_)
        | AppEvent::UnreadCleared(_)
        | AppEvent::ExpiredSwept { .. }
        | AppEvent::Updates(_)
        | AppEvent::Tick
        | AppEvent::Shutdown => true,
    }
}

pub fn update(state: &mut State, event: AppEvent) -> Vec<Effect> {
    let mut effects = route(state, event);
    effects.extend(pictures::wanted(state));
    effects.extend(sync_session(state));
    effects
}

pub(super) fn sync_session(state: &mut State) -> Option<Effect> {
    if !state.config.conversations.restore_session || state.loading {
        return None;
    }
    if !state.quit && state.open.as_ref().is_some_and(|o| o.loading) {
        return None;
    }
    let record = state.session_record();
    if state.saved_session.as_ref() == Some(&record) {
        return None;
    }
    state.saved_session = Some(record.clone());
    Some(Effect::SaveSession(record))
}

#[expect(
    clippy::too_many_lines,
    reason = "one arm per event, each handing off to its module: reads best as one table"
)]
fn route(state: &mut State, event: AppEvent) -> Vec<Effect> {
    match event {
        AppEvent::Terminal(event) => on_terminal(state, &event),
        AppEvent::Backend(event) if state.loading => {
            state.early_events.push(event);
            vec![]
        }
        AppEvent::Backend(event) => on_backend(state, event),
        AppEvent::Loaded {
            conversations,
            last_open,
            recent_reactions,
        } => on_loaded(state, conversations, last_open, recent_reactions),
        AppEvent::SessionLoaded(record) => on_session_loaded(state, record),
        AppEvent::MessagesLoaded {
            conversation,
            messages,
        } => on_messages_loaded(state, conversation, messages),
        AppEvent::OlderLoaded {
            conversation,
            messages,
        } => on_older_loaded(state, conversation, messages),
        AppEvent::MessagesAroundLoaded {
            conversation,
            target,
            messages,
        } => on_messages_around_loaded(state, conversation, target, messages),
        AppEvent::SearchResults {
            conversation,
            query,
            hits,
        } => palette::on_search_results(state, conversation, &query, hits),
        AppEvent::MessageReceiptsLoaded {
            conversation,
            message,
            receipts,
        } => {
            selection::on_receipts_loaded(state, conversation, message, receipts);
            vec![]
        }
        AppEvent::QuoteThreadLoaded {
            conversation,
            target,
            messages,
        } => {
            selection::on_quote_thread_loaded(state, conversation, target, messages);
            vec![]
        }
        AppEvent::SendResult { sent_at, result } => sending::on_send_result(state, sent_at, result),
        AppEvent::EditResult(result) => notice_err(state, "Couldn't edit message", result),
        AppEvent::DeleteResult(result) => notice_err(state, "Couldn't delete message", result),
        AppEvent::DevicesListed(result) => palette::on_devices_listed(state, result),
        AppEvent::DeviceUnlinked { id, name, result } => {
            on_device_unlinked(state, id, &name, result)
        }
        AppEvent::ReactResult {
            conversation,
            target,
            previous,
            result,
        } => selection::on_react_result(state, conversation, target, previous, result),
        AppEvent::Fetched {
            conversation,
            message,
            index,
            result,
        } => selection::on_fetched(state, conversation, message, index, result),
        AppEvent::LongTextFetched {
            conversation,
            message,
            index,
            body,
        } => selection::on_long_text_fetched(state, conversation, message, index, body),
        AppEvent::SavedToDownloads(result) => selection::on_saved_to_downloads(state, result),
        AppEvent::CommandChecked { program, found } => {
            settings::on_command_checked(state, program, found)
        }
        AppEvent::FilesListed { query, entries } => {
            palette::on_files_listed(state, &query, &entries)
        }
        AppEvent::FileStaged(result) => sending::on_file_staged(state, result),
        AppEvent::PasteChecked { raw, staged } => sending::on_paste_checked(state, &raw, staged),
        AppEvent::ClipboardImage {
            conversation,
            result,
        } => sending::on_clipboard_image(state, conversation, result),
        AppEvent::ClipboardPrompt(terminal) => sending::on_clipboard_prompt(state, terminal),
        AppEvent::ClipboardDrained => vec![],
        AppEvent::PlaybackFinished {
            message,
            index,
            error,
        } => {
            selection::on_playback_finished(state, message, index, error);
            vec![]
        }
        AppEvent::PicturesListed(found) => pictures::on_listed(state, found),
        AppEvent::PictureFetched {
            conversation,
            result,
        } => pictures::on_fetched(state, conversation, result),
        AppEvent::Transcribed {
            conversation,
            message,
            index,
            result,
        } => selection::on_transcribed(state, conversation, message, index, result),
        AppEvent::TerminalChecked(terminal) => terminal_checked(state, terminal),
        AppEvent::OpenFailed(error) => notice_err(state, "Couldn't open", Err(error)),
        AppEvent::Unavailable(text) => unavailable(state, text),
        AppEvent::ConfigRead(read) => settings::on_config_read(state, read),
        AppEvent::ThemesRead(files) => settings::on_themes_read(state, files),
        AppEvent::SettingNotSaved(error) => {
            notice_err(state, "Couldn't save the setting", Err(error))
        }
        AppEvent::Drawn(geometry) => on_drawn(state, geometry),
        AppEvent::StoreFailed(error) => notice_err(state, "Storage problem", Err(error)),
        AppEvent::UnreadCleared(cleared) => on_unread_cleared(state, &cleared),
        AppEvent::ExpiredSwept { sweep, next_expiry } => {
            on_expired_swept(state, sweep, next_expiry)
        }
        AppEvent::Updates(event) => updates::on_event(state, event),
        AppEvent::Tick => on_tick(state),
        AppEvent::Shutdown => quit(state),
    }
}

fn notice_err<E: std::fmt::Display>(
    state: &mut State,
    prefix: &str,
    result: Result<(), E>,
) -> Vec<Effect> {
    if let Err(error) = result {
        state.notice = Some(Notice::error(format!("{prefix}: {error}")));
    }
    vec![]
}

fn on_device_unlinked(
    state: &mut State,
    id: u32,
    name: &str,
    result: Result<(), faro_core::backend::BackendError>,
) -> Vec<Effect> {
    match result {
        Ok(()) => {
            let was_current = state
                .devices
                .as_deref()
                .unwrap_or_default()
                .iter()
                .find(|d| d.id == id)
                .is_some_and(|d| d.is_current);
            if let Some(devices) = state.devices.as_mut() {
                devices.retain(|d| d.id != id);
            }
            if was_current {
                state.connection = ConnectionState::Unlinked;
                state.notice = Some(Notice::info(format!(
                    "Unlinked {name} — run `faro link` to link again"
                )));
            } else {
                state.notice = Some(Notice::info(format!("Unlinked {name}")));
            }
        }
        Err(error) => {
            state.notice = Some(Notice::error(format!("Couldn't unlink {name}: {error}")));
        }
    }
    vec![]
}

fn on_tick(state: &mut State) -> Vec<Effect> {
    let now = Instant::now();
    expire_typing(state, now);
    if let Some(playing) = state.playing.as_mut() {
        let elapsed = u32::try_from(now.saturating_duration_since(playing.started_at).as_secs())
            .unwrap_or(u32::MAX);
        playing.elapsed_secs = match playing.duration_secs {
            Some(total) if total > 0 => elapsed.min(total),
            Some(_) | None => elapsed,
        };
    }
    if state
        .notice
        .as_ref()
        .and_then(|n| n.until)
        .is_some_and(|t| now >= t)
    {
        state.notice = None;
    }
    let draft_due = state
        .open
        .as_ref()
        .and_then(|o| o.draft_edited)
        .is_some_and(|t| now >= t + DRAFT_SAVE_DELAY);
    let mut effects = presence::on_tick(state, now);
    if draft_due {
        effects.extend(save_draft(state));
    }
    if state.next_expiry.is_some_and(|(_, at)| now >= at) {
        state.next_expiry = None;
        effects.push(Effect::SweepExpired(wall_clock()));
    }
    if state.auto_lock_deadline().is_some_and(|at| now >= at) {
        effects.extend(lock_screen(state));
    }
    effects.extend(updates::on_tick(state, now));
    effects
}

pub(super) fn lock_screen(state: &mut State) -> Vec<Effect> {
    state.palette = None;
    state.popup = None;
    state.env.repaint = true;
    let mut effects = presence::stop_typing(state);
    if state.playing.take().is_some() {
        effects.push(Effect::StopAudio);
    }
    state.lock.screen = Some(if state.lock.verifier.is_some() {
        LockScreen::Locked {
            input: String::new(),
            error: None,
        }
    } else {
        LockScreen::SetPassphrase {
            input: String::new(),
            confirm: None,
            error: None,
        }
    });
    effects
}

fn on_terminal(state: &mut State, event: &Event) -> Vec<Effect> {
    // Keys, clicks and pastes only come from a focused terminal, and a
    // focus-gained report says so; a focus-lost report clears it. A resize
    // keeps whatever focus was last reported, so refitting an unfocused app
    // can't reclaim it or send read receipts (D125).
    match event {
        Event::Key(_) | Event::Mouse(_) | Event::Paste(_) => {
            state.env.focused = true;
            if !state.has_lock_overlay() {
                state.lock.last_activity = Some(Instant::now());
            }
        }
        Event::FocusGained => state.env.focused = true,
        Event::FocusLost => state.env.focused = false,
        Event::Resize(_, _) => {}
    }
    if state.has_lock_overlay() {
        return on_lock_terminal(state, event);
    }
    let mut effects = match event {
        Event::Key(key) => {
            if state.palette.is_none()
                && state.popup.is_none()
                && state.focus == Focus::Input
                && let Some(effects) = navigation::on_slash_key(state, *key)
                    .or_else(|| navigation::on_mention_key(state, *key))
                    .or_else(|| navigation::on_shortcode_key(state, *key))
            {
                effects
            } else {
                let action = if state.palette.is_some() {
                    keymap::palette_action(&state.keymap, *key)
                } else if let Some(popup) = &state.popup {
                    if matches!(popup, Popup::React { query: Some(_), .. }) {
                        keymap::popup_search_action(*key)
                    } else {
                        keymap::popup_action(*key, matches!(popup, Popup::React { .. }))
                    }
                } else {
                    state.keymap.lookup(*key, state.focus)
                };
                action.map_or_else(Vec::new, |a| navigation::on_action(state, a))
            }
        }
        Event::Mouse(mouse) if state.palette.is_some() => palette::on_mouse(state, *mouse),
        Event::Mouse(mouse) => navigation::on_mouse(state, *mouse),
        Event::Paste(text) if state.palette.is_some() => palette::paste(state, text),
        Event::Paste(text) => {
            state.popup = None;
            navigation::paste(state, text)
        }
        // A tmux client may have attached from somewhere else (D47).
        Event::FocusGained | Event::Resize(_, _) => check_terminal(state),
        Event::FocusLost => vec![],
    };
    effects.extend(presence::catch_up(state));
    effects
}

fn on_lock_terminal(state: &mut State, event: &Event) -> Vec<Effect> {
    match event {
        Event::Key(key) => on_lock_key(state, *key),
        Event::Paste(text) => {
            if let Some(
                LockScreen::Locked { input, error }
                | LockScreen::SetPassphrase { input, error, .. },
            ) = state.lock.screen.as_mut()
            {
                for ch in text.chars().filter(|c| !c.is_control()) {
                    input.push(ch);
                }
                *error = None;
            }
            vec![]
        }
        Event::FocusGained | Event::Resize(_, _) => check_terminal(state),
        Event::Mouse(_) | Event::FocusLost => vec![],
    }
}

fn on_lock_key(state: &mut State, key: KeyEvent) -> Vec<Effect> {
    if key.kind == KeyEventKind::Release {
        return vec![];
    }
    match (key.code, key.modifiers) {
        (KeyCode::Char('q' | 'Q' | 'c' | 'C'), KeyModifiers::CONTROL) => return quit(state),
        (KeyCode::Char('l' | 'L'), KeyModifiers::CONTROL) => {
            state.env.repaint = true;
            return vec![];
        }
        (KeyCode::Esc, _) => {
            if let Some(screen) = state.lock.screen.as_mut() {
                match screen {
                    LockScreen::SetPassphrase { .. } => {
                        state.lock.screen = None;
                        state.lock.last_activity = Some(Instant::now());
                        state.env.repaint = true;
                        return presence::catch_up(state);
                    }
                    LockScreen::Locked { input, error } => {
                        input.clear();
                        *error = None;
                    }
                }
            }
        }
        (KeyCode::Enter, _) => return submit_lock_screen(state),
        (KeyCode::Backspace, _) | (KeyCode::Char('h' | 'H'), KeyModifiers::CONTROL) => {
            if let Some(
                LockScreen::Locked { input, error }
                | LockScreen::SetPassphrase { input, error, .. },
            ) = state.lock.screen.as_mut()
            {
                input.pop();
                *error = None;
            }
        }
        (KeyCode::Char('u' | 'U' | 'w' | 'W'), KeyModifiers::CONTROL) => {
            if let Some(
                LockScreen::Locked { input, error }
                | LockScreen::SetPassphrase { input, error, .. },
            ) = state.lock.screen.as_mut()
            {
                input.clear();
                *error = None;
            }
        }
        (KeyCode::Char(c), m)
            if !m.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) && !c.is_control() =>
        {
            if let Some(
                LockScreen::Locked { input, error }
                | LockScreen::SetPassphrase { input, error, .. },
            ) = state.lock.screen.as_mut()
            {
                input.push(c);
                *error = None;
            }
        }
        _ => {}
    }
    vec![]
}

fn submit_lock_screen(state: &mut State) -> Vec<Effect> {
    let Some(screen) = state.lock.screen.as_mut() else {
        return vec![];
    };
    match screen {
        LockScreen::Locked { input, error } => {
            let candidate = std::mem::take(input);
            let ok = state
                .lock
                .verifier
                .as_ref()
                .is_some_and(|verify| verify(&candidate));
            if ok {
                state.lock.screen = None;
                state.lock.last_activity = Some(Instant::now());
                state.env.repaint = true;
                return presence::catch_up(state);
            }
            *error = Some("Incorrect passphrase".to_owned());
        }
        LockScreen::SetPassphrase {
            input,
            confirm,
            error,
        } => {
            let entered = std::mem::take(input);
            match confirm {
                None => {
                    if entered.is_empty() {
                        *error = Some("Passphrase cannot be empty".to_owned());
                    } else {
                        *confirm = Some(entered);
                        *error = None;
                    }
                }
                Some(first) => {
                    if &entered == first {
                        let expected = first.clone();
                        state.lock.verifier = Some(std::sync::Arc::new(move |candidate: &str| {
                            candidate == expected
                        }));
                        state.lock.screen = Some(LockScreen::Locked {
                            input: String::new(),
                            error: None,
                        });
                        state.env.repaint = true;
                    } else {
                        *confirm = None;
                        *error = Some("Passphrases did not match — try again".to_owned());
                    }
                }
            }
        }
    }
    vec![]
}

/// Take in what the last frame looked like: the normalised scroll position and
/// sidebar offset (which don't change the picture), and load older messages
/// once the top of the loaded history is on screen.
fn on_drawn(state: &mut State, geometry: Geometry) -> Vec<Effect> {
    state.sidebar = SidebarScroll {
        offset: geometry.sidebar_offset,
        reveal: false,
    };
    if geometry.sidebar.is_none() && state.focus == Focus::Conversations {
        // Drawn as input focus already (the sidebar is hidden).
        state.focus = Focus::Input;
    }
    let mut effects = Vec::new();
    if let Some(open) = state.open.as_mut() {
        if !open.loading && !open.messages.is_empty() {
            if let Some(anchor) = open.restored_anchor {
                let images_pending = !geometry.images_wanted.is_empty()
                    || !state.downloading.is_empty()
                    || !state.download_queue.is_empty();
                if images_pending {
                    open.scroll = Some(anchor);
                } else {
                    open.scroll = geometry.scroll;
                    open.restored_anchor = None;
                }
            } else {
                open.scroll = geometry.scroll;
            }
        }
        if let Some(selection) = open.selected.as_mut() {
            selection.reveal = false;
        }
        if open.scroll.is_none() {
            open.unseen = 0;
        }
        let wants_older = geometry.at_top
            && open.scroll.is_some()
            && open.has_older
            && !open.loading
            && !open.loading_older;
        if wants_older && let Some(first) = open.messages.first() {
            open.loading_older = true;
            effects.push(Effect::LoadOlder {
                conversation: open.id,
                before: first.reference(),
            });
        }
    }
    if let (Some(palette), Some(drawn)) = (state.palette.as_mut(), geometry.palette) {
        palette.offset = drawn.offset;
    }
    state.geometry = geometry;
    // Scrolled back down: what arrived meanwhile has now been seen.
    effects.extend(presence::catch_up(state));
    effects
}

/// Stop: save the draft first (effects still run before the store closes).
fn quit(state: &mut State) -> Vec<Effect> {
    state.quit = true;
    let mut effects = presence::stop_typing(state);
    if state.playing.take().is_some() {
        effects.push(Effect::StopAudio);
    }
    effects.extend(save_draft(state));
    effects
}

/// Copy the open conversation's input into its summary as the draft and store
/// it, if it changed since the last save.
pub(super) fn save_draft(state: &mut State) -> Vec<Effect> {
    let Some(open) = state.open.as_mut() else {
        return vec![];
    };
    if open.editing.is_some() || open.draft_edited.take().is_none() {
        return vec![];
    }
    let id = open.id;
    let draft = (!open.input.is_empty() && !crate::app::slash::is_slash_input(open.input.text()))
        .then(|| MessageBody::new(open.input.text()));
    match summary_entry(state, id) {
        Some(summary) => {
            summary.state.draft = draft;
            vec![Effect::SaveConversationState(
                id,
                Box::new(summary.state.clone()),
            )]
        }
        None => vec![],
    }
}

/// Open `id`: reveal it in the list, load its history, take over its draft
/// and playback, and mark it read. The web demo opens its first chat through
/// this directly (D125) instead of driving the list with synthetic keys, which
/// would claim keyboard focus it doesn't have.
pub fn open_conversation(state: &mut State, id: ConversationId) -> Vec<Effect> {
    state.sidebar.reveal = true;
    state.popup = None;
    state.since_you_left = None;
    if state.focus == Focus::Messages {
        state.focus = Focus::Input;
    }
    let mut effects = presence::stop_typing(state);
    if state.playing.take().is_some() {
        effects.push(Effect::StopAudio);
    }
    effects.extend(save_draft(state));
    effects.extend([Effect::LoadMessages(id), Effect::RememberOpen(id)]);
    let mut unread = 0;
    let mut draft = None;
    if let Some(summary) = state.conversations.iter_mut().find(|c| c.id == id) {
        unread = std::mem::take(&mut summary.unread);
        draft = summary.state.draft.as_ref().map(|d| Input::new(d.expose()));
    }
    if unread > 0 {
        effects.push(Effect::MarkRead(id));
    }
    state.open = Some(OpenConversation {
        id,
        messages: Vec::new(),
        loading: true,
        pending: Vec::new(),
        unread_on_open: unread,
        unread_marker: None,
        input: draft.unwrap_or_default(),
        draft_edited: None,
        scroll: None,
        restored_anchor: None,
        has_older: false,
        loading_older: false,
        unseen: 0,
        selected: None,
        reply_to: None,
        editing: None,
        mentions: Vec::new(),
        mention_selected: 0,
        mention_dismissed_at: None,
        shortcode_selected: 0,
        shortcode_dismissed_at: None,
        slash_selected: 0,
        slash_dismissed_for: None,
        staged_attachment: None,
        unreceipted: Vec::new(),
        typing_sent: None,
    });
    effects
}

fn on_session_loaded(state: &mut State, record: Option<SessionRecord>) -> Vec<Effect> {
    state.restoring_session = RestoringSession::Loaded(record.clone());
    state.saved_session = record;
    if !state.loading && state.config.conversations.restore_session {
        restore_from_session(state, state.list.cursor)
    } else {
        vec![]
    }
}

fn restore_from_session(state: &mut State, fallback_open: Option<ConversationId>) -> Vec<Effect> {
    let record = match &state.restoring_session {
        RestoringSession::Loaded(Some(record)) => record.clone(),
        RestoringSession::Loaded(None) | RestoringSession::NotLoaded => SessionRecord {
            conversation: fallback_open,
            anchor: None,
            selected: None,
            palette: None,
        },
    };
    let open_id = record
        .conversation
        .or(fallback_open)
        .filter(|id| state.summary(*id).is_some());
    let already_open = state.open.as_ref().is_some_and(|o| Some(o.id) == open_id);
    let here_unread = if already_open {
        state.open.as_ref().map_or(0, |o| o.unread_on_open)
    } else {
        open_id
            .and_then(|id| state.summary(id))
            .map_or(0, |s| s.unread)
    };
    let strip = SinceYouLeft::compute(state, open_id, here_unread);
    let mut effects = Vec::new();
    if let Some(id) = open_id {
        if let Some(summary) = state.summary(id) {
            state.list.view = state.section(summary).view();
        }
        if !already_open {
            effects.extend(open_conversation(state, id));
        }
        if let Some(open) = state.open.as_mut() {
            open.restored_anchor = record.anchor;
            open.scroll = record.anchor;
            open.selected = record.selected.map(|message| Selection {
                message,
                reveal: false,
            });
            if !open.loading {
                if let Some(anchor) = open.restored_anchor
                    && open.has_older
                    && open
                        .messages
                        .first()
                        .is_some_and(|first| anchor.message.sent_at < first.sent_at)
                {
                    open.loading = true;
                    effects.push(Effect::LoadMessagesAround {
                        conversation: id,
                        target: anchor.message,
                    });
                } else {
                    apply_restored_anchor(open);
                }
            }
        }
    }
    state.since_you_left = Some(strip);
    if let Some(level) = record.palette {
        effects.extend(palette::open_at_level(state, level));
    }
    state.saved_session = Some(state.session_record());
    effects
}

fn apply_restored_anchor(open: &mut OpenConversation) {
    if let Some(sel) = open.selected
        && !open.messages.iter().any(|m| m.reference() == sel.message)
    {
        open.selected = None;
    }
    let Some(anchor) = open.restored_anchor else {
        return;
    };
    let resolved = if open
        .messages
        .iter()
        .any(|m| m.reference() == anchor.message)
    {
        Some(anchor)
    } else {
        open.messages
            .iter()
            .rfind(|m| m.sent_at <= anchor.message.sent_at)
            .or_else(|| open.messages.first())
            .map(|m| Anchor {
                message: m.reference(),
                offset: 0,
            })
    };
    let Some(resolved) = resolved else {
        open.restored_anchor = None;
        open.scroll = None;
        return;
    };
    let at_bottom = open
        .messages
        .last()
        .is_some_and(|last| last.reference() == resolved.message && resolved.offset <= 0);
    if at_bottom {
        open.restored_anchor = None;
        open.scroll = None;
    } else {
        open.restored_anchor = Some(resolved);
        open.scroll = Some(resolved);
        let after_anchor = u32::try_from(
            open.messages
                .iter()
                .filter(|m| {
                    m.sent_at > resolved.message.sent_at
                        && m.direction == Direction::Incoming
                        && !m.deleted
                        && !m.is_system()
                })
                .count(),
        )
        .unwrap_or(u32::MAX);
        open.unseen = after_anchor.min(open.unread_on_open);
    }
}

fn on_loaded(
    state: &mut State,
    mut conversations: Vec<ConversationSummary>,
    last_open: Option<ConversationId>,
    recent_reactions: Vec<String>,
) -> Vec<Effect> {
    selection::on_recent_reactions_loaded(state, recent_reactions);
    sort_conversations(&mut conversations);
    state.conversations = conversations;
    state.recount_names();
    // Still loading while they're applied: the backlog doesn't ring (D49).
    let mut effects = Vec::new();
    for event in std::mem::take(&mut state.early_events) {
        effects.extend(on_backend(state, event));
    }
    state.loading = false;
    note_summary_expiries(state);
    // Nothing opens by itself (D60) unless `restore_session` is on: the list
    // highlights where the user was, in the list it's in (it may be archived),
    // and has the focus.
    let remembered = last_open.filter(|id| state.summary(*id).is_some());
    if let Some(summary) = remembered.and_then(|id| state.summary(id)) {
        state.list.view = state.section(summary).view();
    }
    state.list.cursor = remembered;
    state.sidebar.reveal = true;
    if state.config.conversations.restore_session {
        effects.extend(restore_from_session(state, remembered));
    }
    if state.open.is_none() && state.panels.left {
        state.focus = Focus::Conversations;
    }
    if !state.pictures.listed {
        effects.push(Effect::ListPictures);
    }
    effects
}

fn on_messages_loaded(
    state: &mut State,
    conversation: ConversationId,
    messages: Vec<Message>,
) -> Vec<Effect> {
    let Some(open) = state
        .open
        .as_mut()
        .filter(|o| o.id == conversation && o.loading)
    else {
        // The user moved on before the page arrived.
        return vec![];
    };
    open.has_older = full_page(&messages);
    open.messages = messages;
    for event in std::mem::take(&mut open.pending) {
        timeline::apply(&mut open.messages, &event);
    }
    if let Some(anchor) = open.restored_anchor
        && open.has_older
        && open
            .messages
            .first()
            .is_some_and(|first| anchor.message.sent_at < first.sent_at)
    {
        return vec![Effect::LoadMessagesAround {
            conversation,
            target: anchor.message,
        }];
    }
    open.loading = false;
    apply_restored_anchor(open);
    let unread = last_unread(&open.messages, open.unread_on_open);
    open.unread_marker = unread.first().copied().or(open.unread_marker);
    presence::on_history(state, &unread);
    let mut effects = presence::catch_up(state);
    note_loaded_expiries(state);
    effects.extend(selection::queue_auto_downloads(state));
    effects.extend(selection::queue_auto_transcriptions(state));
    effects
}

fn on_older_loaded(
    state: &mut State,
    conversation: ConversationId,
    messages: Vec<Message>,
) -> Vec<Effect> {
    let Some(open) = state
        .open
        .as_mut()
        .filter(|o| o.id == conversation && o.loading_older)
    else {
        return vec![];
    };
    open.loading_older = false;
    open.has_older = full_page(&messages);
    timeline::prepend(&mut open.messages, messages);
    note_loaded_expiries(state);
    let mut effects = selection::queue_auto_downloads(state);
    effects.extend(selection::queue_auto_transcriptions(state));
    effects
}

fn on_messages_around_loaded(
    state: &mut State,
    conversation: ConversationId,
    target: MessageRef,
    messages: Vec<Message>,
) -> Vec<Effect> {
    // Choosing the result opened its chat already (`palette::jump_to_message`,
    // with everything leaving the previous one does). If another chat is open
    // now, or none, the user moved on while this loaded: it's stale.
    let Some(open) = state.open.as_mut().filter(|o| o.id == conversation) else {
        return vec![];
    };
    let restoring_anchor = open.restored_anchor.is_some_and(|a| a.message == target);
    open.has_older = true;
    open.messages = messages;
    for event in std::mem::take(&mut open.pending) {
        timeline::apply(&mut open.messages, &event);
    }
    open.loading = false;
    open.loading_older = false;
    if restoring_anchor {
        apply_restored_anchor(open);
    } else {
        open.selected = Some(Selection {
            message: target,
            reveal: true,
        });
        state.focus = Focus::Messages;
        state.palette = None;
        state.popup = None;
    }
    let unread = last_unread(&open.messages, open.unread_on_open);
    open.unread_marker = unread.first().copied().or(open.unread_marker);
    presence::on_history(state, &unread);
    let mut effects = presence::catch_up(state);
    note_loaded_expiries(state);
    effects.extend(selection::queue_auto_downloads(state));
    effects.extend(selection::queue_auto_transcriptions(state));
    effects
}

/// A full page means there may be more before it.
fn full_page(messages: &[Message]) -> bool {
    u32::try_from(messages.len()).is_ok_and(|n| n >= PAGE)
}

/// The last `unread` incoming messages, oldest first.
fn last_unread(messages: &[Message], unread: u32) -> Vec<MessageRef> {
    let n = usize::try_from(unread).unwrap_or(usize::MAX);
    let mut refs: Vec<MessageRef> = messages
        .iter()
        .rev()
        .filter(|m| m.direction == Direction::Incoming && !m.deleted && !m.is_system())
        .take(n)
        .map(Message::reference)
        .collect();
    refs.reverse();
    refs
}

fn on_backend(state: &mut State, event: BackendEvent) -> Vec<Effect> {
    let mut effects = Vec::new();
    match &event {
        BackendEvent::Connection(connection) => {
            state.connection = *connection;
            // Back online: pictures that failed get another try (D160).
            if *connection == ConnectionState::Connected {
                state.pictures.failed.clear();
            }
        }
        BackendEvent::Message(message) => on_message(state, message, &mut effects),
        BackendEvent::DeliveryUpdate {
            message,
            status,
            by,
            updated_at,
        } => {
            apply_to_open(state, None, &event);
            selection::on_delivery_update(state, *message, *status, *by, *updated_at);
        }
        BackendEvent::Reaction { conversation, .. } => {
            apply_to_open(state, Some(*conversation), &event);
        }
        BackendEvent::Edited { conversation, .. } | BackendEvent::Deleted { conversation, .. } => {
            apply_to_open(state, Some(*conversation), &event);
            track_last_message(state, *conversation, &event);
        }
        BackendEvent::Typing {
            conversation,
            sender,
            state: typing,
        } => on_typing(state, *conversation, *sender, *typing),
        BackendEvent::ReadElsewhere(messages) => {
            presence::on_read_elsewhere(state, messages);
            effects.push(Effect::ReadElsewhere(messages.clone()));
        }
        BackendEvent::DisappearingTimer {
            conversation,
            seconds,
        } => {
            if let Some(summary) = summary_entry(state, *conversation) {
                summary.disappearing_timer = *seconds;
            }
        }
        BackendEvent::ContactUpdated(contact) => {
            let id = ConversationId::Direct(contact.service_id);
            let title = contact.display_name().map(str::to_owned);
            let phone = contact.phone.clone();
            state.contacts.insert(contact.service_id, contact.clone());
            // Someone new (a contact sync, a first message) is listed at
            // standalone, as the store lists them after a restart.
            let new = !state.conversations.iter().any(|c| c.id == id);
            if let Some(summary) = summary_entry(state, id) {
                summary.title = title;
                summary.phone = phone;
            }
            if new {
                sort_conversations(&mut state.conversations);
            }
            state.recount_names();
        }
        BackendEvent::GroupUpdated(group) => {
            let id = ConversationId::Group(group.id);
            state.groups.insert(group.id, group.clone());
            let new = !state.conversations.iter().any(|c| c.id == id);
            if let Some(summary) = summary_entry(state, id) {
                summary.title = Some(group.title.clone());
            }
            if new {
                sort_conversations(&mut state.conversations);
            }
        }
        BackendEvent::IdentityUpdated(update) => {
            let record = state.identities.entry(update.service_id).or_default();
            let changed = record.update(update.safety_number.clone(), update.verified);
            if changed {
                tracing::warn!(peer = %update.service_id, "safety number changed");
                state.notice = Some(Notice::warning(format!(
                    "Safety number with {} changed — verify before sending",
                    state.person_name(update.service_id)
                )));
            }
        }
        BackendEvent::BlockedSynced { conversations } => {
            curation::on_blocked_synced(state, conversations);
        }
        BackendEvent::MessageRequestResponded {
            conversation,
            action,
        } => {
            effects.extend(curation::apply_request_response(
                state,
                *conversation,
                *action,
            ));
        }
        BackendEvent::Error(error) => {
            state.notice = Some(Notice::warning(format!("Signal: {error}")));
        }
    }
    if persistent(&event) {
        effects.insert(0, Effect::Persist(Box::new(event)));
    }
    effects
}

/// Whether the store keeps anything from this event.
fn persistent(event: &BackendEvent) -> bool {
    match event {
        BackendEvent::Message(_)
        | BackendEvent::DeliveryUpdate { .. }
        | BackendEvent::Reaction { .. }
        | BackendEvent::Edited { .. }
        | BackendEvent::Deleted { .. }
        | BackendEvent::DisappearingTimer { .. }
        | BackendEvent::ContactUpdated(_)
        | BackendEvent::GroupUpdated(_)
        | BackendEvent::IdentityUpdated(_)
        | BackendEvent::BlockedSynced { .. }
        | BackendEvent::MessageRequestResponded { .. } => true,
        // Read syncs are applied by their own effect, which reports what it
        // cleared (D70).
        BackendEvent::ReadElsewhere(_)
        | BackendEvent::Connection(_)
        | BackendEvent::Typing { .. }
        | BackendEvent::Error(_) => false,
    }
}

/// The store lowered these unread counts (messages read on the phone,
/// D70): lower ours by as much. By how much, not to what, so messages that
/// arrived meanwhile still count, and a conversation opened meanwhile
/// stays at zero.
fn on_unread_cleared(state: &mut State, cleared: &[(ConversationId, u32)]) -> Vec<Effect> {
    for (id, dropped) in cleared {
        if let Some(summary) = state.conversations.iter_mut().find(|c| c.id == *id) {
            summary.unread = summary.unread.saturating_sub(*dropped);
        }
    }
    vec![]
}

fn on_message(state: &mut State, message: &Message, effects: &mut Vec<Effect>) {
    let id = message.conversation;
    if let Some(typists) = state.typing.get_mut(&id) {
        typists.retain(|t| t.who != message.sender);
    }
    let is_open = state.open.as_ref().is_some_and(|o| o.id == id);
    // Unread unless it's in front of the user (D42). System lines never count
    // as unread or ring notifications (D167).
    let seen = is_open && state.seeing_open();
    let incoming =
        message.direction == Direction::Incoming && !message.deleted && !message.is_system();
    let is_new_conversation = !state.conversations.iter().any(|c| c.id == id);
    let new_request = is_new_conversation
        && message.direction == Direction::Incoming
        && !message.deleted
        && matches!(id, ConversationId::Direct(peer) if state.is_unknown_peer(peer));
    let is_blocked = state.summary(id).is_some_and(|s| s.state.blocked);
    if !state.loading
        && !is_blocked
        && !new_request
        && !state.summary(id).is_some_and(|s| s.state.request)
        && let Some(
            event @ SystemEvent::Call {
                state: CallState::Incoming,
                ..
            },
        ) = &message.system_event
    {
        state.notice = Some(Notice::info(state.system_event_text(message, event)));
    }
    if let Some(summary) = summary_entry(state, id) {
        update_summary_on_message(summary, message, new_request, seen, incoming, effects);
    }
    if let Some(at) = message.expires_at() {
        state.note_expiry(at, wall_clock(), Instant::now());
    }
    if incoming && !seen && !is_blocked {
        effects.extend(notify::on_unseen_message(state, message));
    }
    sort_conversations(&mut state.conversations);
    if is_open {
        apply_to_open(state, Some(id), &BackendEvent::Message(message.clone()));
        if let Some(open) = state.open.as_mut()
            && open.scroll.is_some()
            && !message.is_system()
        {
            open.unseen += 1;
        }
        presence::on_new_message(state, message);
        // Persisted first (effects run in order), so a `MarkRead` here clears
        // the count the store just raised.
        effects.extend(presence::catch_up(state));
        effects.extend(selection::queue_auto_downloads(state));
        effects.extend(selection::queue_auto_transcriptions(state));
    }
}

fn update_summary_on_message(
    summary: &mut ConversationSummary,
    message: &Message,
    new_request: bool,
    seen: bool,
    incoming: bool,
    effects: &mut Vec<Effect>,
) {
    let id = summary.id;
    let blocked = summary.state.blocked;
    if new_request && !blocked {
        summary.state.request = true;
        effects.push(Effect::SaveConversationState(
            id,
            Box::new(summary.state.clone()),
        ));
    } else if matches!(message.direction, Direction::Outgoing(_))
        && !message.is_system()
        && summary.state.request
    {
        summary.state.request = false;
        effects.push(Effect::SaveConversationState(
            id,
            Box::new(summary.state.clone()),
        ));
    }
    summary.last_activity = summary.last_activity.max(Some(message.sent_at));
    if message.expires_in_secs.is_some() {
        summary.disappearing_timer = message.expires_in_secs;
    }
    // The list's preview (D154): only drawing, never a read (D42).
    if let (
        Some(last),
        Some(SystemEvent::Call {
            state: new_state @ (CallState::Missed | CallState::Accepted),
            ..
        }),
    ) = (summary.last_message.as_mut(), &message.system_event)
        && matches!(
            last.system_event,
            Some(SystemEvent::Call {
                state: CallState::Incoming,
                ..
            })
        )
        && last
            .sent_at
            .as_millis()
            .abs_diff(message.sent_at.as_millis())
            <= 300_000
    {
        if let Some(SystemEvent::Call { state, .. }) = &mut last.system_event {
            *state = *new_state;
        }
    } else if summary
        .last_message
        .as_ref()
        .is_none_or(|last| (last.sent_at, last.sender) < (message.sent_at, message.sender))
    {
        summary.last_message = Some(message.clone());
    }
    if incoming && !seen && !blocked {
        summary.unread += 1;
    }
    // Hidden until a new message arrives (DESIGN.md §7.7), unless blocked.
    if summary.state.hidden && !blocked && !message.deleted {
        summary.state.hidden = false;
        effects.push(Effect::SaveConversationState(
            id,
            Box::new(summary.state.clone()),
        ));
    }
}

/// Apply an edit or deletion to the list's newest message (D154) if it's
/// the target, by the rules the open timeline follows.
fn track_last_message(state: &mut State, conversation: ConversationId, event: &BackendEvent) {
    let Some(summary) = state
        .conversations
        .iter_mut()
        .find(|c| c.id == conversation)
    else {
        return;
    };
    let mut newest: Vec<Message> = summary.last_message.take().into_iter().collect();
    timeline::apply(&mut newest, event);
    summary.last_message = newest.pop();
}

/// The summary for `id`, created if this is a new conversation. (Always `Some`;
/// an `Option` only because getting it back out of the `Vec` can't be proven.)
fn summary_entry(state: &mut State, id: ConversationId) -> Option<&mut ConversationSummary> {
    if let Some(at) = state.conversations.iter().position(|c| c.id == id) {
        return state.conversations.get_mut(at);
    }
    let (title, phone) = match id {
        ConversationId::Direct(peer) => state.contacts.get(&peer).map_or((None, None), |c| {
            (c.display_name().map(str::to_owned), c.phone.clone())
        }),
        ConversationId::Group(group) => (state.groups.get(&group).map(|g| g.title.clone()), None),
    };
    state.conversations.push(ConversationSummary {
        id,
        title,
        phone,
        last_activity: None,
        last_message: None,
        unread: 0,
        state: ConversationState::default(),
        disappearing_timer: None,
    });
    state.conversations.last_mut()
}

fn note_loaded_expiries(state: &mut State) {
    let Some(expiries): Option<Vec<_>> = state
        .open
        .as_ref()
        .map(|o| o.messages.iter().filter_map(Message::expires_at).collect())
    else {
        return;
    };
    let now_wall = wall_clock();
    let now_inst = Instant::now();
    for at in expiries {
        state.note_expiry(at, now_wall, now_inst);
    }
}

fn note_summary_expiries(state: &mut State) {
    let expiries: Vec<_> = state
        .conversations
        .iter()
        .filter_map(|c| c.last_message.as_ref().and_then(Message::expires_at))
        .collect();
    let now_wall = wall_clock();
    let now_inst = Instant::now();
    for at in expiries {
        state.note_expiry(at, now_wall, now_inst);
    }
}

fn on_expired_swept(
    state: &mut State,
    sweep: faro_core::store::ExpiredSweep,
    next_expiry: Option<faro_core::domain::Timestamp>,
) -> Vec<Effect> {
    let now_wall = wall_clock();
    let now_inst = Instant::now();
    state.set_next_expiry(next_expiry, now_wall, now_inst);
    if sweep.messages.is_empty() {
        return vec![];
    }
    let mut effects = Vec::new();
    let was_swept = |conv: ConversationId, r: MessageRef| {
        sweep.messages.iter().any(|(c, mr)| *c == conv && *mr == r)
    };
    if state
        .playing
        .as_ref()
        .is_some_and(|p| was_swept(p.conversation, p.message))
    {
        state.playing = None;
        effects.push(Effect::StopAudio);
    }
    if let Some(open) = state.open.as_mut() {
        let conv = open.id;
        open.messages.retain(|m| !was_swept(conv, m.reference()));
        open.unreceipted.retain(|r| !was_swept(conv, *r));
        if open.unread_marker.is_some_and(|r| was_swept(conv, r)) {
            open.unread_marker = None;
        }
        if let Some(sel) = open.selected
            && !open.messages.iter().any(|m| m.reference() == sel.message)
        {
            open.selected = open.messages.last().map(|m| Selection {
                message: m.reference(),
                reveal: false,
            });
        }
        let popup_target = match &state.popup {
            Some(
                Popup::Actions { target, .. }
                | Popup::React { target, .. }
                | Popup::MessageInfo { target, .. }
                | Popup::ImageViewer { target, .. }
                | Popup::ConfirmDelete { target }
                | Popup::ConfirmRemoteTranscription { target, .. },
            ) => Some(*target),
            Some(
                Popup::StagedImage
                | Popup::About
                | Popup::Conversation { .. }
                | Popup::Mute { .. }
                | Popup::DisappearingTimer { .. }
                | Popup::SafetyNumber(_)
                | Popup::ConfirmUnlinkDevice { .. }
                | Popup::Thread { .. },
            )
            | None => None,
        };
        if popup_target.is_some_and(|t| was_swept(conv, t)) {
            state.popup = None;
        }
        if let Some(Popup::Thread {
            conversation: thread_conv,
            messages,
            selected,
            ..
        }) = state.popup.as_mut()
            && *thread_conv == conv
        {
            messages.retain(|m| !was_swept(conv, m.reference()));
            if messages.len() < 2 && messages.first().is_none_or(|m| m.quote.is_none()) {
                state.popup = None;
            } else {
                *selected = (*selected).min(messages.len().saturating_sub(1));
            }
        }
    }
    for (id, last_activity, last_message, unread) in sweep.conversations {
        if let Some(summary) = state.conversations.iter_mut().find(|c| c.id == id) {
            summary.last_activity = last_activity;
            summary.last_message = last_message;
            summary.unread = unread;
        }
    }
    sort_conversations(&mut state.conversations);
    effects.push(Effect::Backend(crate::app::event::Command::PurgeExpired(
        sweep.messages,
    )));
    effects
}

/// Apply to the open conversation if it's `conversation` (or, for receipts, which
/// don't say, whichever is open). While its history loads, the event waits.
fn apply_to_open(state: &mut State, conversation: Option<ConversationId>, event: &BackendEvent) {
    if let Some(Popup::Thread {
        conversation: thread_conv,
        messages,
        ..
    }) = state.popup.as_mut()
        && conversation.is_none_or(|c| c == *thread_conv)
    {
        match event {
            BackendEvent::Message(msg) => {
                let joins_thread = msg.quote.as_ref().is_some_and(|q| {
                    messages.iter().any(|m| m.reference() == q.target)
                        || messages
                            .first()
                            .and_then(|m| m.quote.as_ref())
                            .is_some_and(|first_q| first_q.target == q.target)
                });
                if joins_thread {
                    timeline::apply(messages, event);
                }
            }
            BackendEvent::DeliveryUpdate { .. }
            | BackendEvent::Reaction { .. }
            | BackendEvent::Edited { .. }
            | BackendEvent::Deleted { .. } => timeline::apply(messages, event),
            BackendEvent::Connection(_)
            | BackendEvent::Typing { .. }
            | BackendEvent::ReadElsewhere(_)
            | BackendEvent::DisappearingTimer { .. }
            | BackendEvent::ContactUpdated(_)
            | BackendEvent::GroupUpdated(_)
            | BackendEvent::IdentityUpdated(_)
            | BackendEvent::BlockedSynced { .. }
            | BackendEvent::MessageRequestResponded { .. }
            | BackendEvent::Error(_) => {}
        }
    }
    let Some(open) = state.open.as_mut() else {
        return;
    };
    if conversation.is_some_and(|c| c != open.id) {
        return;
    }
    if open.loading {
        open.pending.push(event.clone());
    } else {
        timeline::apply(&mut open.messages, event);
    }
}

fn on_typing(state: &mut State, conversation: ConversationId, who: ServiceId, typing: TypingState) {
    if state.summary(conversation).is_some_and(|s| s.state.blocked) {
        return;
    }
    let typists = state.typing.entry(conversation).or_default();
    typists.retain(|t| t.who != who);
    match typing {
        TypingState::Started => typists.push(Typist {
            who,
            since: Instant::now(),
        }),
        TypingState::Stopped => {}
    }
    if typists.is_empty() {
        state.typing.remove(&conversation);
    }
}

fn expire_typing(state: &mut State, now: Instant) {
    state.typing.retain(|_, typists| {
        typists.retain(|t| now < t.since + TYPING_TIMEOUT);
        !typists.is_empty()
    });
}

/// Say the platform can't do it, and leave what was waiting for the answer:
/// the file picker, a clipboard read.
fn unavailable(state: &mut State, text: String) -> Vec<Effect> {
    if state
        .palette
        .as_ref()
        .is_some_and(|p| p.level() == crate::app::palette::Level::AttachFile)
    {
        state.palette = None;
    }
    state.pasting = None;
    state.notice = Some(Notice::warning(text));
    vec![]
}

#[cfg(test)]
mod tests;

/// Don't look for mosh more often than this (resizes come in bursts).
const TERMINAL_RECHECK: Duration = Duration::from_secs(2);

/// Look at the terminal again, if the emoji mode follows mosh (D47), the
/// bubble style follows kitty (D58), or inline images follow mixed/mosh
/// clients (D78) or are drawn with a graphics protocol, which a newly
/// attached tmux client needs measured and sent again (D137).
fn check_terminal(state: &mut State) -> Vec<Effect> {
    let follows = state.env.emoji.auto
        || state.config.messages.bubble_style == BubbleStyle::Auto
        || !matches!(
            state.config.media.inline_images,
            InlineImages::Halfblocks | InlineImages::Off
        );
    let emoji = &mut state.env.emoji;
    let now = Instant::now();
    if !follows || emoji.checked_at.is_some_and(|t| now - t < TERMINAL_RECHECK) {
        return vec![];
    }
    emoji.checked_at = Some(now);
    vec![Effect::CheckTerminal]
}

/// Switch the emoji mode to match, redrawing everything if it changed, and
/// note who draws block glyphs and whether graphics protocols are safe.
fn terminal_checked(state: &mut State, terminal: Terminal) -> Vec<Effect> {
    let Terminal {
        mosh,
        glyphs,
        graphics,
        // Checked afresh on every paste (D91, D150).
        kitty: _,
        name: _,
        remote: _,
        graphics_blocker,
        tmux_clients,
    } = terminal;
    state.env.glyphs = glyphs;
    state.env.graphics_blocker = graphics_blocker;
    state.env.tmux_clients = tmux_clients;
    if state.env.graphics != graphics {
        state.env.graphics = graphics;
        state.env.repaint = true;
    }
    let emoji = &mut state.env.emoji;
    emoji.mosh = mosh;
    let mode = if mosh {
        EmojiMode::Compatible
    } else {
        emoji.full()
    };
    if !emoji.auto || emoji.mode == mode {
        palette::refresh_settings(state);
        return vec![];
    }
    emoji.mode = mode;
    state.env.repaint = true;
    tracing::info!(?mode, "emoji mode changed");
    state.notice = Some(Notice::info(if mosh {
        "mosh detected: drawing emoji in compatible form"
    } else {
        "No mosh: drawing emoji in full"
    }));
    palette::refresh_settings(state);
    vec![]
}
