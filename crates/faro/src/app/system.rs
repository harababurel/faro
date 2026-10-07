//! Effects on the machine faro runs on: the clipboard and the bell (through
//! the terminal) and opening links. Run by the app loop, never the reducer.

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

/// Ring the terminal bell (BEL). Under tmux this sets the window's bell flag
/// and runs `alert-bell` hooks (`monitor-bell`, on by default).
pub fn bell() {
    let mut out = std::io::stdout();
    if let Err(error) = out.write_all(b"\x07").and_then(|()| out.flush()) {
        tracing::warn!(%error, "ringing the bell failed");
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

/// Whether a terminal between faro and the user is mosh, which measures emoji
/// the old way (D47). Outside tmux: whether faro itself runs under
/// `mosh-server`. Inside tmux: whether any client attached to faro's session
/// does (tmux can't tell us about the terminal behind it otherwise). Any
/// failure counts as "no".
pub async fn mosh_attached() -> bool {
    let check = async {
        let table = processes(&run("ps", &["-A", "-o", "pid=,ppid=,comm="]).await?);
        let starts: Vec<u32> = if std::env::var_os("TMUX").is_some() {
            let pane = std::env::var("TMUX_PANE").ok();
            let mut args = vec!["list-clients", "-F", "#{client_pid}"];
            if let Some(pane) = pane.as_deref() {
                args.extend(["-t", pane]);
            }
            run("tmux", &args)
                .await?
                .lines()
                .filter_map(|line| line.trim().parse().ok())
                .collect()
        } else {
            vec![std::process::id()]
        };
        Some(starts.into_iter().any(|pid| under_mosh(&table, pid)))
    };
    if let Ok(found) = tokio::time::timeout(CHECK_TIMEOUT, check).await {
        found.unwrap_or(false)
    } else {
        tracing::warn!("checking for mosh took too long; assuming it isn't there");
        false
    }
}

/// Check for mosh in the background; the answer comes back as
/// `AppEvent::TerminalChecked`.
pub fn check_terminal(results: mpsc::UnboundedSender<AppEvent>) {
    tokio::spawn(async move {
        let mosh = mosh_attached().await;
        // The app may have quit meanwhile.
        let _ = results.send(AppEvent::TerminalChecked { mosh });
    });
}

const CHECK_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

/// Run a short command; its output if it succeeded.
async fn run(program: &str, args: &[&str]) -> Option<String> {
    let output = tokio::process::Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .output()
        .await
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

/// `ps -o pid=,ppid=,comm=` output as pid → (parent pid, command).
fn processes(ps: &str) -> std::collections::HashMap<u32, (u32, String)> {
    ps.lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let pid = fields.next()?.parse().ok()?;
            let parent = fields.next()?.parse().ok()?;
            let command = fields.collect::<Vec<_>>().join(" ");
            Some((pid, (parent, command)))
        })
        .collect()
}

/// Whether `pid` or one of its ancestors is a mosh server (`mosh-server`,
/// `gmosh-server`…).
fn under_mosh(table: &std::collections::HashMap<u32, (u32, String)>, mut pid: u32) -> bool {
    for _ in 0..64 {
        let Some((parent, command)) = table.get(&pid) else {
            return false;
        };
        if command.contains("mosh-server") {
            return true;
        }
        if *parent == pid || *parent == 0 {
            return false;
        }
        pid = *parent;
    }
    false
}

#[cfg(test)]
mod mosh_tests {
    use super::*;

    const PS: &str = "    1     0 systemd
  100     1 mosh-server
  101   100 fish
  102   101 tmux: client
  200     1 sshd
  201   200 bash
  202   201 tmux: client
  300     1 /usr/local/bin/gmosh-server
  301   300 zsh
";

    #[test]
    fn finds_mosh_among_the_ancestors() {
        let table = processes(PS);
        assert!(under_mosh(&table, 102));
        assert!(under_mosh(&table, 301));
        assert!(!under_mosh(&table, 202));
        assert!(!under_mosh(&table, 1));
        assert!(!under_mosh(&table, 999));
    }

    #[test]
    fn commands_with_spaces_are_kept_whole() {
        let table = processes(PS);
        assert_eq!(table.get(&102), Some(&(101, "tmux: client".to_owned())));
    }
}
