//! Effects on the machine faro runs on: the clipboard (through the terminal)
//! and opening links. Run by the app loop, never the reducer.

use std::io::Write as _;
use std::process::Stdio;

use crossterm::clipboard::CopyToClipboard;
use tokio::sync::mpsc;

use super::event::AppEvent;

/// Copy with OSC 52: the terminal (even over SSH, and through tmux with
/// `set-clipboard on`) puts the text on the local clipboard.
pub fn copy(text: &str) {
    let mut out = std::io::stdout();
    let written = crossterm::execute!(out, CopyToClipboard::to_clipboard_from(text))
        .and_then(|()| out.flush());
    if let Err(error) = written {
        tracing::warn!(%error, "writing the clipboard sequence failed");
    }
}

/// Open `url` with `open` (macOS) or `xdg-open` (elsewhere), without waiting.
/// Failures come back as `AppEvent::OpenFailed`.
pub fn open_url(url: String, results: mpsc::UnboundedSender<AppEvent>) {
    let opener = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    tokio::spawn(async move {
        let status = tokio::process::Command::new(opener)
            .arg(&url)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(false)
            .status()
            .await;
        let failure = match status {
            Ok(status) if status.success() => return,
            Ok(status) => format!("{opener} exited with {status}"),
            Err(error) => format!("couldn't run {opener}: {error}"),
        };
        tracing::warn!(%failure, "opening a link failed");
        // The app may have quit meanwhile.
        let _ = results.send(AppEvent::OpenFailed(failure));
    });
}

/// Whether faro runs in an SSH session (as far as the environment says; a
/// tmux server started locally and attached over SSH later won't show it).
pub fn over_ssh() -> bool {
    ["SSH_CONNECTION", "SSH_CLIENT", "SSH_TTY"]
        .iter()
        .any(|var| std::env::var_os(var).is_some())
}
