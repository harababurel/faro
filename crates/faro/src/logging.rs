//! Logging to a file in the data dir. Never to stdout/stderr: the TUI owns the terminal.
//!
//! Privacy (DESIGN.md §12): never log message bodies or phone numbers by default.
//!
//! Filter with `FARO_LOG` (same syntax as `RUST_LOG`), e.g. `FARO_LOG=faro=debug`.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Context as _;
use faro_core::paths;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::EnvFilter;

const LOG_FILE: &str = "faro.log";
const FILTER_ENV: &str = "FARO_LOG";
const DEFAULT_FILTER: &str = "info";
/// At startup, a log bigger than this is moved to `faro.log.old` (replacing any older one).
const MAX_LOG_BYTES: u64 = 10 * 1024 * 1024;

/// Start logging to `<log_dir>/faro.log`. Keep the returned guard alive until exit
/// so buffered lines are flushed.
pub fn init(log_dir: &Path) -> anyhow::Result<(WorkerGuard, PathBuf)> {
    let path = log_dir.join(LOG_FILE);
    rotate_if_large(&path, MAX_LOG_BYTES)
        .with_context(|| format!("rotating log file {}", path.display()))?;
    let file = paths::open_private_append(&path)
        .with_context(|| format!("opening log file {}", path.display()))?;
    let (writer, guard) = tracing_appender::non_blocking(file);

    let filter =
        EnvFilter::try_from_env(FILTER_ENV).unwrap_or_else(|_| EnvFilter::new(DEFAULT_FILTER));

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(writer)
        .with_ansi(false)
        .try_init()
        .map_err(|e| anyhow::anyhow!("installing log subscriber: {e}"))?;

    Ok((guard, path))
}

fn rotate_if_large(path: &Path, max_bytes: u64) -> std::io::Result<()> {
    match fs::metadata(path) {
        Ok(meta) if meta.len() > max_bytes => fs::rename(path, path.with_extension("log.old")),
        Ok(_) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotates_only_when_too_big() {
        let tmp = tempfile::tempdir().unwrap();
        let log = tmp.path().join(LOG_FILE);
        let old = tmp.path().join("faro.log.old");

        rotate_if_large(&log, 4).unwrap(); // missing file is fine

        fs::write(&log, "1234").unwrap();
        rotate_if_large(&log, 4).unwrap();
        assert!(log.exists() && !old.exists());

        fs::write(&log, "12345").unwrap();
        rotate_if_large(&log, 4).unwrap();
        assert!(!log.exists() && old.exists());
    }
}
