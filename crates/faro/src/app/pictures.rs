//! Conversations' pictures on disk (D160, `faro_core::pictures`): reading
//! the folder at startup, saving what the backend fetched, and `faro
//! refresh-pictures` emptying it (with presage's own copies) so every
//! picture is fetched again.

use std::path::{Path, PathBuf};

use faro_core::backend::{Backend, BackendError};
use faro_core::domain::ConversationId;
use faro_core::{paths, pictures};
use tokio::sync::mpsc;

use super::event::AppEvent;

/// Read the folder in the background; answered with
/// `AppEvent::PicturesListed` (empty when it can't be read).
pub fn list(dir: PathBuf, results: mpsc::UnboundedSender<AppEvent>) {
    tokio::spawn(async move {
        let found = read(&dir).await.unwrap_or_else(|error| {
            if error.kind() != std::io::ErrorKind::NotFound {
                tracing::warn!(%error, "can't read the pictures folder");
            }
            Vec::new()
        });
        tracing::debug!(known = found.len(), "pictures folder read");
        let _ = results.send(AppEvent::PicturesListed(found));
    });
}

async fn read(dir: &Path) -> std::io::Result<Vec<(ConversationId, Option<PathBuf>)>> {
    let mut found = Vec::new();
    let mut entries = tokio::fs::read_dir(dir).await?;
    while let Some(entry) = entries.next_entry().await? {
        let name = entry.file_name();
        let Some((id, has_picture)) = name.to_str().and_then(pictures::parse) else {
            continue;
        };
        found.push((id, has_picture.then(|| entry.path())));
    }
    Ok(found)
}

/// Fetch `conversation`'s picture and save it (or the mark that it has
/// none) in `dir`, `0600`. The picture's path, or `None`.
pub async fn fetch<B: Backend>(
    backend: &B,
    dir: &Path,
    conversation: ConversationId,
) -> Result<Option<PathBuf>, BackendError> {
    let picture = backend
        .fetch_picture(conversation)
        .await?
        .filter(|bytes| !bytes.is_empty());
    let has_picture = picture.is_some();
    let path = dir.join(pictures::file_name(conversation, has_picture));
    let bytes = picture.unwrap_or_default();
    let written = path.clone();
    tokio::task::spawn_blocking(move || paths::write_private_atomic(&written, &bytes))
        .await
        .map_err(|e| BackendError::Storage(format!("picture writer task: {e}")))?
        .map_err(|e| BackendError::Storage(format!("saving a picture: {e}")))?;
    Ok(has_picture.then_some(path))
}

/// Delete every saved picture and mark in `dir`; how many went. Other
/// files are left alone.
pub async fn clear(dir: &Path) -> std::io::Result<usize> {
    let mut removed = 0;
    let mut entries = match tokio::fs::read_dir(dir).await {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(error),
    };
    while let Some(entry) = entries.next_entry().await? {
        let name = entry.file_name();
        if name.to_str().and_then(pictures::parse).is_some() {
            tokio::fs::remove_file(entry.path()).await?;
            removed += 1;
        }
    }
    Ok(removed)
}

#[cfg(test)]
#[expect(
    clippy::disallowed_methods,
    reason = "#[tokio::test] expands to block_on; tests may block"
)]
mod tests {
    use faro_core::backend::{BackendEvent, Capability};
    use faro_core::domain::ConnectionState;
    use faro_core::mock::{MockBackend, MockConfig};

    use super::*;

    #[tokio::test]
    async fn pictures_are_saved_listed_and_cleared() {
        let dir = tempfile::tempdir().unwrap();
        let (backend, mut events) = MockBackend::new(&MockConfig {
            activity: None,
            disconnects: false,
            errors: false,
            ..MockConfig::default()
        });
        assert!(backend.capabilities().supports(Capability::Pictures));
        let connected = BackendEvent::Connection(ConnectionState::Connected);
        tokio::time::timeout(std::time::Duration::from_secs(30), async {
            while events.recv().await.is_some_and(|event| event != connected) {}
        })
        .await
        .unwrap();
        let groups = backend.groups().await.unwrap();
        let group = ConversationId::Group(groups.first().unwrap().id);
        let path = fetch(&backend, dir.path(), group).await.unwrap().unwrap();
        let mode = std::fs::metadata(&path).unwrap();
        assert_eq!(
            std::os::unix::fs::PermissionsExt::mode(&mode.permissions()) & 0o777,
            0o600
        );
        let stray = dir.path().join("notes.txt");
        std::fs::write(&stray, "mine").unwrap();
        assert_eq!(read(dir.path()).await.unwrap(), vec![(group, Some(path))]);
        assert_eq!(clear(dir.path()).await.unwrap(), 1);
        assert_eq!(read(dir.path()).await.unwrap(), vec![]);
        assert!(stray.exists(), "other files are left alone");
    }
}
