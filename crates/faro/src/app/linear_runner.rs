//! The linear-mode event loop (`--linear`, #48): runs the same reducer, store
//! worker, and backend command task as the TUI without entering the alternate
//! screen or raw mode. Emits append-only plain-text lines to stdout and reads
//! line-oriented input from stdin.

use std::io::Write;

use anyhow::Context as _;
use faro_core::backend::{Backend, BackendEvents};
use faro_core::store::Store;
use tokio::signal::unix::{SignalKind, signal};
use tokio::sync::mpsc;
use tokio::time::{Instant, sleep_until};

use super::event::{AppEvent, Effect};
use super::state::State;
use super::{
    Directory, Router, clipboard, commands, effects, initial_state, linear, shutdown,
    unlink_expired_attachments, update,
};
use crate::config::{self, Startup};

/// Run faro in `--linear` transcript mode using process stdin and stdout.
pub async fn run<B: Backend + 'static>(
    backend: B,
    backend_events: BackendEvents,
    store: Store,
    directory: Directory,
    tz: jiff::tz::TimeZone,
    detected: super::state::Detected,
    startup: Startup,
) -> anyhow::Result<()> {
    let input_lines = spawn_stdin_lines();
    let stdout = std::io::stdout();
    run_with_io(
        backend,
        backend_events,
        store,
        directory,
        tz,
        detected,
        startup,
        input_lines,
        stdout,
    )
    .await
}

#[expect(
    clippy::disallowed_methods,
    reason = "linear mode reads line-oriented stdin on a dedicated OS thread"
)]
fn spawn_stdin_lines() -> mpsc::UnboundedReceiver<std::io::Result<String>> {
    let (tx, rx) = mpsc::unbounded_channel();
    std::thread::spawn(move || {
        use std::io::BufRead as _;
        let stdin = std::io::stdin();
        for line in stdin.lock().lines() {
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    rx
}

/// Run linear mode over a line channel `input_lines` and sync output `out`
/// (used by [`run`] with stdin/stdout and by integration tests).
#[expect(
    clippy::too_many_arguments,
    reason = "wires backend, events, store, directory, timezone, detected terminal state, startup config, input channel and output writer"
)]
pub async fn run_with_io<B, W>(
    backend: B,
    mut backend_events: BackendEvents,
    store: Store,
    directory: Directory,
    tz: jiff::tz::TimeZone,
    detected: super::state::Detected,
    mut startup: Startup,
    mut input_lines: mpsc::UnboundedReceiver<std::io::Result<String>>,
    mut out: W,
) -> anyhow::Result<()>
where
    B: Backend + 'static,
    W: Write,
{
    let mut interrupt = signal(SignalKind::interrupt()).context("listening for SIGINT")?;
    let mut terminate = signal(SignalKind::terminate()).context("listening for SIGTERM")?;
    let mut hangup = signal(SignalKind::hangup()).context("listening for SIGHUP")?;

    let (results_tx, mut results) = mpsc::unbounded_channel();
    let (effects_tx, worker) = effects::spawn(store, results_tx.clone());
    let (commands_tx, command_task) = commands::spawn(
        backend,
        directory.me,
        startup.attachments_dir.clone(),
        startup.pictures_dir.clone(),
        results_tx.clone(),
    );
    let (config_writer, config_task) =
        config::spawn_writer(startup.path.clone(), results_tx.clone());
    let watch = config::try_watch(&startup.path, &results_tx, &mut startup.problems);
    let (router, _clipboard_requests) =
        Router::new(commands_tx, effects_tx, config_writer, results_tx);

    let mut state = initial_state(directory, detected, &mut startup);
    clipboard::sweep(state.attachments_dir.clone());
    let configured = update::configure(&mut state, startup.config, startup.problems);
    dispatch_linear(&router, configured, &state);
    let mut deferred_startup_notice = linear::drain_notice(&mut state);

    dispatch_linear(&router, vec![Effect::LoadConversations], &state);

    let mut stdin_open = true;
    let mut printed_banner = false;

    loop {
        if state.quit {
            break;
        }
        let can_read_stdin = stdin_open
            && printed_banner
            && !linear::is_loading(&state)
            && !linear::has_sending(&state)
            && state.connection != faro_core::domain::ConnectionState::Connecting;
        let deadline = state.next_deadline();

        let incoming_events = tokio::select! {
            line_opt = input_lines.recv(), if can_read_stdin => {
                match line_opt {
                    Some(Ok(line)) => {
                        let now = update::wall_clock();
                        let (effects, out_lines) = linear::handle_line(&mut state, &line, &tz, now);
                        dispatch_linear(&router, effects, &state);
                        if !write_lines(&mut out, &out_lines)? {
                            break;
                        }
                        continue;
                    }
                    None => {
                        stdin_open = false;
                        vec![AppEvent::Shutdown]
                    }
                    Some(Err(error)) => {
                        tracing::error!(%error, "linear mode stdin read failed");
                        stdin_open = false;
                        vec![AppEvent::Shutdown]
                    }
                }
            }
            () = crate::crash::stopped() => break,
            Some(event) = backend_events.recv() => vec![AppEvent::Backend(event)],
            Some(event) = results.recv() => vec![event],
            () = sleep_until(deadline.unwrap_or_else(Instant::now)), if deadline.is_some() => {
                vec![AppEvent::Tick]
            }
            _ = interrupt.recv() => vec![AppEvent::Shutdown],
            _ = terminate.recv() => vec![AppEvent::Shutdown],
            _ = hangup.recv() => vec![AppEvent::Shutdown],
        };

        let mut broke_pipe = false;
        for event in incoming_events {
            if let AppEvent::ExpiredSwept { sweep, .. } = &event
                && !sweep.attachment_paths.is_empty()
            {
                unlink_expired_attachments(sweep.attachment_paths.clone());
            }
            let now = update::wall_clock();
            let (effects, mut out_lines) =
                linear::on_app_event(&mut state, event, &mut printed_banner, &tz, now);
            if printed_banner && let Some(notice) = deferred_startup_notice.take() {
                out_lines.insert(usize::from(!out_lines.is_empty()), notice);
            }
            dispatch_linear(&router, effects, &state);
            if !write_lines(&mut out, &out_lines)? {
                broke_pipe = true;
                break;
            }
        }
        if broke_pipe {
            break;
        }
    }

    drop(watch);
    let attachments_dir = state.attachments_dir.clone();
    shutdown(router, command_task, worker, config_task, attachments_dir).await;
    Ok(())
}

/// Filter out TUI-only terminal escape/bell/avatar effects before dispatching
/// to `router`.
fn dispatch_linear(router: &Router, effects: Vec<Effect>, state: &State) {
    let filtered: Vec<Effect> = effects.into_iter().filter(allow_effect_in_linear).collect();
    router.dispatch(filtered, state);
}

const fn allow_effect_in_linear(effect: &Effect) -> bool {
    match effect {
        Effect::CheckTerminal
        | Effect::ListPictures
        | Effect::ReadClipboardImage { .. }
        | Effect::Bell
        | Effect::DesktopNotify { .. }
        | Effect::Copy(_)
        | Effect::CheckForUpdate => false,
        Effect::Backend(_)
        | Effect::OpenUrl(_)
        | Effect::OpenFile(_)
        | Effect::PlayAudio { .. }
        | Effect::StopAudio
        | Effect::SaveToDownloads { .. }
        | Effect::ListFiles(_)
        | Effect::CheckCommand(_)
        | Effect::StageFile(_)
        | Effect::CheckPaste { .. }
        | Effect::ReleaseFocus
        | Effect::SaveSetting(_)
        | Effect::LoadConversations
        | Effect::LoadMessages(_)
        | Effect::LoadOlder { .. }
        | Effect::LoadMessagesAround { .. }
        | Effect::LoadQuoteThread { .. }
        | Effect::SearchMessages { .. }
        | Effect::LoadMessageReceipts { .. }
        | Effect::Persist(_)
        | Effect::SaveAttachment { .. }
        | Effect::InlineLongText { .. }
        | Effect::MarkRead(_)
        | Effect::ReadElsewhere(_)
        | Effect::SweepExpired(_)
        | Effect::SaveDisappearingTimer(..)
        | Effect::DeleteConversation(_)
        | Effect::SaveConversationState(..)
        | Effect::SaveNickname(..)
        | Effect::SaveVerification(..)
        | Effect::SaveRecentReactions(_)
        | Effect::SaveSavedSearches(_)
        | Effect::LoadUpdateState
        | Effect::SaveUpdateState { .. }
        | Effect::LoadSession
        | Effect::SaveSession(_)
        | Effect::RememberOpen(_) => true,
    }
}

/// Write plain-text lines to `out` and flush. Returns `Ok(false)` on
/// `BrokenPipe` so pipelines like `faro demo --linear | head -n 5` exit 0.
fn write_lines<W: Write>(out: &mut W, lines: &[String]) -> anyhow::Result<bool> {
    if lines.is_empty() {
        return Ok(true);
    }
    for line in lines {
        if let Err(err) = writeln!(out, "{line}") {
            if err.kind() == std::io::ErrorKind::BrokenPipe {
                return Ok(false);
            }
            return Err(err).context("writing linear transcript");
        }
    }
    if let Err(err) = out.flush() {
        if err.kind() == std::io::ErrorKind::BrokenPipe {
            return Ok(false);
        }
        return Err(err).context("flushing linear transcript");
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use faro_core::config::Config;
    use faro_core::mock::{MockBackend, MockConfig};
    use faro_core::store::{Profile, Store};

    use super::*;
    use crate::app::{load_directory, state};

    #[test]
    #[expect(
        clippy::disallowed_methods,
        reason = "test drives a single-threaded runtime and LocalSet"
    )]
    fn linear_runner_streams_initial_transcript_executes_commands_and_sends_messages() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let local = tokio::task::LocalSet::new();

        local.block_on(&rt, async {
            let store = Store::open_in_memory().await.unwrap();
            let mock_cfg = MockConfig {
                seed: 7,
                activity: None,
                disconnects: false,
                errors: false,
                ..MockConfig::default()
            };
            let (backend, events) = MockBackend::new(&mock_cfg);
            let directory = load_directory(&backend, &store).await.unwrap();

            let temp = std::env::temp_dir().join("faro-linear-test");
            let startup = Startup {
                path: temp.join("config.toml"),
                attachments_dir: temp.join("attachments"),
                pictures_dir: temp.join("pictures"),
                config: Config::default(),
                problems: Vec::new(),
                themes: Vec::new(),
                profile: Profile::Demo,
                passphrase_verifier: None,
            };

            let (tx, rx) = mpsc::unbounded_channel();
            for line in [
                "/chats",
                "Hello from linear test — مرحبا — 日本語",
                "/status",
                "/quit",
            ] {
                tx.send(Ok(line.to_owned())).unwrap();
            }
            drop(tx);

            let mut output: Vec<u8> = Vec::new();

            run_with_io(
                backend,
                events,
                store.clone(),
                directory,
                jiff::tz::TimeZone::UTC,
                state::Detected::default(),
                startup,
                rx,
                &mut output,
            )
            .await
            .unwrap();

            let transcript = String::from_utf8(output).unwrap();
            assert!(
                transcript.starts_with("# faro linear — Chat: "),
                "starts with linear header:\n{transcript}"
            );
            assert!(
                !transcript.contains('\x1b'),
                "never emits ANSI escape sequences:\n{transcript}"
            );
            assert!(
                transcript.contains("* Conversations:"),
                "lists conversations:\n{transcript}"
            );
            assert!(
                transcript.contains("You: Hello from linear test — مرحبا — 日本語"),
                "streams sent message with RTL and CJK intact:\n{transcript}"
            );
            assert!(
                transcript.contains("* Status: chat: "),
                "prints status:\n{transcript}"
            );
            store.close().await.unwrap();
        });
    }
}
