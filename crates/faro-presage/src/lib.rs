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
pub mod transfer;

use std::path::Path;

use presage::Manager;
use presage::manager::Registered;
use presage::model::identity::OnNewIdentity;
use presage::store::StateStore as _;
use presage_store_sqlite::{SqliteConnectOptions, SqliteStore, SqliteStoreError};

pub use backend::{PresageBackend, start};
pub use link::{default_device_name, link, with_backup_capability};
pub use transfer::{
    TransferError, TransferOutcome, TransferStage, TransferSummary, TransferredHistory,
    fetch_transfer_archive,
};

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
    #[error("presage's database: {0}")]
    Database(#[from] sqlx::Error),
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
    let mut options = SqliteConnectOptions::new()
        .filename(file)
        .create_if_missing(true);
    if let Some(pragma) = faro_core::vault::presage_db_pragma_for(file) {
        options = options.pragma("key", pragma);
    }
    Ok(SqliteStore::open_with_options(options, OnNewIdentity::Trust).await?)
}

/// What [`forget_pictures`] cleared from presage's database.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Forgotten {
    pub profiles: u64,
    pub profile_pictures: u64,
    pub group_pictures: u64,
}

/// presage's tables `faro refresh-pictures` empties (D160): people's
/// profiles (names, the picture's path) and both picture caches. presage
/// keeps them forever and has no call to drop one; its `clear_profiles`
/// would also drop everyone's profile keys. Profile keys, contacts, groups
/// and messages stay. A test checks these tables still exist after each
/// presage update.
const FORGET: [&str; 3] = [
    "DELETE FROM profiles",
    "DELETE FROM profile_avatars",
    "DELETE FROM group_avatars",
];

/// Forget presage's copies of profiles and pictures, so the next run fetches
/// them again (D160). Nothing to do when `file` doesn't exist (no account
/// linked here). Run while faro isn't using the database.
pub async fn forget_pictures(file: &Path) -> Result<Forgotten, Error> {
    if !tokio::fs::try_exists(file).await.map_err(Error::Private)? {
        return Ok(Forgotten {
            profiles: 0,
            profile_pictures: 0,
            group_pictures: 0,
        });
    }
    let mut options = SqliteConnectOptions::new().filename(file);
    if let Some(pragma) = faro_core::vault::presage_db_pragma_for(file) {
        options = options.pragma("key", pragma);
    }
    let mut connection = sqlx::ConnectOptions::connect(&options).await?;
    let mut transaction = sqlx::Connection::begin(&mut connection).await?;
    let mut counts = [0; 3];
    for (statement, count) in FORGET.into_iter().zip(&mut counts) {
        *count = sqlx::query(statement)
            .execute(&mut *transaction)
            .await?
            .rows_affected();
    }
    transaction.commit().await?;
    let [profiles, profile_pictures, group_pictures] = counts;
    Ok(Forgotten {
        profiles,
        profile_pictures,
        group_pictures,
    })
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

/// Clear registration data in `presage.db` when the phone returns
/// `RELINK_REQUESTED` during history transfer, so `faro link` can be run
/// again without manually deleting `presage.db`.
pub async fn clear_link(linked: &Linked) -> Result<(), Error> {
    let mut store = linked.store().clone();
    store.clear_registration().await?;
    Ok(())
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

    #[test]
    fn link_url_advertises_backup5_capability() {
        let base: url::Url = "sgnl://linkdevice?uuid=abc&pub_key=def".parse().unwrap();
        let with_cap = with_backup_capability(base);
        assert_eq!(
            with_cap.as_str(),
            "sgnl://linkdevice?uuid=abc&pub_key=def&capabilities=backup5"
        );
    }

    /// `faro refresh-pictures` (D160) relies on presage's table names: this
    /// fails when a presage update renames them. Pictures go; profile keys
    /// stay.
    #[tokio::test]
    async fn forgetting_pictures_keeps_profile_keys() {
        use presage::libsignal_service::zkgroup::profiles::ProfileKey;
        use presage::store::ContentsStore as _;

        let dir = tempfile::tempdir().unwrap();
        let file = store_file(dir.path());
        let none = forget_pictures(&file).await.unwrap();
        assert_eq!(none.group_pictures, 0, "no database yet: nothing to do");

        let mut store = open_store(&file).await.unwrap();
        let person = uuid::Uuid::from_u128(5);
        let key = ProfileKey::create([7; 32]);
        store.upsert_profile_key(&person, key).await.unwrap();
        let group = presage::model::groups::Group {
            title: "Climbing".to_owned(),
            avatar: "groups/abc".to_owned(),
            disappearing_messages_timer: None,
            access_control: None,
            revision: 1,
            members: vec![],
            pending_members: vec![],
            requesting_members: vec![],
            invite_link_password: vec![],
            description: None,
        };
        store.save_group([1; 32], group).await.unwrap();
        store
            .save_group_avatar([1; 32], &vec![1, 2, 3])
            .await
            .unwrap();
        store
            .save_profile_avatar(person, key, &vec![4, 5])
            .await
            .unwrap();

        let forgotten = forget_pictures(&file).await.unwrap();
        assert_eq!(forgotten.group_pictures, 1);
        assert_eq!(forgotten.profile_pictures, 1);
        assert_eq!(store.group_avatar([1; 32]).await.unwrap(), None);
        assert_eq!(store.profile_avatar(person, key).await.unwrap(), None);
        let kept = store
            .profile_key(&presage::libsignal_service::protocol::Aci::from(person).into())
            .await
            .unwrap();
        assert!(kept.is_some_and(|k| k.bytes == key.bytes), "keys stay");
        assert!(store.group([1; 32]).await.unwrap().is_some(), "groups stay");
    }
}
