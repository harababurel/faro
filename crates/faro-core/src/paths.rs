//! Where faro keeps its config and data (DECISIONS D17).
//!
//! XDG layout on every Unix, including macOS:
//! - config: `$XDG_CONFIG_HOME/faro/config.toml` (default `~/.config/faro/config.toml`)
//! - data:   `$XDG_DATA_HOME/faro/` (default `~/.local/share/faro/`)
//!
//! Directories faro creates are `0700`, files are `0600`.

use std::fs::{self, DirBuilder, File, OpenOptions};
use std::io::{self, Write as _};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use etcetera::BaseStrategy;

use crate::domain::MessageRef;

const APP_DIR: &str = "faro";
const CONFIG_FILE: &str = "config.toml";
const LOG_DIR: &str = "logs";
const ATTACHMENTS_DIR: &str = "attachments";
const DB_FILE: &str = "faro.db";

/// Permissions for directories faro creates.
pub const DIR_MODE: u32 = 0o700;
/// Permissions for files faro creates.
pub const FILE_MODE: u32 = 0o600;

#[derive(Debug, thiserror::Error)]
pub enum PathsError {
    #[error("could not determine the home directory")]
    NoHomeDir,
}

/// Resolved locations of faro's files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    config_file: PathBuf,
    data_dir: PathBuf,
}

impl Paths {
    /// Resolve paths from the environment, applying CLI/env overrides.
    pub fn resolve(
        config_override: Option<PathBuf>,
        data_dir_override: Option<PathBuf>,
    ) -> Result<Self, PathsError> {
        let xdg = etcetera::base_strategy::Xdg::new().map_err(|_| PathsError::NoHomeDir)?;
        Ok(Self::from_bases(
            &xdg.config_dir(),
            &xdg.data_dir(),
            config_override,
            data_dir_override,
        ))
    }

    /// Build paths from explicit XDG base directories. Pure; used by `resolve` and tests.
    pub fn from_bases(
        config_home: &Path,
        data_home: &Path,
        config_override: Option<PathBuf>,
        data_dir_override: Option<PathBuf>,
    ) -> Self {
        Self {
            config_file: config_override
                .unwrap_or_else(|| config_home.join(APP_DIR).join(CONFIG_FILE)),
            data_dir: data_dir_override.unwrap_or_else(|| data_home.join(APP_DIR)),
        }
    }

    pub fn config_file(&self) -> &Path {
        &self.config_file
    }

    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    pub fn log_dir(&self) -> PathBuf {
        self.data_dir.join(LOG_DIR)
    }

    /// Where downloaded and outgoing attachment copies live (D75, D77).
    pub fn attachments_dir(&self) -> PathBuf {
        self.data_dir.join(ATTACHMENTS_DIR)
    }

    /// faro's own database (history, conversation state), not presage's.
    pub fn db_file(&self) -> PathBuf {
        self.data_dir.join(DB_FILE)
    }

    /// Create the data, log and attachments directories (`0700`) if they don't exist.
    pub fn ensure_data_dirs(&self) -> io::Result<()> {
        create_private_dir_all(&self.data_dir)?;
        create_private_dir_all(&self.log_dir())?;
        create_private_dir_all(&self.attachments_dir())
    }
}

/// File name under `attachments/` for the `index`-th attachment on `message` (D75):
/// `<sent_at>-<sender8>-<index>-<safe name>`.
pub fn attachment_file_name(
    message: MessageRef,
    index: usize,
    file_name: Option<&str>,
    content_type: &str,
) -> String {
    let sent_at = message.sent_at.as_millis();
    let mut sender = message.sender.uuid().simple().to_string();
    sender.truncate(8);
    let safe = sanitize_file_name(file_name, content_type);
    format!("{sent_at}-{sender}-{index}-{safe}")
}

/// Reduce an untrusted attachment file name to safe ASCII characters, falling
/// back to `attachment.<ext>` from `content_type` when nothing usable remains.
pub fn sanitize_file_name(file_name: Option<&str>, content_type: &str) -> String {
    let cleaned = file_name
        .map(|raw| raw.rsplit(['/', '\\']).next().unwrap_or(raw))
        .map(|base| {
            base.chars()
                .map(|c| {
                    if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                        c
                    } else {
                        '_'
                    }
                })
                .collect::<String>()
        })
        .map(|s| {
            s.trim_start_matches(['.', '_'])
                .chars()
                .take(64)
                .collect::<String>()
        })
        .filter(|s| !s.is_empty());
    cleaned.unwrap_or_else(|| format!("attachment.{}", extension_for(content_type)))
}

/// File extension (without leading dot) for a MIME type.
pub fn extension_for(content_type: &str) -> &'static str {
    match content_type.to_ascii_lowercase().as_str() {
        "image/jpeg" | "image/jpg" => "jpg",
        "image/png" => "png",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "video/mp4" => "mp4",
        "audio/aac" | "audio/mp4" => "m4a",
        "audio/ogg" => "ogg",
        "audio/mpeg" => "mp3",
        "audio/wav" => "wav",
        "application/pdf" => "pdf",
        "text/plain" => "txt",
        "text/csv" => "csv",
        _ => "bin",
    }
}

/// MIME type guessed from a file path's extension (D77).
pub fn content_type_for(path: &Path) -> &'static str {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase);
    match ext.as_deref() {
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("png") => "image/png",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("mp4") => "video/mp4",
        Some("m4a" | "aac") => "audio/aac",
        Some("ogg" | "oga" | "opus") => "audio/ogg",
        Some("mp3") => "audio/mpeg",
        Some("wav") => "audio/wav",
        Some("pdf") => "application/pdf",
        Some("txt" | "md" | "log" | "rs" | "toml" | "json") => "text/plain",
        Some("csv") => "text/csv",
        Some("zip") => "application/zip",
        _ => "application/octet-stream",
    }
}

/// Like `fs::create_dir_all`, but every directory it creates is `0700`.
/// Existing directories are left untouched (they may be user-chosen, e.g. via `--data-dir`).
///
/// Blocking: call at startup or from a blocking task, never from the UI loop.
#[expect(clippy::disallowed_methods, reason = "startup-only filesystem setup")]
pub fn create_private_dir_all(path: &Path) -> io::Result<()> {
    DirBuilder::new()
        .recursive(true)
        .mode(DIR_MODE)
        .create(path)
}

/// Open `path` for appending, creating it as `0600`. An existing file is tightened to `0600`.
///
/// Blocking: call at startup or from a blocking task, never from the UI loop.
#[expect(clippy::disallowed_methods, reason = "startup-only filesystem setup")]
pub fn open_private_append(path: &Path) -> io::Result<File> {
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .mode(FILE_MODE)
        .open(path)?;
    let perms = file.metadata()?.permissions();
    if perms.mode() & 0o777 != FILE_MODE {
        fs::set_permissions(path, fs::Permissions::from_mode(FILE_MODE))?;
    }
    Ok(file)
}

/// Write `bytes` to `path` (`0600`, parent created as `0700`) via a temporary
/// sibling file and a rename, so a half-written file is never seen (D75).
///
/// Blocking: call from `spawn_blocking`, never from the UI loop.
#[expect(clippy::disallowed_methods, reason = "called from spawn_blocking")]
pub fn write_private_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        create_private_dir_all(parent)?;
    }
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("attachment");
    let tmp = path.with_file_name(format!(".{name}.tmp-{:016x}", fastrand::u64(..)));
    let write = || -> io::Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(FILE_MODE)
            .open(&tmp)?;
        let perms = file.metadata()?.permissions();
        if perms.mode() & 0o777 != FILE_MODE {
            fs::set_permissions(&tmp, fs::Permissions::from_mode(FILE_MODE))?;
        }
        file.write_all(bytes)?;
        file.flush()?;
        fs::rename(&tmp, path)
    };
    let result = write();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

#[cfg(test)]
#[expect(
    clippy::disallowed_methods,
    reason = "tests set up files synchronously"
)]
mod tests {
    use uuid::Uuid;

    use super::*;
    use crate::domain::{ServiceId, Timestamp};

    fn mode(path: &Path) -> u32 {
        fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    #[test]
    fn defaults_follow_xdg_layout() {
        let paths = Paths::from_bases(
            Path::new("/home/u/.config"),
            Path::new("/home/u/.local/share"),
            None,
            None,
        );
        assert_eq!(
            paths.config_file(),
            Path::new("/home/u/.config/faro/config.toml")
        );
        assert_eq!(paths.data_dir(), Path::new("/home/u/.local/share/faro"));
        assert_eq!(paths.log_dir(), Path::new("/home/u/.local/share/faro/logs"));
        assert_eq!(
            paths.attachments_dir(),
            Path::new("/home/u/.local/share/faro/attachments")
        );
        assert_eq!(
            paths.db_file(),
            Path::new("/home/u/.local/share/faro/faro.db")
        );
    }

    #[test]
    fn overrides_win() {
        let paths = Paths::from_bases(
            Path::new("/home/u/.config"),
            Path::new("/home/u/.local/share"),
            Some("/tmp/c.toml".into()),
            Some("/tmp/data".into()),
        );
        assert_eq!(paths.config_file(), Path::new("/tmp/c.toml"));
        assert_eq!(paths.data_dir(), Path::new("/tmp/data"));
    }

    #[test]
    fn creates_private_dirs_and_files() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::from_bases(tmp.path(), tmp.path(), None, None);
        paths.ensure_data_dirs().unwrap();
        assert_eq!(mode(paths.data_dir()), DIR_MODE);
        assert_eq!(mode(&paths.log_dir()), DIR_MODE);
        assert_eq!(mode(&paths.attachments_dir()), DIR_MODE);

        let log = paths.log_dir().join("faro.log");
        open_private_append(&log).unwrap();
        assert_eq!(mode(&log), FILE_MODE);

        let att = paths.attachments_dir().join("1-00000000-0-photo.jpg");
        write_private_atomic(&att, b"jpeg-bytes").unwrap();
        assert_eq!(mode(&att), FILE_MODE);
        assert_eq!(fs::read(&att).unwrap(), b"jpeg-bytes");
    }

    #[test]
    fn attachment_names_are_sanitized() {
        let msg = MessageRef {
            sender: ServiceId::Aci(Uuid::from_u128(0xabcd_1234)),
            sent_at: Timestamp::from_millis(1_700_000),
        };
        assert_eq!(
            attachment_file_name(msg, 0, Some("../../etc/passwd"), "text/plain"),
            "1700000-00000000-0-passwd"
        );
        assert_eq!(
            attachment_file_name(msg, 2, Some("..hidden file?.png"), "image/png"),
            "1700000-00000000-2-hidden_file_.png"
        );
        assert_eq!(
            attachment_file_name(msg, 1, None, "image/webp"),
            "1700000-00000000-1-attachment.webp"
        );
        assert_eq!(content_type_for(Path::new("/tmp/cat.JPG")), "image/jpeg");
        assert_eq!(
            content_type_for(Path::new("/tmp/unknown")),
            "application/octet-stream"
        );
    }

    #[test]
    fn tightens_existing_file() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("loose.log");
        fs::write(&file, "x").unwrap();
        fs::set_permissions(&file, fs::Permissions::from_mode(0o644)).unwrap();
        open_private_append(&file).unwrap();
        assert_eq!(mode(&file), FILE_MODE);
    }

    #[test]
    fn leaves_existing_dirs_alone() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("existing");
        fs::create_dir(&dir).unwrap();
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();
        create_private_dir_all(&dir).unwrap();
        assert_eq!(mode(&dir), 0o755);
    }
}
