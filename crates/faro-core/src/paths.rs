//! Where faro keeps its config and data (DECISIONS D17).
//!
//! XDG layout on every Unix, including macOS:
//! - config: `$XDG_CONFIG_HOME/faro/config.toml` (default `~/.config/faro/config.toml`)
//! - data:   `$XDG_DATA_HOME/faro/` (default `~/.local/share/faro/`)
//!
//! `faro demo` keeps its data apart, in `<temp>/faro-demo-<uid>/` by default
//! ([`default_demo_dir`], [`prepare_demo_dir`]), and shares only the config.
//!
//! Directories faro creates are `0700`, files are `0600`.

use std::fs::{self, DirBuilder, File, OpenOptions};
use std::io::{self, Write as _};
use std::os::unix::fs::{DirBuilderExt, MetadataExt as _, OpenOptionsExt, PermissionsExt};
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

    /// The same config file with the data in `dir`: the layout `faro demo`
    /// uses inside its own directory (D87). Check `dir` with
    /// [`prepare_demo_dir`] first.
    #[must_use]
    pub fn demo(self, dir: PathBuf) -> Self {
        Self {
            config_file: self.config_file,
            data_dir: dir,
        }
    }
}

/// The file that marks a directory as made by `faro demo` (D87). The demo
/// only starts in, and `--fresh` only ever deletes, a directory that has it.
pub const DEMO_MARKER: &str = ".faro-demo";

/// presage's database, holding a linked Signal account (its name is fixed by
/// `faro-presage`). A directory with one is never a demo directory.
const PRESAGE_DB: &str = "presage.db";

/// What the marker file says, for anyone who finds it.
const MARKER_TEXT: &str = "Made by `faro demo`: fake data only. Safe to delete.\n";

/// Where `faro demo` keeps its data unless told otherwise:
/// `<temp>/faro-demo-<uid>/` (D87). The uid keeps users of a shared machine
/// apart; the system clears the temp directory in its own time.
pub fn default_demo_dir(temp_dir: &Path, uid: u32) -> PathBuf {
    temp_dir.join(format!("faro-demo-{uid}"))
}

/// The real user ID of this process.
pub fn current_uid() -> u32 {
    rustix::process::getuid().as_raw()
}

/// faro's own database file name, in a demo directory as anywhere.
const FARO_DB: &str = "faro.db";

/// Where a real run's `dir` meets a demo directory (D87): `dir` itself or
/// the nearest ancestor that carries the [`DEMO_MARKER`], checked on the path
/// as given and after resolving symlinks. Real runs, `faro link` and `faro
/// ids` refuse to keep data there, so `--fresh` can never wipe it.
///
/// Blocking: call at startup, before the UI.
pub fn demo_dir_around(dir: &Path) -> io::Result<Option<PathBuf>> {
    for path in [absolute(dir)?, resolve(dir)?] {
        for ancestor in path.ancestors() {
            if matches!(symlink_metadata(&ancestor.join(DEMO_MARKER)), Ok(Some(_))) {
                return Ok(Some(ancestor.to_owned()));
            }
        }
    }
    Ok(None)
}

/// Whether `path` exists (following symlinks).
///
/// Blocking: call at startup or from a blocking task, never from the UI loop.
#[expect(clippy::disallowed_methods, reason = "startup, or a blocking task")]
pub fn exists(path: &Path) -> io::Result<bool> {
    path.try_exists()
}

/// Why a directory can't hold the demo's data (D87).
#[derive(Debug, thiserror::Error)]
pub enum DemoDirError {
    #[error(
        "{dir} overlaps faro's data directory {data_dir}; the demo keeps its data apart \
         (pick another with --dir)"
    )]
    OverlapsRealData { dir: PathBuf, data_dir: PathBuf },
    #[error("{0} is a symbolic link; the demo only uses files and directories it made itself")]
    Symlink(PathBuf),
    #[error("{0} is not a directory")]
    NotADirectory(PathBuf),
    #[error("{0} is not a regular file; the demo only uses files it made itself")]
    NotAFile(PathBuf),
    #[error("{dir} belongs to another user (uid {owner}); the demo only uses its own directory")]
    NotOwned { dir: PathBuf, owner: u32 },
    #[error("{dir} has mode {mode:04o}, not 0700; the demo only uses a private directory")]
    NotPrivate { dir: PathBuf, mode: u32 },
    #[error("{0} holds a linked Signal account (presage.db); the demo never uses it")]
    SignalAccount(PathBuf),
    #[error(
        "{0} isn't empty and isn't a demo directory (no {DEMO_MARKER} file); \
         pick another with --dir"
    )]
    NotMarked(PathBuf),
    #[error("--fresh won't delete {dir}: {found} {why}")]
    FreshRefused {
        dir: PathBuf,
        found: PathBuf,
        why: &'static str,
    },
    #[error("preparing the demo directory {dir}: {source}")]
    Io { dir: PathBuf, source: io::Error },
}

/// What [`prepare_demo_dir`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DemoDir {
    /// Made a new demo directory (or claimed an empty one).
    Created,
    /// Kept the demo world from an earlier run.
    Reused,
    /// Deleted the earlier demo world and started over (`--fresh`).
    Wiped,
}

/// Make `dir` ready for the demo's data, or say why it can't be (D87).
///
/// Each check stands on its own, so a wrong path never reaches real data:
/// - `dir` must not be, contain, or sit inside any of `real_data_dirs`
///   (compared after resolving symlinks).
/// - A missing `dir` is created `0700` with the [`DEMO_MARKER`] file.
/// - An existing `dir` must be a real directory (not a symlink) owned by
///   `uid`, must not hold `presage.db` or a symlink, its `faro.db` must be a
///   regular file, and it must either be marked and `0700` or be empty (it's
///   then tightened to `0700` and marked).
/// - `fresh` deletes `dir` and starts over only if it passed every check,
///   was already marked, holds no `presage.db` and no `faro.db` anywhere
///   below the top (walked without following symlinks), and its own
///   `faro.db`, if any, passes `is_demo_db` (marked `demo`). Never an
///   arbitrary directory.
///
/// Blocking: call at startup, before the UI.
pub fn prepare_demo_dir(
    dir: &Path,
    fresh: bool,
    real_data_dirs: &[PathBuf],
    uid: u32,
    is_demo_db: impl FnOnce(&Path) -> bool,
) -> Result<DemoDir, DemoDirError> {
    let io = |source| DemoDirError::Io {
        dir: dir.to_owned(),
        source,
    };
    let resolved = resolve(dir).map_err(io)?;
    for real in real_data_dirs {
        let real_resolved = resolve(real).map_err(io)?;
        if resolved.starts_with(&real_resolved) || real_resolved.starts_with(&resolved) {
            return Err(DemoDirError::OverlapsRealData {
                dir: dir.to_owned(),
                data_dir: real.clone(),
            });
        }
    }
    let Some(metadata) = symlink_metadata(dir).map_err(io)? else {
        create_demo_dir(dir).map_err(io)?;
        return Ok(DemoDir::Created);
    };
    let file_type = metadata.file_type();
    if file_type.is_symlink() {
        return Err(DemoDirError::Symlink(dir.to_owned()));
    }
    if !file_type.is_dir() {
        return Err(DemoDirError::NotADirectory(dir.to_owned()));
    }
    if metadata.uid() != uid {
        return Err(DemoDirError::NotOwned {
            dir: dir.to_owned(),
            owner: metadata.uid(),
        });
    }
    let entries = entries(dir).map_err(io)?;
    if entries.iter().any(|(name, _)| name == PRESAGE_DB) {
        return Err(DemoDirError::SignalAccount(dir.to_owned()));
    }
    for (name, kind) in &entries {
        if kind.is_symlink() {
            return Err(DemoDirError::Symlink(dir.join(name)));
        }
        if name == FARO_DB && !kind.is_file() {
            return Err(DemoDirError::NotAFile(dir.join(name)));
        }
    }
    let marked = entries
        .iter()
        .any(|(name, kind)| name == DEMO_MARKER && kind.is_file());
    let mode = metadata.permissions().mode() & 0o777;
    if marked {
        if mode != DIR_MODE {
            return Err(DemoDirError::NotPrivate {
                dir: dir.to_owned(),
                mode,
            });
        }
        if fresh {
            check_wipeable(dir, &entries, is_demo_db)?;
            wipe_demo_dir(dir).map_err(io)?;
            create_demo_dir(dir).map_err(io)?;
            return Ok(DemoDir::Wiped);
        }
        return Ok(DemoDir::Reused);
    }
    if !entries.is_empty() {
        return Err(DemoDirError::NotMarked(dir.to_owned()));
    }
    // Empty and ours (e.g. from `mktemp -d` or a script): claim it.
    claim_empty_dir(dir).map_err(io)?;
    Ok(DemoDir::Created)
}

/// Refuse to wipe a marked demo directory that holds anything a real run
/// could have left there: a `presage.db` or a `faro.db` anywhere below the
/// top, or a top-level `faro.db` not marked `demo` (D87).
fn check_wipeable(
    dir: &Path,
    top: &[(std::ffi::OsString, fs::FileType)],
    is_demo_db: impl FnOnce(&Path) -> bool,
) -> Result<(), DemoDirError> {
    let refuse = |found: PathBuf, why| DemoDirError::FreshRefused {
        dir: dir.to_owned(),
        found,
        why,
    };
    // Below the top, without following symlinks.
    let mut pending: Vec<PathBuf> = top
        .iter()
        .filter(|(_, kind)| kind.is_dir())
        .map(|(name, _)| dir.join(name))
        .collect();
    while let Some(sub) = pending.pop() {
        let found = entries(&sub).map_err(|source| DemoDirError::Io {
            dir: dir.to_owned(),
            source,
        })?;
        for (name, kind) in found {
            let path = sub.join(&name);
            if name == PRESAGE_DB {
                return Err(refuse(path, "is a Signal account's database"));
            }
            if name == FARO_DB {
                return Err(refuse(path, "is another faro database"));
            }
            if kind.is_dir() {
                pending.push(path);
            }
        }
    }
    let db = dir.join(FARO_DB);
    if top.iter().any(|(name, _)| name == FARO_DB) && !is_demo_db(&db) {
        return Err(refuse(db, "isn't marked as a demo database"));
    }
    Ok(())
}

/// `path` made absolute (against the current directory), as written.
fn absolute(path: &Path) -> io::Result<PathBuf> {
    Ok(if path.is_absolute() {
        path.to_owned()
    } else {
        std::env::current_dir()?.join(path)
    })
}

/// `path` made absolute with symlinks resolved, as far as it exists; the
/// missing rest is appended as written.
#[expect(clippy::disallowed_methods, reason = "startup, before the UI")]
fn resolve(path: &Path) -> io::Result<PathBuf> {
    let absolute = absolute(path)?;
    let mut existing = absolute.as_path();
    let mut missing = Vec::new();
    loop {
        match fs::canonicalize(existing) {
            Ok(mut real) => {
                real.extend(missing.iter().rev());
                return Ok(real);
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                match (existing.file_name(), existing.parent()) {
                    (Some(name), Some(parent)) => {
                        missing.push(name.to_owned());
                        existing = parent;
                    }
                    // `..` or the root: compare as written.
                    _ => return Ok(absolute),
                }
            }
            Err(error) => return Err(error),
        }
    }
}

/// `path`'s own metadata (a symlink is not followed), `None` if it doesn't exist.
#[expect(clippy::disallowed_methods, reason = "startup, before the UI")]
fn symlink_metadata(path: &Path) -> io::Result<Option<fs::Metadata>> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => Ok(Some(metadata)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

/// The names in directory `dir` and what each is (a symlink is reported as
/// one, not followed).
#[expect(clippy::disallowed_methods, reason = "startup, before the UI")]
fn entries(dir: &Path) -> io::Result<Vec<(std::ffi::OsString, fs::FileType)>> {
    fs::read_dir(dir)?
        .map(|entry| entry.and_then(|e| Ok((e.file_name(), e.file_type()?))))
        .collect()
}

/// Create the demo directory itself `0700` (its parents too, if missing)
/// and mark it. Fails if something appeared there in the meantime.
#[expect(clippy::disallowed_methods, reason = "startup, before the UI")]
fn create_demo_dir(dir: &Path) -> io::Result<()> {
    if let Some(parent) = dir.parent().filter(|p| !p.as_os_str().is_empty()) {
        create_private_dir_all(parent)?;
    }
    DirBuilder::new().mode(DIR_MODE).create(dir)?;
    // The umask can only take bits away; make sure it's exactly 0700.
    fs::set_permissions(dir, fs::Permissions::from_mode(DIR_MODE))?;
    write_marker(dir)
}

/// Tighten an empty directory we own to `0700` and mark it.
#[expect(clippy::disallowed_methods, reason = "startup, before the UI")]
fn claim_empty_dir(dir: &Path) -> io::Result<()> {
    fs::set_permissions(dir, fs::Permissions::from_mode(DIR_MODE))?;
    write_marker(dir)
}

#[expect(clippy::disallowed_methods, reason = "startup, before the UI")]
fn write_marker(dir: &Path) -> io::Result<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(FILE_MODE)
        .open(dir.join(DEMO_MARKER))?;
    file.write_all(MARKER_TEXT.as_bytes())
}

/// Delete a demo directory that [`prepare_demo_dir`] has checked and found
/// marked. `remove_dir_all` doesn't follow symlinks inside it.
#[expect(clippy::disallowed_methods, reason = "startup, before the UI")]
fn wipe_demo_dir(dir: &Path) -> io::Result<()> {
    fs::remove_dir_all(dir)
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

/// Resolve the user's Downloads directory (`$XDG_DOWNLOAD_DIR` when non-empty,
/// else `~/Downloads`).
pub fn resolve_downloads_dir() -> Result<PathBuf, PathsError> {
    let xdg = etcetera::base_strategy::Xdg::new().map_err(|_| PathsError::NoHomeDir)?;
    downloads_dir(
        std::env::var_os("XDG_DOWNLOAD_DIR").as_deref(),
        Some(xdg.home_dir()),
    )
}

/// Pure helper for [`resolve_downloads_dir`].
pub fn downloads_dir(
    xdg_download_dir: Option<&std::ffi::OsStr>,
    home_dir: Option<&Path>,
) -> Result<PathBuf, PathsError> {
    if let Some(dir) = xdg_download_dir.filter(|d| !d.is_empty()) {
        return Ok(PathBuf::from(dir));
    }
    let home = home_dir.ok_or(PathsError::NoHomeDir)?;
    Ok(home.join("Downloads"))
}

/// Copy `source` into `dir` under a sanitized name derived from `file_name` /
/// `content_type`, numbering collisions as `name (1).ext`, `name (2).ext`, …
/// so an existing file is never overwritten (D75).
///
/// Blocking: call from `spawn_blocking`, never from the UI loop.
#[expect(clippy::disallowed_methods, reason = "called from spawn_blocking")]
pub fn save_to_downloads(
    source: &Path,
    dir: &Path,
    file_name: Option<&str>,
    content_type: &str,
) -> io::Result<PathBuf> {
    fs::create_dir_all(dir)?;
    let bytes = fs::read(source)?;
    let safe = sanitize_file_name(file_name, content_type);
    let split = safe
        .rsplit_once('.')
        .filter(|(stem, ext)| !stem.is_empty() && !ext.is_empty());
    for attempt in 0..10_000usize {
        let candidate = if attempt == 0 {
            safe.clone()
        } else if let Some((stem, ext)) = split {
            format!("{stem} ({attempt}).{ext}")
        } else {
            format!("{safe} ({attempt})")
        };
        let dest = dir.join(&candidate);
        match OpenOptions::new().write(true).create_new(true).open(&dest) {
            Ok(mut file) => {
                file.write_all(&bytes)?;
                file.flush()?;
                return Ok(dest);
            }
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {}
            Err(err) => return Err(err),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "too many duplicate files in Downloads",
    ))
}

#[cfg(test)]
#[expect(
    clippy::disallowed_methods,
    reason = "tests set up files synchronously"
)]
mod tests {
    use std::ffi::OsStr;

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
    fn downloads_dir_and_collision_numbering() {
        assert_eq!(
            downloads_dir(Some(OsStr::new("/custom/dl")), Some(Path::new("/home/u"))).unwrap(),
            Path::new("/custom/dl")
        );
        assert_eq!(
            downloads_dir(Some(OsStr::new("")), Some(Path::new("/home/u"))).unwrap(),
            Path::new("/home/u/Downloads")
        );
        assert_eq!(
            downloads_dir(None, Some(Path::new("/home/u"))).unwrap(),
            Path::new("/home/u/Downloads")
        );

        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src.bin");
        fs::write(&src, b"payload").unwrap();
        let dl = tmp.path().join("Downloads");

        let first = save_to_downloads(&src, &dl, Some("crag.jpg"), "image/jpeg").unwrap();
        assert_eq!(first, dl.join("crag.jpg"));
        assert_eq!(fs::read(&first).unwrap(), b"payload");

        let second = save_to_downloads(&src, &dl, Some("crag.jpg"), "image/jpeg").unwrap();
        assert_eq!(second, dl.join("crag (1).jpg"));

        let third = save_to_downloads(&src, &dl, Some("crag.jpg"), "image/jpeg").unwrap();
        assert_eq!(third, dl.join("crag (2).jpg"));

        let no_ext = save_to_downloads(&src, &dl, Some("README"), "text/plain").unwrap();
        assert_eq!(no_ext, dl.join("README"));
        let no_ext_dup = save_to_downloads(&src, &dl, Some("README"), "text/plain").unwrap();
        assert_eq!(no_ext_dup, dl.join("README (1)"));
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

    /// A fake real data dir next to the demo dirs under test.
    fn real_dir(tmp: &Path) -> Vec<PathBuf> {
        let real = tmp.join("real");
        fs::create_dir(&real).unwrap();
        fs::write(real.join("faro.db"), "history").unwrap();
        vec![real]
    }

    /// Every top-level `faro.db` is a demo one (the store decides in faro).
    fn demo_db(_: &Path) -> bool {
        true
    }

    fn marked(dir: &Path) -> bool {
        fs::symlink_metadata(dir.join(DEMO_MARKER)).is_ok_and(|m| m.is_file())
    }

    #[test]
    fn demo_paths_keep_the_config() {
        let paths = Paths::from_bases(
            Path::new("/home/u/.config"),
            Path::new("/home/u/.local/share"),
            None,
            None,
        )
        .demo("/tmp/faro-demo-1000".into());
        assert_eq!(
            paths.config_file(),
            Path::new("/home/u/.config/faro/config.toml")
        );
        assert_eq!(paths.db_file(), Path::new("/tmp/faro-demo-1000/faro.db"));
        assert_eq!(paths.log_dir(), Path::new("/tmp/faro-demo-1000/logs"));
        assert_eq!(
            default_demo_dir(Path::new("/tmp"), 1000),
            Path::new("/tmp/faro-demo-1000")
        );
    }

    #[test]
    fn demo_dir_is_created_private_and_marked_then_reused() {
        let tmp = tempfile::tempdir().unwrap();
        let real = real_dir(tmp.path());
        let dir = tmp.path().join("nested").join("demo");
        let uid = current_uid();
        assert_eq!(demo_dir_around(&dir).unwrap(), None);
        assert_eq!(demo_dir_around(&real[0]).unwrap(), None);
        assert_eq!(
            prepare_demo_dir(&dir, false, &real, uid, demo_db).unwrap(),
            DemoDir::Created
        );
        assert_eq!(demo_dir_around(&dir).unwrap(), Some(dir.clone()));
        assert_eq!(mode(&dir), DIR_MODE);
        assert!(marked(&dir));
        assert_eq!(mode(&dir.join(DEMO_MARKER)), FILE_MODE);
        fs::write(dir.join("faro.db"), "demo").unwrap();
        assert_eq!(
            prepare_demo_dir(&dir, false, &real, uid, demo_db).unwrap(),
            DemoDir::Reused
        );
        assert!(dir.join("faro.db").exists(), "reused, not wiped");
    }

    #[test]
    fn demo_dir_claims_an_empty_dir_but_not_a_full_one() {
        let tmp = tempfile::tempdir().unwrap();
        let real = real_dir(tmp.path());
        let uid = current_uid();
        let empty = tmp.path().join("empty");
        fs::create_dir(&empty).unwrap();
        fs::set_permissions(&empty, fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(
            prepare_demo_dir(&empty, false, &real, uid, demo_db).unwrap(),
            DemoDir::Created
        );
        assert_eq!(mode(&empty), DIR_MODE, "tightened");
        assert!(marked(&empty));

        let full = tmp.path().join("full");
        fs::create_dir(&full).unwrap();
        fs::write(full.join("notes.txt"), "mine").unwrap();
        for fresh in [false, true] {
            assert!(matches!(
                prepare_demo_dir(&full, fresh, &real, uid, demo_db),
                Err(DemoDirError::NotMarked(_))
            ));
        }
        assert!(full.join("notes.txt").exists(), "never deleted");
        assert!(!marked(&full));
    }

    #[test]
    fn demo_dir_refuses_a_signal_account_even_when_marked() {
        let tmp = tempfile::tempdir().unwrap();
        let real = real_dir(tmp.path());
        let uid = current_uid();
        let dir = tmp.path().join("demo");
        prepare_demo_dir(&dir, false, &real, uid, demo_db).unwrap();
        fs::write(dir.join("presage.db"), "keys").unwrap();
        for fresh in [false, true] {
            assert!(matches!(
                prepare_demo_dir(&dir, fresh, &real, uid, demo_db),
                Err(DemoDirError::SignalAccount(_))
            ));
        }
        assert!(dir.join("presage.db").exists());
    }

    #[test]
    fn demo_dir_refuses_symlinks_files_others_and_loose_modes() {
        let tmp = tempfile::tempdir().unwrap();
        let real = real_dir(tmp.path());
        let uid = current_uid();
        let dir = tmp.path().join("demo");
        prepare_demo_dir(&dir, false, &real, uid, demo_db).unwrap();

        let link = tmp.path().join("link");
        std::os::unix::fs::symlink(&dir, &link).unwrap();
        assert!(matches!(
            prepare_demo_dir(&link, true, &real, uid, demo_db),
            Err(DemoDirError::Symlink(_))
        ));
        assert!(marked(&dir), "the link's target is left alone");

        let file = tmp.path().join("file");
        fs::write(&file, "x").unwrap();
        assert!(matches!(
            prepare_demo_dir(&file, false, &real, uid, demo_db),
            Err(DemoDirError::NotADirectory(_))
        ));

        assert!(matches!(
            prepare_demo_dir(&dir, true, &real, uid.wrapping_add(1), demo_db),
            Err(DemoDirError::NotOwned { .. })
        ));
        assert!(marked(&dir), "not wiped");

        fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(matches!(
            prepare_demo_dir(&dir, true, &real, uid, demo_db),
            Err(DemoDirError::NotPrivate { mode: 0o755, .. })
        ));
        assert!(marked(&dir), "not wiped");
    }

    #[test]
    fn demo_dir_never_overlaps_the_real_data_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let real = real_dir(tmp.path());
        let uid = current_uid();
        let inside = real[0].join("demo");
        let link = tmp.path().join("to-real");
        std::os::unix::fs::symlink(&real[0], &link).unwrap();
        for dir in [
            real[0].clone(),
            inside.clone(),
            link.join("demo"),
            tmp.path().to_owned(),
        ] {
            assert!(
                matches!(
                    prepare_demo_dir(&dir, true, &real, uid, demo_db),
                    Err(DemoDirError::OverlapsRealData { .. })
                ),
                "{}",
                dir.display()
            );
        }
        assert!(!inside.exists());
        assert!(real[0].join("faro.db").exists());
        assert!(!marked(&real[0]));
        // Marking it by hand doesn't help.
        fs::write(real[0].join(DEMO_MARKER), "").unwrap();
        assert!(matches!(
            prepare_demo_dir(&real[0], true, &real, uid, demo_db),
            Err(DemoDirError::OverlapsRealData { .. })
        ));
        assert!(real[0].join("faro.db").exists());
    }

    #[test]
    fn fresh_wipes_only_a_marked_demo_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let real = real_dir(tmp.path());
        let uid = current_uid();
        let dir = tmp.path().join("demo");
        assert_eq!(
            prepare_demo_dir(&dir, true, &real, uid, demo_db).unwrap(),
            DemoDir::Created
        );
        fs::create_dir(dir.join("logs")).unwrap();
        fs::write(dir.join("faro.db"), "demo").unwrap();
        assert_eq!(
            prepare_demo_dir(&dir, true, &real, uid, demo_db).unwrap(),
            DemoDir::Wiped
        );
        assert!(!dir.join("faro.db").exists());
        assert!(!dir.join("logs").exists());
        assert!(marked(&dir));
        assert_eq!(mode(&dir), DIR_MODE);
    }

    #[test]
    fn demo_dir_refuses_symlinks_and_odd_files_at_the_top() {
        let tmp = tempfile::tempdir().unwrap();
        let real = real_dir(tmp.path());
        let uid = current_uid();
        let dir = tmp.path().join("demo");
        prepare_demo_dir(&dir, false, &real, uid, demo_db).unwrap();

        fs::create_dir(dir.join("faro.db")).unwrap();
        assert!(matches!(
            prepare_demo_dir(&dir, false, &real, uid, demo_db),
            Err(DemoDirError::NotAFile(_))
        ));
        fs::remove_dir(dir.join("faro.db")).unwrap();

        // A database or log directory that leads to real data.
        for name in ["faro.db", "logs"] {
            let target = real[0].join(if name == "logs" { "." } else { "faro.db" });
            std::os::unix::fs::symlink(&target, dir.join(name)).unwrap();
            for fresh in [false, true] {
                assert!(matches!(
                    prepare_demo_dir(&dir, fresh, &real, uid, demo_db),
                    Err(DemoDirError::Symlink(_))
                ));
            }
            fs::remove_file(dir.join(name)).unwrap();
        }
        assert!(real[0].join("faro.db").exists());
    }

    #[test]
    fn fresh_refuses_other_data_below_the_top() {
        let tmp = tempfile::tempdir().unwrap();
        let real = real_dir(tmp.path());
        let uid = current_uid();
        let dir = tmp.path().join("demo");
        prepare_demo_dir(&dir, false, &real, uid, demo_db).unwrap();
        let nested = dir.join("acct").join("deeper");
        fs::create_dir_all(&nested).unwrap();
        for name in ["presage.db", "faro.db"] {
            let file = nested.join(name);
            fs::write(&file, "real").unwrap();
            assert!(
                matches!(
                    prepare_demo_dir(&dir, true, &real, uid, demo_db),
                    Err(DemoDirError::FreshRefused { ref found, .. }) if *found == file
                ),
                "{name}"
            );
            assert!(file.exists(), "{name} kept");
            // Without --fresh nothing is deleted, so it may carry on.
            assert_eq!(
                prepare_demo_dir(&dir, false, &real, uid, demo_db).unwrap(),
                DemoDir::Reused
            );
            fs::remove_file(&file).unwrap();
        }
        assert_eq!(
            prepare_demo_dir(&dir, true, &real, uid, demo_db).unwrap(),
            DemoDir::Wiped
        );
    }

    #[test]
    fn fresh_refuses_a_database_not_marked_demo() {
        let tmp = tempfile::tempdir().unwrap();
        let real = real_dir(tmp.path());
        let uid = current_uid();
        let dir = tmp.path().join("demo");
        prepare_demo_dir(&dir, false, &real, uid, demo_db).unwrap();
        let db = dir.join("faro.db");
        fs::write(&db, "history").unwrap();
        let mut asked = None;
        assert!(matches!(
            prepare_demo_dir(&dir, true, &real, uid, |path: &Path| {
                asked = Some(path.to_owned());
                false
            }),
            Err(DemoDirError::FreshRefused { .. })
        ));
        assert_eq!(asked, Some(db.clone()));
        assert!(db.exists());
        assert_eq!(
            prepare_demo_dir(&dir, true, &real, uid, demo_db).unwrap(),
            DemoDir::Wiped
        );
        assert!(!db.exists());
    }

    #[test]
    fn fresh_never_follows_symlinks_below_the_top() {
        let tmp = tempfile::tempdir().unwrap();
        let real = real_dir(tmp.path());
        let uid = current_uid();
        let elsewhere = tmp.path().join("elsewhere");
        fs::create_dir(&elsewhere).unwrap();
        fs::write(elsewhere.join("presage.db"), "keys").unwrap();
        let dir = tmp.path().join("demo");
        prepare_demo_dir(&dir, false, &real, uid, demo_db).unwrap();
        fs::create_dir(dir.join("logs")).unwrap();
        std::os::unix::fs::symlink(&elsewhere, dir.join("logs").join("away")).unwrap();
        assert_eq!(
            prepare_demo_dir(&dir, true, &real, uid, demo_db).unwrap(),
            DemoDir::Wiped
        );
        assert!(elsewhere.join("presage.db").exists());
    }

    #[test]
    fn a_data_dir_inside_a_demo_dir_is_found() {
        let tmp = tempfile::tempdir().unwrap();
        let real = real_dir(tmp.path());
        let uid = current_uid();
        let dir = tmp.path().join("demo");
        prepare_demo_dir(&dir, false, &real, uid, demo_db).unwrap();
        // Not there yet, and deeper down.
        assert_eq!(
            demo_dir_around(&dir.join("acct")).unwrap(),
            Some(dir.clone())
        );
        fs::create_dir_all(dir.join("acct").join("sub")).unwrap();
        assert_eq!(
            demo_dir_around(&dir.join("acct").join("sub")).unwrap(),
            Some(dir.clone())
        );
        // Through a symlink from outside.
        let link = tmp.path().join("link");
        std::os::unix::fs::symlink(dir.join("acct"), &link).unwrap();
        assert_eq!(
            demo_dir_around(&link).unwrap(),
            Some(fs::canonicalize(&dir).unwrap())
        );
        assert_eq!(demo_dir_around(&real[0]).unwrap(), None);
        assert_eq!(demo_dir_around(&tmp.path().join("other")).unwrap(), None);
    }
}
