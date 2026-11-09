//! CLI encryption subcommands (`faro encrypt`, `faro decrypt`, `faro passwd`)
//! and startup passphrase unlock for both CLI commands and the TUI (D168).

use std::io::{BufRead as _, Write as _};
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use anyhow::Context as _;
use crossterm::event::EventStream;
use faro_app::app::state::PassphraseVerifier;
use faro_app::app::term_event::{Event, KeyCode, KeyEventKind, KeyModifiers};
use faro_app::theme::{ColorDepth, Themes, UserThemes};
use faro_app::ui::lock::render_prompt_card;
use faro_core::config::Config;
use faro_core::paths::Paths;
use faro_core::vault::{self, VaultError};
use futures_util::StreamExt as _;
use ratatui::widgets::Block;

use crate::theme::ThemeText;
use crate::tui::Tui;

/// Outcome of unlocking a data directory before starting the TUI.
pub enum UnlockOutcome {
    /// Directory is not encrypted (`vault.json` does not exist).
    Unencrypted,
    /// Directory was unlocked and process-wide keys were installed.
    Unlocked(PassphraseVerifier),
    /// User pressed `Esc` or `Ctrl+Q` at the passphrase prompt.
    Cancelled,
}

/// `faro encrypt`: encrypt `faro.db`, `presage.db`, `attachments/`, and
/// `pictures/` in `paths.data_dir()` with a passphrase.
pub async fn encrypt(paths: &Paths) -> anyhow::Result<()> {
    let data_dir = paths.data_dir();
    if vault::is_encrypted(data_dir) {
        anyhow::bail!(
            "{} is already encrypted. Use `faro passwd` to change the passphrase or \
             `faro decrypt` to remove encryption",
            data_dir.display()
        );
    }
    let first = prompt_passphrase("New passphrase: ")?;
    if first.is_empty() {
        anyhow::bail!("passphrase cannot be empty");
    }
    let confirm = prompt_passphrase("Confirm passphrase: ")?;
    if first != confirm {
        anyhow::bail!("passphrases do not match");
    }
    say(&format!(
        "Encrypting databases and media in {}…\n",
        data_dir.display()
    ));
    let summary = vault::encrypt_directory(paths, &first).await?;
    say(&format!(
        "Encrypted {} (faro.db: {}, presage.db: {}, {} media files).\n",
        data_dir.display(),
        yes_no(summary.faro_db),
        yes_no(summary.presage_db),
        summary.media_files,
    ));
    Ok(())
}

/// `faro decrypt`: decrypt `faro.db`, `presage.db`, `attachments/`, and
/// `pictures/` in `paths.data_dir()` back to plaintext.
pub async fn decrypt(paths: &Paths, passphrase_command: &str) -> anyhow::Result<()> {
    let data_dir = paths.data_dir();
    if !vault::is_encrypted(data_dir) {
        anyhow::bail!("{} is not encrypted", data_dir.display());
    }
    let passphrase = match try_passphrase_command(passphrase_command).await {
        Some(Ok(pass)) => pass,
        Some(Err(err)) => {
            say(&format!("passphrase_command failed ({err}); prompting.\n"));
            prompt_passphrase("Passphrase: ")?
        }
        None => prompt_passphrase("Passphrase: ")?,
    };
    say(&format!(
        "Decrypting databases and media in {}…\n",
        data_dir.display()
    ));
    let summary = vault::decrypt_directory(paths, &passphrase).await?;
    say(&format!(
        "Decrypted {} back to plaintext (faro.db: {}, presage.db: {}, {} media files).\n",
        data_dir.display(),
        yes_no(summary.faro_db),
        yes_no(summary.presage_db),
        summary.media_files,
    ));
    Ok(())
}

/// `faro passwd`: re-wrap the master key under a new passphrase without
/// rewriting databases or media files.
pub async fn passwd(paths: &Paths) -> anyhow::Result<()> {
    let data_dir = paths.data_dir();
    if !vault::is_encrypted(data_dir) {
        anyhow::bail!(
            "{} is not encrypted (run `faro encrypt` first)",
            data_dir.display()
        );
    }
    let current = prompt_passphrase("Current passphrase: ")?;
    let next = prompt_passphrase("New passphrase: ")?;
    if next.is_empty() {
        anyhow::bail!("new passphrase cannot be empty");
    }
    let confirm = prompt_passphrase("Confirm new passphrase: ")?;
    if next != confirm {
        anyhow::bail!("new passphrases do not match");
    }
    let paths_clone = paths.clone();
    tokio::task::spawn_blocking(move || vault::change_passphrase(&paths_clone, &current, &next))
        .await
        .context("joining passphrase update task")??;
    say("Passphrase updated.\n");
    Ok(())
}

/// Unlock `data_dir` for CLI subcommands (`faro link`, `faro ids`,
/// `faro refresh-pictures`) when `vault.json` is present.
pub async fn unlock_cli(
    data_dir: &Path,
    passphrase_command: &str,
) -> anyhow::Result<Option<PassphraseVerifier>> {
    if !vault::is_encrypted(data_dir) {
        return Ok(None);
    }
    if let Some(res) = try_passphrase_command(passphrase_command).await {
        match res {
            Ok(pass) => match unlock_dir(data_dir, pass).await? {
                Ok(verifier) => return Ok(Some(verifier)),
                Err(VaultError::WrongPassphrase) => {
                    say("Passphrase from `[security] passphrase_command` was rejected.\n");
                }
                Err(err) => return Err(err.into()),
            },
            Err(err) => {
                say(&format!(
                    "`[security] passphrase_command` failed ({err}); prompting.\n"
                ));
            }
        }
    }
    for attempt in 1..=3 {
        let pass = prompt_passphrase("Passphrase: ")?;
        match unlock_dir(data_dir, pass).await? {
            Ok(verifier) => return Ok(Some(verifier)),
            Err(VaultError::WrongPassphrase) if attempt < 3 => {
                say("Wrong passphrase; try again.\n");
            }
            Err(err) => return Err(err.into()),
        }
    }
    anyhow::bail!("too many failed passphrase attempts")
}

/// Unlock `data_dir` before opening stores for the TUI. Uses
/// `[security] passphrase_command` if configured, falling back to a centered
/// TUI passphrase card.
#[expect(
    clippy::too_many_lines,
    reason = "startup TUI unlock event loop reads best in one place"
)]
pub async fn unlock_tui(
    data_dir: &Path,
    config: &Config,
    theme_files: &[ThemeText],
) -> anyhow::Result<UnlockOutcome> {
    if !vault::is_encrypted(data_dir) {
        return Ok(UnlockOutcome::Unencrypted);
    }
    let mut initial_error: Option<String> = None;
    if let Some(res) = try_passphrase_command(&config.security.passphrase_command).await {
        match res {
            Ok(pass) => match unlock_dir(data_dir, pass).await? {
                Ok(verifier) => return Ok(UnlockOutcome::Unlocked(verifier)),
                Err(VaultError::WrongPassphrase) => {
                    initial_error = Some("passphrase_command returned wrong passphrase".to_owned());
                }
                Err(err) => return Err(err.into()),
            },
            Err(err) => {
                initial_error = Some(format!("passphrase_command failed: {err}"));
            }
        }
    }

    let theme = resolve_unlock_theme(config, theme_files);
    let mut tui = Tui::enter(false).context("setting up the terminal for passphrase prompt")?;
    let mut stream = EventStream::new();
    let mut input = String::new();
    let mut error = initial_error;

    loop {
        draw_unlock_card(
            &mut tui,
            &theme,
            "Enter your passphrase to unlock:",
            input.chars().count(),
            error.as_deref(),
        )?;

        let Some(next) = stream.next().await else {
            return Ok(UnlockOutcome::Cancelled);
        };
        let ev = crate::tui::input::convert(next.context("reading terminal input")?);
        match ev {
            Event::Paste(text) => {
                error = None;
                for ch in text.chars().filter(|&c| c != '\n' && c != '\r') {
                    input.push(ch);
                }
            }
            Event::Key(key) if key.kind != KeyEventKind::Release => {
                if (key.modifiers == KeyModifiers::CONTROL
                    && matches!(key.code, KeyCode::Char('q' | 'c')))
                    || (key.modifiers.is_empty() && key.code == KeyCode::Esc)
                {
                    zeroize::Zeroize::zeroize(&mut input);
                    return Ok(UnlockOutcome::Cancelled);
                }
                match key.code {
                    KeyCode::Enter => {
                        if input.is_empty() {
                            error = Some("Passphrase cannot be empty".to_owned());
                            continue;
                        }
                        let _ = draw_unlock_card(
                            &mut tui,
                            &theme,
                            "Unlocking vault (Argon2id)…",
                            input.chars().count(),
                            None,
                        );
                        let candidate = std::mem::take(&mut input);
                        match unlock_dir(data_dir, candidate).await? {
                            Ok(verifier) => return Ok(UnlockOutcome::Unlocked(verifier)),
                            Err(VaultError::WrongPassphrase) => {
                                error = Some("Wrong passphrase".to_owned());
                            }
                            Err(err) => return Err(err.into()),
                        }
                    }
                    KeyCode::Backspace => {
                        error = None;
                        input.pop();
                    }
                    KeyCode::Char('u') if key.modifiers == KeyModifiers::CONTROL => {
                        error = None;
                        zeroize::Zeroize::zeroize(&mut input);
                    }
                    KeyCode::Char(c)
                        if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT =>
                    {
                        error = None;
                        input.push(c);
                    }
                    KeyCode::Left
                    | KeyCode::Right
                    | KeyCode::Up
                    | KeyCode::Down
                    | KeyCode::Home
                    | KeyCode::End
                    | KeyCode::PageUp
                    | KeyCode::PageDown
                    | KeyCode::Tab
                    | KeyCode::BackTab
                    | KeyCode::Delete
                    | KeyCode::Insert
                    | KeyCode::F(_)
                    | KeyCode::Char(_)
                    | KeyCode::Esc
                    | KeyCode::Other => {}
                }
            }
            Event::FocusGained
            | Event::FocusLost
            | Event::Key(_)
            | Event::Mouse(_)
            | Event::Resize(..) => {}
        }
    }
}

fn resolve_unlock_theme(config: &Config, theme_files: &[ThemeText]) -> faro_app::theme::Theme {
    let mut user_themes = UserThemes::default();
    let _ = user_themes.load(theme_files.to_vec());
    let detected_colors = ColorDepth::detect(|var| std::env::var(var).ok());
    let colors = match config.ui.colors {
        faro_core::config::ColorChoice::Auto => detected_colors,
        faro_core::config::ColorChoice::Truecolor => ColorDepth::Truecolor,
        faro_core::config::ColorChoice::Colors256 => ColorDepth::Indexed256,
        faro_core::config::ColorChoice::Colors16 => ColorDepth::Ansi16,
    };
    let mut themes = Themes::default();
    themes.get(&config.theme, colors, &user_themes).clone()
}

fn draw_unlock_card(
    tui: &mut Tui,
    theme: &faro_app::theme::Theme,
    prompt: &str,
    input_chars: usize,
    error: Option<&str>,
) -> anyhow::Result<()> {
    tui.draw(|frame| {
        let area = frame.area();
        frame.render_widget(Block::default().style(theme.base()), area);
        render_prompt_card(
            frame,
            area,
            theme,
            " faro locked ",
            prompt,
            input_chars,
            error,
            Some("Encrypted at rest (SQLCipher + AES-256-GCM)"),
            "enter unlock · esc / ctrl+q quit",
        );
    })
    .context("drawing passphrase prompt")
}

async fn unlock_dir(
    data_dir: &Path,
    mut passphrase: String,
) -> anyhow::Result<Result<PassphraseVerifier, VaultError>> {
    let dir = data_dir.to_owned();
    tokio::task::spawn_blocking(move || {
        let res = vault::unlock(&dir, &passphrase).map(vault::VaultHeader::verifier);
        zeroize::Zeroize::zeroize(&mut passphrase);
        res
    })
    .await
    .context("joining vault unlock task")
}

async fn try_passphrase_command(command: &str) -> Option<Result<String, String>> {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return None;
    }
    let fut = tokio::process::Command::new("sh")
        .args(["-c", trimmed])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .output();
    let res = match tokio::time::timeout(Duration::from_secs(30), fut).await {
        Ok(Ok(output)) if output.status.success() => {
            let pass = String::from_utf8_lossy(&output.stdout)
                .trim_end_matches(&['\r', '\n'][..])
                .to_owned();
            if pass.is_empty() {
                Err("command printed an empty passphrase".to_owned())
            } else {
                Ok(pass)
            }
        }
        Ok(Ok(output)) => {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
            if stderr.is_empty() {
                Err(format!("exited with {}", output.status))
            } else {
                Err(stderr)
            }
        }
        Ok(Err(err)) => Err(err.to_string()),
        Err(_) => Err("timed out after 30s".to_owned()),
    };
    Some(res)
}

#[expect(
    clippy::disallowed_methods,
    reason = "CLI passphrase prompt runs before the TUI event loop"
)]
fn prompt_passphrase(prompt: &str) -> anyhow::Result<String> {
    if let Ok(mut tty) = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/tty")
    {
        tty.write_all(prompt.as_bytes())
            .and_then(|()| tty.flush())
            .context("writing passphrase prompt to /dev/tty")?;
        let guard = EchoGuard::disable(&tty);
        let mut reader = std::io::BufReader::new(&tty);
        let mut line = String::new();
        reader
            .read_line(&mut line)
            .context("reading passphrase from /dev/tty")?;
        drop(guard);
        let _ = tty.write_all(b"\n").and_then(|()| tty.flush());
        while line.ends_with('\n') || line.ends_with('\r') {
            line.pop();
        }
        return Ok(line);
    }
    say(prompt);
    let mut line = String::new();
    std::io::stdin()
        .lock()
        .read_line(&mut line)
        .context("reading passphrase from stdin")?;
    while line.ends_with('\n') || line.ends_with('\r') {
        line.pop();
    }
    Ok(line)
}

struct EchoGuard<'a> {
    tty: &'a std::fs::File,
    original: Option<rustix::termios::Termios>,
}

impl<'a> EchoGuard<'a> {
    fn disable(tty: &'a std::fs::File) -> Self {
        let original = rustix::termios::tcgetattr(tty).ok();
        if let Some(ref orig) = original {
            let mut next = orig.clone();
            next.local_modes.remove(rustix::termios::LocalModes::ECHO);
            let _ = rustix::termios::tcsetattr(tty, rustix::termios::OptionalActions::Now, &next);
        }
        Self { tty, original }
    }
}

impl Drop for EchoGuard<'_> {
    fn drop(&mut self) {
        if let Some(ref orig) = self.original {
            let _ =
                rustix::termios::tcsetattr(self.tty, rustix::termios::OptionalActions::Now, orig);
        }
    }
}

const fn yes_no(done: bool) -> &'static str {
    if done { "yes" } else { "not present" }
}

#[expect(
    clippy::print_stdout,
    reason = "CLI encryption subcommands print status on stdout before any TUI exists"
)]
fn say(text: &str) {
    print!("{text}");
    let _ = std::io::stdout().flush();
}
