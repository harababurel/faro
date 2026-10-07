//! Where faro keeps its config and data (DECISIONS D17).
//!
//! XDG layout on every Unix, including macOS:
//! - config: `$XDG_CONFIG_HOME/faro/config.toml` (default `~/.config/faro/config.toml`)
//! - data:   `$XDG_DATA_HOME/faro/` (default `~/.local/share/faro/`)
//!
//! Directories faro creates are `0700`, files are `0600`.

use std::fs::{self, DirBuilder, File, OpenOptions};
use std::io;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use etcetera::BaseStrategy;

const APP_DIR: &str = "faro";
const CONFIG_FILE: &str = "config.toml";
const LOG_DIR: &str = "logs";

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

    /// Create the data and log directories (`0700`) if they don't exist.
    pub fn ensure_data_dirs(&self) -> io::Result<()> {
        create_private_dir_all(&self.data_dir)?;
        create_private_dir_all(&self.log_dir())
    }
}

/// Like `fs::create_dir_all`, but every directory it creates is `0700`.
/// Existing directories are left untouched (they may be user-chosen, e.g. via `--data-dir`).
pub fn create_private_dir_all(path: &Path) -> io::Result<()> {
    DirBuilder::new()
        .recursive(true)
        .mode(DIR_MODE)
        .create(path)
}

/// Open `path` for appending, creating it as `0600`. An existing file is tightened to `0600`.
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

#[cfg(test)]
mod tests {
    use super::*;

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

        let log = paths.log_dir().join("faro.log");
        open_private_append(&log).unwrap();
        assert_eq!(mode(&log), FILE_MODE);
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
