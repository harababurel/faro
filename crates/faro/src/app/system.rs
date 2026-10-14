//! Effects on the machine faro runs on: the clipboard and the bell (through
//! the terminal) and opening links. Run by the app loop, never the reducer.

use std::ffi::OsString;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use crossterm::clipboard::CopyToClipboard;
use faro_core::config::DesktopNotifications;
use faro_core::paths;
use tokio::sync::mpsc;

use super::event::AppEvent;
use super::files::{expand_tilde, split_file_query};
use super::state::{
    BlockGlyphs, FileEntry, GraphicsBlocker, GraphicsSupport, MAX_ATTACHMENT_SIZE,
    StagedAttachment, Terminal,
};

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

/// Send a desktop notification (DESIGN.md §8): through the terminal with
/// OSC 99 (kitty) or OSC 777 (wrapped in tmux DCS passthrough inside tmux),
/// or with `osascript` / `notify-send` when `mode == Native` on a local
/// session.
pub fn desktop_notify(
    title: String,
    body: String,
    mode: DesktopNotifications,
    glyphs: BlockGlyphs,
    is_remote: bool,
) {
    match mode {
        DesktopNotifications::Off => {}
        DesktopNotifications::Native if !is_remote => native_notify(title, body),
        DesktopNotifications::Auto | DesktopNotifications::Osc | DesktopNotifications::Native => {
            let kitty = glyphs == BlockGlyphs::Native
                || glyphs_of_environment(|var| std::env::var(var).ok()) == BlockGlyphs::Native;
            if let Err(error) = crate::tui::Tui::notify(&title, &body, kitty) {
                tracing::warn!(%error, "writing the desktop notification sequence failed");
            }
        }
    }
}

fn native_notify(title: String, body: String) {
    tokio::spawn(async move {
        let result = if cfg!(target_os = "macos") {
            tokio::process::Command::new("osascript")
                .args([
                    "-e",
                    "on run argv",
                    "-e",
                    "display notification (item 2 of argv) with title (item 1 of argv)",
                    "-e",
                    "end run",
                    "--",
                ])
                .arg(&title)
                .arg(&body)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .kill_on_drop(false)
                .status()
                .await
        } else {
            tokio::process::Command::new("notify-send")
                .args(["-a", "faro", "--"])
                .arg(&title)
                .arg(&body)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .kill_on_drop(false)
                .status()
                .await
        };
        if let Err(error) = result {
            tracing::warn!(%error, "sending native desktop notification failed");
        }
    });
}

/// Open `url` with `open` (macOS) or `xdg-open` (elsewhere), without waiting.
/// Failures come back as `AppEvent::OpenFailed`.
pub fn open_url(url: String, results: mpsc::UnboundedSender<AppEvent>) {
    open_with_system(OsString::from(url), results);
}

/// Open a local file with `open` (macOS) or `xdg-open` (elsewhere), without
/// waiting (D75). Failures come back as `AppEvent::OpenFailed`.
pub fn open_file(path: PathBuf, results: mpsc::UnboundedSender<AppEvent>) {
    open_with_system(path.into_os_string(), results);
}

fn open_with_system(target: OsString, results: mpsc::UnboundedSender<AppEvent>) {
    let opener = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    tokio::spawn(async move {
        let status = tokio::process::Command::new(opener)
            .arg(&target)
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
        tracing::warn!(%failure, "opening with system viewer failed");
        // The app may have quit meanwhile.
        let _ = results.send(AppEvent::OpenFailed(failure));
    });
}

/// Copy `source` into `~/Downloads/` (or `$XDG_DOWNLOAD_DIR`), numbering
/// collisions so an existing file is never overwritten (D75).
pub fn save_to_downloads(
    source: PathBuf,
    file_name: Option<String>,
    content_type: String,
    results: mpsc::UnboundedSender<AppEvent>,
) {
    tokio::spawn(async move {
        let outcome = tokio::task::spawn_blocking(move || {
            let dir = paths::resolve_downloads_dir().map_err(|e| e.to_string())?;
            paths::save_to_downloads(&source, &dir, file_name.as_deref(), &content_type)
                .map_err(|e| e.to_string())
        })
        .await
        .map_err(|e| e.to_string())
        .and_then(|r| r);
        let _ = results.send(AppEvent::SavedToDownloads(outcome));
    });
}

/// Manages a single external audio player child process (D81). Starting a new
/// voice note or calling [`AudioPlayer::stop`] aborts the running task, which
/// drops and kills (`kill_on_drop(true)`) the child process immediately.
#[derive(Default)]
pub struct AudioPlayer {
    task: Option<tokio::task::JoinHandle<()>>,
}

impl Drop for AudioPlayer {
    fn drop(&mut self) {
        self.stop();
    }
}

impl AudioPlayer {
    pub fn stop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }

    pub fn play(
        &mut self,
        message: faro_core::domain::MessageRef,
        index: usize,
        path: PathBuf,
        player: String,
        results: mpsc::UnboundedSender<AppEvent>,
    ) {
        self.stop();
        self.task = Some(tokio::spawn(async move {
            let error = run_audio_player(&path, &player).await.err();
            if let Some(ref err) = error {
                tracing::warn!(%err, "audio playback failed");
            }
            let _ = results.send(AppEvent::PlaybackFinished {
                message,
                index,
                error,
            });
        }));
    }
}

const AUTO_PLAYERS_MACOS: &[(&str, &[&str])] = &[
    ("afplay", &[]),
    ("mpv", &["--no-video", "--really-quiet"]),
    ("ffplay", &["-nodisp", "-autoexit", "-loglevel", "quiet"]),
    ("play", &["-q"]),
];

const AUTO_PLAYERS_UNIX: &[(&str, &[&str])] = &[
    ("mpv", &["--no-video", "--really-quiet"]),
    ("ffplay", &["-nodisp", "-autoexit", "-loglevel", "quiet"]),
    ("paplay", &[]),
    ("aplay", &["-q"]),
    ("play", &["-q"]),
];

async fn run_audio_player(path: &Path, player: &str) -> Result<(), String> {
    let trimmed = player.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("auto") {
        let candidates = if cfg!(target_os = "macos") {
            AUTO_PLAYERS_MACOS
        } else {
            AUTO_PLAYERS_UNIX
        };
        for &(program, args) in candidates {
            let status = tokio::process::Command::new(program)
                .args(args)
                .arg(path)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .kill_on_drop(true)
                .status()
                .await;
            match status {
                Ok(status) if status.success() => return Ok(()),
                Ok(status) => return Err(format!("{program} exited with {status}")),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(format!("couldn't run {program}: {error}")),
            }
        }
        return Err(
            "no audio player found (install mpv, ffplay, paplay or aplay, or set [voice] player)"
                .to_owned(),
        );
    }
    let mut parts = trimmed.split_whitespace();
    let Some(program) = parts.next() else {
        return Err("no audio player configured".to_owned());
    };
    let status = tokio::process::Command::new(program)
        .args(parts)
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .status()
        .await;
    match status {
        Ok(status) if status.success() => Ok(()),
        Ok(status) => Err(format!("{program} exited with {status}")),
        Err(error) => Err(format!("couldn't run {program}: {error}")),
    }
}

/// Read directory entries for the palette's `Level::AttachFile` off the UI thread (D77).
pub fn list_files(query: String, results: mpsc::UnboundedSender<AppEvent>) {
    tokio::spawn(async move {
        let q = query.clone();
        let entries = tokio::task::spawn_blocking(move || list_files_blocking(&q))
            .await
            .unwrap_or_default();
        let _ = results.send(AppEvent::FilesListed { query, entries });
    });
}

#[expect(
    clippy::disallowed_methods,
    reason = "directory listing runs inside spawn_blocking off the UI thread"
)]
fn list_files_blocking(query: &str) -> Vec<FileEntry> {
    let (dir_prefix, file_prefix) = split_file_query(query);
    let dir_path = if dir_prefix.is_empty() {
        PathBuf::from(".")
    } else {
        expand_tilde(dir_prefix)
    };
    let show_dotfiles = file_prefix.starts_with('.');
    let Ok(read_dir) = std::fs::read_dir(&dir_path) else {
        return Vec::new();
    };
    let mut entries = Vec::new();
    for entry in read_dir.flatten() {
        let file_name = entry.file_name();
        let name = file_name.to_string_lossy().into_owned();
        if name.starts_with('.') && !show_dotfiles {
            continue;
        }
        let path = entry.path();
        let Ok(meta) = std::fs::metadata(&path) else {
            continue;
        };
        let is_dir = meta.is_dir();
        if !is_dir && !meta.is_file() {
            continue;
        }
        let dir_query = if dir_prefix.is_empty() {
            format!("{name}/")
        } else {
            format!("{dir_prefix}{name}/")
        };
        entries.push(FileEntry {
            name,
            path,
            dir_query,
            is_dir,
            size: if is_dir { 0 } else { meta.len() },
        });
    }
    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then_with(|| a.name.cmp(&b.name))
    });
    entries
}

/// Inspect and stage a file for sending off the UI thread (D77).
pub fn stage_file(path: PathBuf, results: mpsc::UnboundedSender<AppEvent>) {
    tokio::spawn(async move {
        let outcome = tokio::task::spawn_blocking(move || stage_file_blocking(path))
            .await
            .map_err(|e| e.to_string())
            .and_then(|r| r);
        let _ = results.send(AppEvent::FileStaged(outcome));
    });
}

/// Check a pasted candidate path off the UI thread: if it is a regular file on
/// disk, stage it (or report >100 MB error); otherwise paste the raw text (D77).
pub fn check_paste(raw: String, path: PathBuf, results: mpsc::UnboundedSender<AppEvent>) {
    tokio::spawn(async move {
        let checked = tokio::task::spawn_blocking(move || check_paste_blocking(path))
            .await
            .ok()
            .flatten();
        let event = match checked {
            Some(Ok(staged)) => AppEvent::PasteChecked {
                raw,
                staged: Some(staged),
            },
            Some(Err(err)) => AppEvent::FileStaged(Err(err)),
            None => AppEvent::PasteChecked { raw, staged: None },
        };
        let _ = results.send(event);
    });
}

#[expect(
    clippy::disallowed_methods,
    reason = "paste path check runs inside spawn_blocking off the UI thread"
)]
fn check_paste_blocking(path: PathBuf) -> Option<Result<StagedAttachment, String>> {
    let meta = std::fs::metadata(&path).ok()?;
    if !meta.is_file() {
        return None;
    }
    Some(stage_file_blocking(path))
}

#[expect(
    clippy::disallowed_methods,
    reason = "file metadata and image dimension probing run inside spawn_blocking"
)]
fn stage_file_blocking(path: PathBuf) -> Result<StagedAttachment, String> {
    let meta = std::fs::metadata(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    if !meta.is_file() {
        return Err(format!("not a regular file: {}", path.display()));
    }
    let size = meta.len();
    if size > MAX_ATTACHMENT_SIZE {
        return Err(format!(
            "file exceeds 100 MB limit ({})",
            faro_core::domain::format_byte_size(size)
        ));
    }
    let name = path.file_name().map_or_else(
        || "attachment".to_owned(),
        |n| n.to_string_lossy().into_owned(),
    );
    let content_type = guess_content_type(&path);
    let (width, height) = if content_type.starts_with("image/") {
        image::ImageReader::open(&path)
            .and_then(image::ImageReader::with_guessed_format)
            .ok()
            .and_then(|r| r.into_dimensions().ok())
            .map_or((None, None), |(w, h)| (Some(w), Some(h)))
    } else {
        (None, None)
    };
    Ok(StagedAttachment {
        path,
        name,
        content_type,
        size,
        width,
        height,
    })
}

fn guess_content_type(path: &Path) -> String {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase);
    let mime = match ext.as_deref() {
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("png") => "image/png",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("bmp") => "image/bmp",
        Some("svg") => "image/svg+xml",
        Some("pdf") => "application/pdf",
        Some("txt" | "md" | "rst") => "text/plain",
        Some("json") => "application/json",
        Some("csv") => "text/csv",
        Some("mp3") => "audio/mpeg",
        Some("ogg") => "audio/ogg",
        Some("wav") => "audio/wav",
        Some("m4a") => "audio/mp4",
        Some("mp4") => "video/mp4",
        Some("webm") => "video/webm",
        Some("mov") => "video/quicktime",
        Some("zip") => "application/zip",
        Some("tar") => "application/x-tar",
        Some("gz" | "tgz") => "application/gzip",
        _ => "application/octet-stream",
    };
    mime.to_owned()
}

/// Whether faro runs in an SSH session (as far as the environment says; a
/// tmux server started locally and attached over SSH later won't show it).
pub fn over_ssh() -> bool {
    ["SSH_CONNECTION", "SSH_CLIENT", "SSH_TTY"]
        .iter()
        .any(|var| std::env::var_os(var).is_some())
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
            let mut args = vec![
                "list-clients",
                "-F",
                "#{client_pid} #{client_tty} #{client_termname}",
            ];
            if let Some(pane) = pane.as_deref() {
                args.extend(["-t", pane]);
            }
            let clients = clients(&run("tmux", &args).await.unwrap_or_default());
            let any_mosh = clients.iter().any(|c| mosh(c.pid));
            let (graphics, graphics_blocker) = graphics_of_clients(&clients, mosh);
            Terminal {
                mosh: any_mosh,
                glyphs: glyphs_of_clients(&clients),
                graphics,
                kitty: clients.iter().any(|c| is_kitty(&c.term) && !mosh(c.pid)),
                graphics_blocker,
            }
        } else {
            let under_mosh = mosh(std::process::id());
            let glyphs = glyphs_of_environment(|var| std::env::var(var).ok());
            Terminal {
                mosh: under_mosh,
                glyphs,
                kitty: glyphs == BlockGlyphs::Native && !under_mosh,
                graphics: if under_mosh {
                    GraphicsSupport::HalfblocksOnly
                } else {
                    GraphicsSupport::Direct
                },
                graphics_blocker: under_mosh.then_some(GraphicsBlocker::Mosh),
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
fn glyphs_of_clients(clients: &[Client]) -> BlockGlyphs {
    if !clients.is_empty() && clients.iter().all(|c| is_kitty(&c.term)) {
        BlockGlyphs::Native
    } else {
        BlockGlyphs::Font
    }
}

/// Inside tmux: terminal graphics protocols are safe only when at least one
/// client is attached, none is under mosh, and all attached clients share the
/// same `client_termname` (D78); otherwise, which client is in the way
/// (D137). Among different terminals the odd one out is a non-kitty one when
/// some are kitty (the user's own is likely the kitty), else the first that
/// differs from the first.
fn graphics_of_clients(
    clients: &[Client],
    mosh: impl Fn(u32) -> bool,
) -> (GraphicsSupport, Option<GraphicsBlocker>) {
    let blocker = if let Some(on_mosh) = clients.iter().find(|c| mosh(c.pid)) {
        Some(GraphicsBlocker::MoshClient(on_mosh.tty.clone()))
    } else if let Some(first) = clients.first() {
        let odd = if first.term.is_empty() {
            Some(first)
        } else if clients.iter().all(|c| c.term == first.term) {
            None
        } else if clients.iter().any(|c| is_kitty(&c.term)) {
            clients.iter().find(|c| !is_kitty(&c.term))
        } else {
            clients.iter().find(|c| c.term != first.term)
        };
        odd.map(|c| GraphicsBlocker::OtherClient {
            tty: c.tty.clone(),
            term: c.term.clone(),
        })
    } else {
        Some(GraphicsBlocker::NoClient)
    };
    let graphics = if blocker.is_some() {
        GraphicsSupport::HalfblocksOnly
    } else {
        GraphicsSupport::Direct
    };
    (graphics, blocker)
}

/// A tmux client attached to faro's session.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Client {
    pid: u32,
    /// Its tty, as `tmux detach-client -t` takes it.
    tty: String,
    /// Its `TERM` (`client_termname`); empty if tmux doesn't know.
    term: String,
}

/// `tmux list-clients -F "#{client_pid} #{client_tty} #{client_termname}"`
/// output.
fn clients(text: &str) -> Vec<Client> {
    text.lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let pid = fields.next()?.parse().ok()?;
            let tty = fields.next().unwrap_or_default().to_owned();
            let term = fields.next().unwrap_or_default().to_owned();
            Some(Client { pid, tty, term })
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
        let kitty = clients("102 /dev/ttys001 xterm-kitty\n202 /dev/ttys002 xterm-kitty\n");
        assert_eq!(kitty.len(), 2);
        assert_eq!(kitty[1].tty, "/dev/ttys002");
        assert_eq!(glyphs_of_clients(&kitty), BlockGlyphs::Native);
        let mixed = clients("102 /dev/ttys001 xterm-kitty\n202 /dev/ttys002 xterm-256color\n");
        assert_eq!(glyphs_of_clients(&mixed), BlockGlyphs::Font);
        assert_eq!(glyphs_of_clients(&clients("102\n")), BlockGlyphs::Font);
        assert_eq!(glyphs_of_clients(&[]), BlockGlyphs::Font);
    }

    /// Graphics only with one kind of terminal and no mosh, and otherwise
    /// the client in the way, so the note can name it (D137).
    #[test]
    fn the_client_in_the_way_of_graphics_is_named() {
        let no_mosh = |_| false;
        let kitty = clients("102 /dev/ttys001 xterm-kitty\n202 /dev/ttys002 xterm-kitty\n");
        assert_eq!(
            graphics_of_clients(&kitty, no_mosh),
            (GraphicsSupport::Direct, None)
        );
        assert_eq!(
            graphics_of_clients(&kitty, |pid| pid == 202),
            (
                GraphicsSupport::HalfblocksOnly,
                Some(GraphicsBlocker::MoshClient("/dev/ttys002".to_owned()))
            )
        );
        // The owner's case: a forgotten client in another terminal; the
        // non-kitty one is named, whichever comes first.
        let mixed = clients("202 /dev/ttys022 xterm-256color\n102 /dev/ttys011 xterm-kitty\n");
        assert_eq!(
            graphics_of_clients(&mixed, no_mosh).1,
            Some(GraphicsBlocker::OtherClient {
                tty: "/dev/ttys022".to_owned(),
                term: "xterm-256color".to_owned()
            })
        );
        let neither = clients("1 /dev/a xterm\n2 /dev/b screen\n");
        assert_eq!(
            graphics_of_clients(&neither, no_mosh).1,
            Some(GraphicsBlocker::OtherClient {
                tty: "/dev/b".to_owned(),
                term: "screen".to_owned()
            })
        );
        assert_eq!(
            graphics_of_clients(&[], no_mosh),
            (
                GraphicsSupport::HalfblocksOnly,
                Some(GraphicsBlocker::NoClient)
            )
        );
    }
}
