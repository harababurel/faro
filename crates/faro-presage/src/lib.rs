//! faro's Signal backend on [presage] (D5, M7).
//!
//! - [`open_store`] opens presage's own database (`<data dir>/presage.db`).
//! - [`link`] links this computer as a new device of a Signal account, showing
//!   the provisioning URL (as a QR code) through a callback.
//! - [`load`] loads a linked account; [`start`] turns it into a
//!   [`faro_core::backend::Backend`] plus its event stream, receiving in a
//!   local task (presage's futures aren't `Send`, D30) that reconnects with
//!   backoff.
//!
//! presage sends nothing to other people on its own (D62): every message to
//! someone else goes through a [`Backend`](faro_core::backend::Backend) call,
//! which the allowlist guard checks.

mod backend;
mod convert;
mod link;

use std::path::Path;

use presage::Manager;
use presage::manager::Registered;
use presage::model::identity::OnNewIdentity;
use presage::store::StateStore as _;
use presage_store_sqlite::{SqliteConnectOptions, SqliteStore, SqliteStoreError};

pub use backend::{PresageBackend, start};
pub use link::{default_device_name, link};

/// A linked account, ready to start.
pub type Linked = Manager<SqliteStore, Registered>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("this computer isn't linked to a Signal account yet: run `faro link`")]
    NotLinked,
    #[error("presage's database: {0}")]
    Store(#[from] SqliteStoreError),
    #[error("making presage's database private: {0}")]
    Private(std::io::Error),
    #[error(transparent)]
    Signal(Box<presage::Error<SqliteStoreError>>),
}

impl From<presage::Error<SqliteStoreError>> for Error {
    fn from(error: presage::Error<SqliteStoreError>) -> Self {
        Self::Signal(Box::new(error))
    }
}

/// presage's database file inside faro's data directory.
pub fn store_file(data_dir: &Path) -> std::path::PathBuf {
    data_dir.join("presage.db")
}

/// Open (creating if needed) presage's database. New identity keys of
/// contacts are trusted, as in Signal's own apps (the safety number changes).
pub async fn open_store(file: &Path) -> Result<SqliteStore, Error> {
    make_private(file).await.map_err(Error::Private)?;
    let options = SqliteConnectOptions::new()
        .filename(file)
        .create_if_missing(true);
    Ok(SqliteStore::open_with_options(options, OnNewIdentity::Trust).await?)
}

/// Create the file `0600` (it holds the account's keys), or tighten it. SQLite
/// gives its `-wal` and `-shm` files the database's permissions.
async fn make_private(file: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    let mode = faro_core::paths::FILE_MODE;
    let handle = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .mode(mode)
        .open(file)
        .await?;
    if handle.metadata().await?.permissions().mode() & 0o777 != mode {
        tokio::fs::set_permissions(file, std::fs::Permissions::from_mode(mode)).await?;
    }
    Ok(())
}

pub async fn is_linked(store: &SqliteStore) -> bool {
    store.is_registered().await
}

/// Load the linked account, or [`Error::NotLinked`].
pub async fn load(store: SqliteStore) -> Result<Linked, Error> {
    if !is_linked(&store).await {
        return Err(Error::NotLinked);
    }
    Ok(Manager::load_registered(store).await?)
}

/// What `faro ids` prints.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ids {
    pub me: faro_core::domain::ServiceId,
    pub usernames: Vec<(String, Option<faro_core::domain::ServiceId>)>,
    /// Title and ID of each group presage knows.
    pub groups: Vec<(String, faro_core::domain::GroupId)>,
}

/// How long `ids` waits to catch up with queued messages.
const CATCH_UP: std::time::Duration = std::time::Duration::from_secs(30);

/// Look up usernames and list known groups. First catches up with queued
/// messages (up to [`CATCH_UP`]) so presage learns groups from them; those
/// messages are not shown or kept by faro.
pub async fn ids(mut linked: Linked, usernames: &[String]) -> Result<Ids, Error> {
    use futures::StreamExt as _;
    use presage::model::messages::Received;
    use presage::store::ContentsStore as _;

    let me = faro_core::domain::ServiceId::Aci(linked.registration_data().service_ids.aci);
    let mut stream = Box::pin(linked.clone().receive_messages()).await?;
    let caught_up = tokio::time::timeout(CATCH_UP, async {
        while let Some(received) = stream.next().await {
            if matches!(received, Received::QueueEmpty) {
                break;
            }
        }
    })
    .await;
    if caught_up.is_err() {
        tracing::warn!("still catching up with queued messages; listing what's known");
    }
    drop(stream);
    let mut found = Vec::new();
    for name in usernames {
        let aci = Box::pin(linked.lookup_username(name)).await?;
        found.push((
            name.clone(),
            aci.map(|aci| faro_core::domain::ServiceId::Aci(uuid::Uuid::from(aci))),
        ));
    }
    let mut groups = Vec::new();
    for entry in linked.store().groups().await? {
        let (key, group) = entry?;
        groups.push((group.title, convert::group_id(&key)));
    }
    Ok(Ids {
        me,
        usernames: found,
        groups,
    })
}

#[cfg(test)]
#[expect(
    clippy::disallowed_methods,
    reason = "#[tokio::test] expands to block_on; tests may block"
)]
mod tests {
    use std::os::unix::fs::PermissionsExt as _;

    use super::*;

    fn mode(path: &Path) -> u32 {
        std::fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    #[tokio::test]
    async fn the_store_is_private_and_starts_unlinked() {
        let dir = tempfile::tempdir().unwrap();
        let file = store_file(dir.path());
        std::fs::write(&file, b"").unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
        let store = open_store(&file).await.unwrap();
        assert_eq!(mode(&file), faro_core::paths::FILE_MODE);
        assert!(!is_linked(&store).await);
        assert!(matches!(load(store).await, Err(Error::NotLinked)));
    }

    #[test]
    fn device_names() {
        assert_eq!(default_device_name(Some("smithers")), "faro (smithers)");
        assert_eq!(
            default_device_name(Some("smithers.example.com")),
            "faro (smithers)"
        );
        assert_eq!(default_device_name(Some("  ")), "faro");
        assert_eq!(default_device_name(None), "faro");
    }
}
