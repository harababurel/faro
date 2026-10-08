//! Effects on the machine faro runs on: the clipboard and the bell (through
//! the terminal) and opening links. Run by the app loop, never the reducer.

use std::io::Write as _;
use std::process::Stdio;

use crossterm::clipboard::CopyToClipboard;
use tokio::sync::mpsc;

use super::event::AppEvent;
use super::state::BlockGlyphs;

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

/// What faro can find out about the terminals between it and the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Terminal {
    /// mosh is in the way: it measures emoji the old way (D47).
    pub mosh: bool,
    /// Who draws block, Powerline and Legacy Computing glyphs (D58).
    pub glyphs: BlockGlyphs,
}

/// Look at the terminals between faro and the user. Outside tmux: whether
/// faro runs under `mosh-server`, and whether the environment says kitty.
/// Inside tmux: the same for every client attached to faro's session (tmux
/// can't tell us about the terminal behind it otherwise; one client is
/// enough to break its view). Any failure counts as "no mosh" and "glyphs
/// from the font".
pub async fn inspect_terminal() -> Terminal {
    let check = async {
        let table = run("ps", &["-A", "-o", "pid=,ppid=,comm="])
            .await
            .map(|ps| processes(&ps));
        let mosh = |pid: u32| table.as_ref().is_some_and(|t| under_mosh(t, pid));
        if std::env::var_os("TMUX").is_some() {
            let pane = std::env::var("TMUX_PANE").ok();
            let mut args = vec!["list-clients", "-F", "#{client_pid} #{client_termname}"];
            if let Some(pane) = pane.as_deref() {
                args.extend(["-t", pane]);
            }
            let clients = clients(&run("tmux", &args).await.unwrap_or_default());
            Terminal {
                mosh: clients.iter().any(|(pid, _)| mosh(*pid)),
                glyphs: glyphs_of_clients(&clients),
            }
        } else {
            Terminal {
                mosh: mosh(std::process::id()),
                glyphs: glyphs_of_environment(|var| std::env::var(var).ok()),
            }
        }
    };
    if let Ok(found) = tokio::time::timeout(CHECK_TIMEOUT, check).await {
        found
    } else {
        tracing::warn!("checking the terminal took too long; assuming no mosh and no kitty");
        Terminal::default()
    }
}

/// Check the terminal in the background; the answer comes back as
/// `AppEvent::TerminalChecked`.
pub fn check_terminal(results: mpsc::UnboundedSender<AppEvent>) {
    tokio::spawn(async move {
        let terminal = inspect_terminal().await;
        // The app may have quit meanwhile.
        let _ = results.send(AppEvent::TerminalChecked(terminal));
    });
}

/// Whether `term` (a `TERM` value) is kitty, which draws box drawing, block
/// elements, Powerline (U+E0B0–E0BF) and Legacy Computing (U+1FB00–1FBAE)
/// glyphs itself (kitty's `fonts.c`, `font_for_cell`), whatever the font.
fn is_kitty(term: &str) -> bool {
    term.contains("kitty")
}

/// Outside tmux: kitty, as far as the environment says (`TERM` is forwarded
/// over SSH; `KITTY_WINDOW_ID` and `TERM_PROGRAM` only locally).
fn glyphs_of_environment(var: impl Fn(&str) -> Option<String>) -> BlockGlyphs {
    let kitty = var("TERM").is_some_and(|t| is_kitty(&t))
        || var("TERM_PROGRAM").is_some_and(|t| t.eq_ignore_ascii_case("kitty"))
        || var("KITTY_WINDOW_ID").is_some();
    if kitty {
        BlockGlyphs::Native
    } else {
        BlockGlyphs::Font
    }
}

/// Inside tmux: native only if every attached client is in kitty.
fn glyphs_of_clients(clients: &[(u32, String)]) -> BlockGlyphs {
    if !clients.is_empty() && clients.iter().all(|(_, term)| is_kitty(term)) {
        BlockGlyphs::Native
    } else {
        BlockGlyphs::Font
    }
}

/// `tmux list-clients -F "#{client_pid} #{client_termname}"` output.
fn clients(text: &str) -> Vec<(u32, String)> {
    text.lines()
        .filter_map(|line| {
            let (pid, term) = line.trim().split_once(' ').unwrap_or((line.trim(), ""));
            Some((pid.parse().ok()?, term.trim().to_owned()))
        })
        .collect()
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

    #[test]
    fn kitty_is_found_in_the_environment() {
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            move |var: &str| {
                pairs
                    .iter()
                    .find(|(k, _)| *k == var)
                    .map(|(_, v)| (*v).to_owned())
            }
        };
        let native = BlockGlyphs::Native;
        assert_eq!(
            glyphs_of_environment(env(&[("TERM", "xterm-kitty")])),
            native
        );
        assert_eq!(
            glyphs_of_environment(env(&[("KITTY_WINDOW_ID", "3")])),
            native
        );
        assert_eq!(
            glyphs_of_environment(env(&[("TERM_PROGRAM", "kitty")])),
            native
        );
        let font = BlockGlyphs::Font;
        assert_eq!(
            glyphs_of_environment(env(&[("TERM", "xterm-256color")])),
            font
        );
        assert_eq!(glyphs_of_environment(env(&[])), font);
    }

    #[test]
    fn every_tmux_client_must_be_kitty() {
        let kitty = clients("102 xterm-kitty\n202 xterm-kitty\n");
        assert_eq!(
            kitty,
            [
                (102, "xterm-kitty".to_owned()),
                (202, "xterm-kitty".to_owned())
            ]
        );
        assert_eq!(glyphs_of_clients(&kitty), BlockGlyphs::Native);
        let mixed = clients("102 xterm-kitty\n202 xterm-256color\n");
        assert_eq!(glyphs_of_clients(&mixed), BlockGlyphs::Font);
        assert_eq!(glyphs_of_clients(&clients("102\n")), BlockGlyphs::Font);
        assert_eq!(glyphs_of_clients(&[]), BlockGlyphs::Font);
    }
}
