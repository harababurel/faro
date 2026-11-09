//! Encryption at rest (D168): envelope-encrypted master key (`vault.json`),
//! domain-separated `SQLCipher` keys for `faro.db` and `presage.db`, and
//! `AES-256-GCM` media encryption for `attachments/` and `pictures/`.

use std::fmt::Write as _;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use aes_gcm::aead::{Aead as _, Payload};
use aes_gcm::{Aes256Gcm, KeyInit as _};
use argon2::{Algorithm, Argon2, Params, Version};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use hkdf::Hkdf;
use rand::RngCore as _;
use sha2::Sha256;
use zeroize::Zeroizing;

use crate::paths::{self, Paths};

/// File in `<data dir>/` that marks an encrypted data directory and stores the
/// Argon2id parameters, salt, and wrapped master key.
pub const VAULT_FILE: &str = "vault.json";

/// 8-byte magic prefix on every encrypted file in `attachments/` and `pictures/`.
pub const MEDIA_MAGIC: &[u8; 8] = b"FAROENC1";

const VAULT_VERSION: u32 = 1;
const VAULT_AAD: &[u8] = b"faro-vault-v1";
const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 12;
const KEY_LEN: usize = 32;
const TAG_LEN: usize = 16;
const WRAPPED_KEY_LEN: usize = KEY_LEN + TAG_LEN;

#[cfg(not(test))]
const DEFAULT_M_COST_KIB: u32 = 19_456;
#[cfg(test)]
const DEFAULT_M_COST_KIB: u32 = 1_024;

#[cfg(not(test))]
const DEFAULT_T_COST: u32 = 2;
#[cfg(test)]
const DEFAULT_T_COST: u32 = 1;

const DEFAULT_P_COST: u32 = 1;

static ACTIVE_KEYS: RwLock<Option<(PathBuf, VaultKeys)>> = RwLock::new(None);

#[derive(Debug, thiserror::Error)]
pub enum VaultError {
    #[error("this data directory is not encrypted (run `faro encrypt` first)")]
    NotEncrypted,
    #[error(
        "this data directory is already encrypted (use `faro passwd` to change the passphrase)"
    )]
    AlreadyEncrypted,
    #[error("passphrase cannot be empty")]
    EmptyPassphrase,
    #[error("incorrect passphrase")]
    WrongPassphrase,
    #[error("corrupt vault file ({0})")]
    Corrupt(String),
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
    #[error("database migration failed: {0}")]
    Database(String),
}

/// Stored metadata in `<data dir>/vault.json`. Contains no secret material;
/// `wrapped_key` is encrypted with `AES-256-GCM` under the Argon2id KEK.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VaultHeader {
    pub version: u32,
    pub m_cost_kib: u32,
    pub t_cost: u32,
    pub p_cost: u32,
    pub salt: [u8; SALT_LEN],
    pub nonce: [u8; NONCE_LEN],
    pub wrapped_key: [u8; WRAPPED_KEY_LEN],
}

/// Domain-separated subkeys derived from the 32-byte master DEK via HKDF-SHA256.
#[derive(Clone)]
pub struct VaultKeys {
    faro_db: Zeroizing<[u8; KEY_LEN]>,
    presage_db: Zeroizing<[u8; KEY_LEN]>,
    media: Zeroizing<[u8; KEY_LEN]>,
}

/// Summary of an `encrypt_directory` or `decrypt_directory` run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MigrationSummary {
    pub faro_db: bool,
    pub presage_db: bool,
    pub media_files: usize,
}

impl VaultHeader {
    /// Generate a fresh random 32-byte master DEK and wrap it under `passphrase`.
    pub fn create(passphrase: &str) -> Result<(Self, VaultKeys), VaultError> {
        if passphrase.is_empty() {
            return Err(VaultError::EmptyPassphrase);
        }
        let mut master = Zeroizing::new([0u8; KEY_LEN]);
        rand::rng().fill_bytes(master.as_mut());
        let header = Self::wrap_master_key(&master, passphrase)?;
        let keys = VaultKeys::derive(&master);
        Ok((header, keys))
    }

    fn wrap_master_key(master: &[u8; KEY_LEN], passphrase: &str) -> Result<Self, VaultError> {
        if passphrase.is_empty() {
            return Err(VaultError::EmptyPassphrase);
        }
        let mut salt = [0u8; SALT_LEN];
        let mut nonce = [0u8; NONCE_LEN];
        rand::rng().fill_bytes(&mut salt);
        rand::rng().fill_bytes(&mut nonce);
        let kek = derive_kek(
            passphrase,
            &salt,
            DEFAULT_M_COST_KIB,
            DEFAULT_T_COST,
            DEFAULT_P_COST,
        )?;
        let cipher = Aes256Gcm::new((&*kek).into());
        let ciphertext = cipher
            .encrypt(
                (&nonce).into(),
                Payload {
                    msg: master,
                    aad: VAULT_AAD,
                },
            )
            .map_err(|_| VaultError::Corrupt("AES-GCM wrap failed".to_owned()))?;
        let wrapped_key: [u8; WRAPPED_KEY_LEN] = ciphertext
            .try_into()
            .map_err(|_| VaultError::Corrupt("unexpected wrapped key length".to_owned()))?;
        Ok(Self {
            version: VAULT_VERSION,
            m_cost_kib: DEFAULT_M_COST_KIB,
            t_cost: DEFAULT_T_COST,
            p_cost: DEFAULT_P_COST,
            salt,
            nonce,
            wrapped_key,
        })
    }

    fn unwrap_master_key(&self, passphrase: &str) -> Result<Zeroizing<[u8; KEY_LEN]>, VaultError> {
        if passphrase.is_empty() {
            return Err(VaultError::WrongPassphrase);
        }
        let kek = derive_kek(
            passphrase,
            &self.salt,
            self.m_cost_kib,
            self.t_cost,
            self.p_cost,
        )?;
        let cipher = Aes256Gcm::new((&*kek).into());
        let plain = Zeroizing::new(
            cipher
                .decrypt(
                    (&self.nonce).into(),
                    Payload {
                        msg: &self.wrapped_key,
                        aad: VAULT_AAD,
                    },
                )
                .map_err(|_| VaultError::WrongPassphrase)?,
        );
        let array: [u8; KEY_LEN] = plain
            .as_slice()
            .try_into()
            .map_err(|_| VaultError::Corrupt("unwrapped key length mismatch".to_owned()))?;
        Ok(Zeroizing::new(array))
    }

    /// Unwrap the master DEK and derive the subkeys.
    pub fn unlock(&self, passphrase: &str) -> Result<VaultKeys, VaultError> {
        let master = self.unwrap_master_key(passphrase)?;
        Ok(VaultKeys::derive(&master))
    }

    /// Check whether `passphrase` unwraps this vault header.
    pub fn verify_passphrase(&self, passphrase: &str) -> bool {
        self.unwrap_master_key(passphrase).is_ok()
    }

    /// Closure suitable for the TUI lock screen's passphrase check.
    pub fn verifier(self) -> Arc<dyn Fn(&str) -> bool + Send + Sync> {
        Arc::new(move |candidate: &str| self.verify_passphrase(candidate))
    }

    pub fn to_json(&self) -> String {
        let val = serde_json::json!({
            "version": self.version,
            "m_cost_kib": self.m_cost_kib,
            "t_cost": self.t_cost,
            "p_cost": self.p_cost,
            "salt": BASE64.encode(self.salt),
            "nonce": BASE64.encode(self.nonce),
            "wrapped_key": BASE64.encode(self.wrapped_key),
        });
        let mut text = serde_json::to_string_pretty(&val).unwrap_or_default();
        text.push('\n');
        text
    }

    pub fn from_json(text: &str) -> Result<Self, VaultError> {
        let val: serde_json::Value =
            serde_json::from_str(text).map_err(|e| VaultError::Corrupt(e.to_string()))?;
        let version = val
            .get("version")
            .and_then(serde_json::Value::as_u64)
            .and_then(|v| u32::try_from(v).ok())
            .ok_or_else(|| VaultError::Corrupt("missing version".to_owned()))?;
        if version != VAULT_VERSION {
            return Err(VaultError::Corrupt(format!(
                "unsupported vault version {version}"
            )));
        }
        let m_cost_kib = u32_field(&val, "m_cost_kib")?;
        let t_cost = u32_field(&val, "t_cost")?;
        let p_cost = u32_field(&val, "p_cost")?;
        let salt = decode_fixed::<SALT_LEN>(&val, "salt")?;
        let nonce = decode_fixed::<NONCE_LEN>(&val, "nonce")?;
        let wrapped_key = decode_fixed::<WRAPPED_KEY_LEN>(&val, "wrapped_key")?;
        Ok(Self {
            version,
            m_cost_kib,
            t_cost,
            p_cost,
            salt,
            nonce,
            wrapped_key,
        })
    }
}

impl VaultKeys {
    fn derive(master: &[u8; KEY_LEN]) -> Self {
        let hk = Hkdf::<Sha256>::new(Some(b"faro-vault-v1-hkdf"), master);
        let mut faro_db = Zeroizing::new([0u8; KEY_LEN]);
        let mut presage_db = Zeroizing::new([0u8; KEY_LEN]);
        let mut media = Zeroizing::new([0u8; KEY_LEN]);
        let _ = hk.expand(b"faro-v1-faro-db", faro_db.as_mut());
        let _ = hk.expand(b"faro-v1-presage-db", presage_db.as_mut());
        let _ = hk.expand(b"faro-v1-media-aead", media.as_mut());
        Self {
            faro_db,
            presage_db,
            media,
        }
    }

    /// `SQLCipher` raw 256-bit key literal (`"x'<64 hex>'"`) for `faro.db`.
    pub fn faro_db_pragma(&self) -> String {
        sqlcipher_hex_pragma(&self.faro_db)
    }

    /// `SQLCipher` raw 256-bit key literal (`"x'<64 hex>'"`) for `presage.db`.
    pub fn presage_db_pragma(&self) -> String {
        sqlcipher_hex_pragma(&self.presage_db)
    }

    /// Encrypt `plaintext` as `FAROENC1 || nonce(12) || ciphertext+tag`.
    pub fn encrypt_media(&self, plaintext: &[u8]) -> io::Result<Vec<u8>> {
        if plaintext.is_empty() || is_encrypted_media(plaintext) {
            return Ok(plaintext.to_vec());
        }
        let mut nonce = [0u8; NONCE_LEN];
        rand::rng().fill_bytes(&mut nonce);
        let cipher = Aes256Gcm::new((&*self.media).into());
        let ct = cipher
            .encrypt(
                (&nonce).into(),
                Payload {
                    msg: plaintext,
                    aad: MEDIA_MAGIC,
                },
            )
            .map_err(|_| io::Error::other("media encryption failed"))?;
        let mut out = Vec::with_capacity(MEDIA_MAGIC.len() + NONCE_LEN + ct.len());
        out.extend_from_slice(MEDIA_MAGIC);
        out.extend_from_slice(&nonce);
        out.extend_from_slice(&ct);
        Ok(out)
    }

    /// Decrypt a `FAROENC1` media blob back to plaintext. Non-encrypted slices
    /// are returned unchanged.
    pub fn decrypt_media(&self, bytes: &[u8]) -> io::Result<Vec<u8>> {
        let Some(rest) = bytes.strip_prefix(MEDIA_MAGIC) else {
            return Ok(bytes.to_vec());
        };
        let Some((nonce, ct)) = rest.split_first_chunk::<NONCE_LEN>() else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "truncated encrypted media header",
            ));
        };
        let cipher = Aes256Gcm::new((&*self.media).into());
        cipher
            .decrypt(
                nonce.into(),
                Payload {
                    msg: ct,
                    aad: MEDIA_MAGIC,
                },
            )
            .map_err(|_| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "media authentication failed (wrong key or corrupt file)",
                )
            })
    }
}

/// Whether `bytes` begins with the `FAROENC1` magic header.
pub fn is_encrypted_media(bytes: &[u8]) -> bool {
    bytes.starts_with(MEDIA_MAGIC)
}

/// Whether `<data_dir>/vault.json` exists.
#[expect(clippy::disallowed_methods, reason = "startup or CLI check")]
pub fn is_encrypted(data_dir: &Path) -> bool {
    data_dir.join(VAULT_FILE).is_file()
}

/// Read `<data_dir>/vault.json`.
#[expect(
    clippy::disallowed_methods,
    reason = "startup or CLI vault header read"
)]
pub fn read_header(data_dir: &Path) -> Result<VaultHeader, VaultError> {
    let path = data_dir.join(VAULT_FILE);
    match std::fs::read_to_string(&path) {
        Ok(text) => VaultHeader::from_json(&text),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Err(VaultError::NotEncrypted),
        Err(err) => Err(VaultError::Io(err)),
    }
}

/// Unlock `<data_dir>/vault.json` with `passphrase`, installing the derived keys
/// for `data_dir` in this process (`Store::open`, `faro_presage::open_store`, and media I/O).
pub fn unlock(data_dir: &Path, passphrase: &str) -> Result<VaultHeader, VaultError> {
    let header = read_header(data_dir)?;
    let keys = header.unlock(passphrase)?;
    set_active_keys(Some((data_dir.to_path_buf(), keys)));
    Ok(header)
}

/// Install or clear the process-wide `(data_dir, VaultKeys)`.
pub fn set_active_keys(keys: Option<(PathBuf, VaultKeys)>) {
    if let Ok(mut guard) = ACTIVE_KEYS.write() {
        *guard = keys;
    }
}

/// `SQLCipher` `PRAGMA key` value for `faro.db` at `db_path` if its data directory is unlocked.
pub fn faro_db_pragma_for(db_path: &Path) -> Option<String> {
    ACTIVE_KEYS.read().ok().and_then(|g| {
        g.as_ref()
            .filter(|(dir, _)| db_path.starts_with(dir))
            .map(|(_, k)| k.faro_db_pragma())
    })
}

/// `SQLCipher` `PRAGMA key` value for `presage.db` at `db_path` if its data directory is unlocked.
pub fn presage_db_pragma_for(db_path: &Path) -> Option<String> {
    ACTIVE_KEYS.read().ok().and_then(|g| {
        g.as_ref()
            .filter(|(dir, _)| db_path.starts_with(dir))
            .map(|(_, k)| k.presage_db_pragma())
    })
}

/// Encrypt `bytes` if `path` resides under the unlocked vault directory; otherwise return a copy.
pub fn encrypt_media_for_path(path: &Path, bytes: &[u8]) -> io::Result<Vec<u8>> {
    let guard = ACTIVE_KEYS
        .read()
        .map_err(|_| io::Error::other("vault lock poisoned"))?;
    match guard.as_ref() {
        Some((dir, keys)) if path.starts_with(dir) => keys.encrypt_media(bytes),
        Some(_) | None => Ok(bytes.to_vec()),
    }
}

/// Decrypt `bytes` if it starts with `FAROENC1`; fails with `PermissionDenied`
/// if the file is encrypted and no vault is unlocked.
pub fn decrypt_media_if_needed(bytes: &[u8]) -> io::Result<Vec<u8>> {
    if !is_encrypted_media(bytes) {
        return Ok(bytes.to_vec());
    }
    let guard = ACTIVE_KEYS
        .read()
        .map_err(|_| io::Error::other("vault lock poisoned"))?;
    let Some((_, keys)) = guard.as_ref() else {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "encrypted media file requires an unlocked vault",
        ));
    };
    keys.decrypt_media(bytes)
}

/// Change the passphrase protecting `<data_dir>/vault.json` in O(1) time by
/// re-wrapping the 32-byte master DEK under `new_passphrase`.
pub fn change_passphrase(
    paths: &Paths,
    current_passphrase: &str,
    new_passphrase: &str,
) -> Result<VaultHeader, VaultError> {
    if new_passphrase.is_empty() {
        return Err(VaultError::EmptyPassphrase);
    }
    let old_header = read_header(paths.data_dir())?;
    let master = old_header.unwrap_master_key(current_passphrase)?;
    let new_header = VaultHeader::wrap_master_key(&master, new_passphrase)?;
    let new_keys = VaultKeys::derive(&master);
    paths::write_raw_private_atomic(&paths.vault_file(), new_header.to_json().as_bytes())?;
    set_active_keys(Some((paths.data_dir().to_path_buf(), new_keys)));
    Ok(new_header)
}

/// Encrypt an existing unencrypted data directory (`faro.db`, `presage.db`,
/// `attachments/` and `pictures/`) in place, writing `vault.json` once
/// everything succeeds.
#[cfg(feature = "sqlite")]
pub async fn encrypt_directory(
    paths: &Paths,
    passphrase: &str,
) -> Result<MigrationSummary, VaultError> {
    let data_dir = paths.data_dir();
    if is_encrypted(data_dir) {
        return Err(VaultError::AlreadyEncrypted);
    }
    let (header, keys) = VaultHeader::create(passphrase)?;
    let faro_db = paths.db_file();
    let presage_db = data_dir.join("presage.db");

    let faro_staging = staging_path(&faro_db, "enc");
    let presage_staging = staging_path(&presage_db, "enc");

    let has_faro = export_sqlite_db(&faro_db, &faro_staging, None, Some(&keys.faro_db_pragma()))
        .await
        .inspect_err(|_| cleanup_file(&faro_staging))?;
    let has_presage = export_sqlite_db(
        &presage_db,
        &presage_staging,
        None,
        Some(&keys.presage_db_pragma()),
    )
    .await
    .inspect_err(|_| {
        cleanup_file(&faro_staging);
        cleanup_file(&presage_staging);
    })?;

    let attachments_dir = paths.attachments_dir();
    let pictures_dir = paths.pictures_dir();
    let keys_for_media = keys.clone();
    let media_files = tokio::task::spawn_blocking(move || -> io::Result<usize> {
        let a = transform_dir_media(&attachments_dir, &keys_for_media, true)?;
        let p = transform_dir_media(&pictures_dir, &keys_for_media, true)?;
        Ok(a + p)
    })
    .await
    .map_err(|e| VaultError::Io(io::Error::other(e)))??;

    if has_faro {
        replace_sqlite_db(&faro_db, &faro_staging)?;
    }
    if has_presage {
        replace_sqlite_db(&presage_db, &presage_staging)?;
    }
    paths::write_raw_private_atomic(&paths.vault_file(), header.to_json().as_bytes())?;
    set_active_keys(Some((data_dir.to_path_buf(), keys)));
    Ok(MigrationSummary {
        faro_db: has_faro,
        presage_db: has_presage,
        media_files,
    })
}

/// Decrypt an encrypted data directory (`faro.db`, `presage.db`,
/// `attachments/` and `pictures/`) back to plaintext and remove `vault.json`.
#[cfg(feature = "sqlite")]
pub async fn decrypt_directory(
    paths: &Paths,
    passphrase: &str,
) -> Result<MigrationSummary, VaultError> {
    let data_dir = paths.data_dir();
    let header = read_header(data_dir)?;
    let keys = header.unlock(passphrase)?;
    let faro_db = paths.db_file();
    let presage_db = data_dir.join("presage.db");

    let faro_staging = staging_path(&faro_db, "dec");
    let presage_staging = staging_path(&presage_db, "dec");

    let has_faro = export_sqlite_db(&faro_db, &faro_staging, Some(&keys.faro_db_pragma()), None)
        .await
        .inspect_err(|_| cleanup_file(&faro_staging))?;
    let has_presage = export_sqlite_db(
        &presage_db,
        &presage_staging,
        Some(&keys.presage_db_pragma()),
        None,
    )
    .await
    .inspect_err(|_| {
        cleanup_file(&faro_staging);
        cleanup_file(&presage_staging);
    })?;

    let attachments_dir = paths.attachments_dir();
    let pictures_dir = paths.pictures_dir();
    let keys_for_media = keys.clone();
    let media_files = tokio::task::spawn_blocking(move || -> io::Result<usize> {
        let a = transform_dir_media(&attachments_dir, &keys_for_media, false)?;
        let p = transform_dir_media(&pictures_dir, &keys_for_media, false)?;
        Ok(a + p)
    })
    .await
    .map_err(|e| VaultError::Io(io::Error::other(e)))??;

    if has_faro {
        replace_sqlite_db(&faro_db, &faro_staging)?;
    }
    if has_presage {
        replace_sqlite_db(&presage_db, &presage_staging)?;
    }
    cleanup_file(&paths.vault_file());
    set_active_keys(None);
    Ok(MigrationSummary {
        faro_db: has_faro,
        presage_db: has_presage,
        media_files,
    })
}

#[cfg(feature = "sqlite")]
fn staging_path(db_path: &Path, tag: &str) -> PathBuf {
    let name = db_path.file_name().and_then(|n| n.to_str()).unwrap_or("db");
    db_path.with_file_name(format!(".{name}.{tag}-staging"))
}

#[cfg(feature = "sqlite")]
#[expect(clippy::disallowed_methods, reason = "CLI migration cleanup")]
fn cleanup_file(path: &Path) {
    let _ = std::fs::remove_file(path);
}

#[cfg(feature = "sqlite")]
#[expect(clippy::disallowed_methods, reason = "CLI database swap after export")]
fn replace_sqlite_db(db_path: &Path, staging: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::set_permissions(staging, std::fs::Permissions::from_mode(paths::FILE_MODE))?;
    std::fs::rename(staging, db_path)?;
    for suffix in ["-wal", "-shm"] {
        let mut sidecar = db_path.as_os_str().to_owned();
        sidecar.push(suffix);
        let _ = std::fs::remove_file(PathBuf::from(sidecar));
    }
    Ok(())
}

#[cfg(feature = "sqlite")]
#[expect(
    clippy::disallowed_methods,
    reason = "CLI migration checks db metadata"
)]
async fn export_sqlite_db(
    src: &Path,
    dest: &Path,
    src_pragma: Option<&str>,
    dest_pragma: Option<&str>,
) -> Result<bool, VaultError> {
    use sqlx::sqlite::SqliteConnectOptions;
    use sqlx::{ConnectOptions as _, Connection as _};

    match std::fs::metadata(src) {
        Ok(meta) if meta.len() == 0 => {
            let _ = std::fs::remove_file(src);
            return Ok(false);
        }
        Ok(_) => {}
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(err) => return Err(VaultError::Io(err)),
    }
    cleanup_file(dest);
    paths::open_private_append(dest)?;

    let mut opts = SqliteConnectOptions::new()
        .filename(src)
        .busy_timeout(std::time::Duration::from_secs(1));
    if let Some(pragma) = src_pragma {
        opts = opts.pragma("key", pragma.to_owned());
    }
    let mut conn = opts
        .connect()
        .await
        .map_err(|e| VaultError::Database(format!("opening {}: {e}", src.display())))?;
    let _ = sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
        .execute(&mut conn)
        .await;
    let (user_version,): (i64,) = sqlx::query_as("PRAGMA main.user_version")
        .fetch_one(&mut conn)
        .await
        .map_err(|e| VaultError::Database(format!("reading {}: {e}", src.display())))?;

    let attach_sql = match dest_pragma {
        Some(pragma) => format!("ATTACH DATABASE ?1 AS target KEY {pragma}"),
        None => "ATTACH DATABASE ?1 AS target KEY ''".to_owned(),
    };
    let dest_str = dest
        .to_str()
        .ok_or_else(|| VaultError::Database("non-UTF-8 database path".to_owned()))?;
    sqlx::query(sqlx::AssertSqlSafe(attach_sql))
        .bind(dest_str)
        .execute(&mut conn)
        .await
        .map_err(|e| VaultError::Database(format!("attaching staging db: {e}")))?;
    sqlx::query("SELECT sqlcipher_export('target')")
        .execute(&mut conn)
        .await
        .map_err(|e| VaultError::Database(format!("sqlcipher_export on {}: {e}", src.display())))?;
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "PRAGMA target.user_version = {user_version}"
    )))
    .execute(&mut conn)
    .await
    .map_err(|e| VaultError::Database(format!("setting user_version: {e}")))?;
    sqlx::query("DETACH DATABASE target")
        .execute(&mut conn)
        .await
        .map_err(|e| VaultError::Database(format!("detaching staging db: {e}")))?;
    conn.close()
        .await
        .map_err(|e| VaultError::Database(e.to_string()))?;

    // Verify the exported database opens cleanly under its target key.
    let mut verify_opts = SqliteConnectOptions::new().filename(dest).read_only(true);
    if let Some(pragma) = dest_pragma {
        verify_opts = verify_opts.pragma("key", pragma.to_owned());
    }
    let mut verify_conn = verify_opts
        .connect()
        .await
        .map_err(|e| VaultError::Database(format!("verifying exported {}: {e}", src.display())))?;
    let _: (i64,) = sqlx::query_as("SELECT count(*) FROM sqlite_master")
        .fetch_one(&mut verify_conn)
        .await
        .map_err(|e| VaultError::Database(format!("verifying exported {}: {e}", src.display())))?;
    verify_conn
        .close()
        .await
        .map_err(|e| VaultError::Database(e.to_string()))?;
    Ok(true)
}

#[cfg(feature = "sqlite")]
#[expect(
    clippy::disallowed_methods,
    reason = "runs inside spawn_blocking during CLI migration"
)]
fn transform_dir_media(dir: &Path, keys: &VaultKeys, encrypt: bool) -> io::Result<usize> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(0),
        Err(err) => return Err(err),
    };
    let mut count = 0;
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        let meta = std::fs::symlink_metadata(&path)?;
        if !meta.is_file() || meta.len() == 0 {
            continue;
        }
        let name = entry.file_name();
        if name.to_string_lossy().starts_with('.') {
            continue;
        }
        let raw = std::fs::read(&path)?;
        if encrypt {
            if is_encrypted_media(&raw) {
                continue;
            }
            let encrypted = keys.encrypt_media(&raw)?;
            paths::write_raw_private_atomic(&path, &encrypted)?;
            count += 1;
        } else if is_encrypted_media(&raw) {
            let decrypted = keys.decrypt_media(&raw)?;
            paths::write_raw_private_atomic(&path, &decrypted)?;
            count += 1;
        }
    }
    Ok(count)
}

fn derive_kek(
    passphrase: &str,
    salt: &[u8; SALT_LEN],
    m_cost_kib: u32,
    t_cost: u32,
    p_cost: u32,
) -> Result<Zeroizing<[u8; KEY_LEN]>, VaultError> {
    let params = Params::new(m_cost_kib, t_cost, p_cost, Some(KEY_LEN))
        .map_err(|e| VaultError::Corrupt(format!("invalid Argon2 params: {e}")))?;
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut kek = Zeroizing::new([0u8; KEY_LEN]);
    argon2
        .hash_password_into(passphrase.as_bytes(), salt, kek.as_mut())
        .map_err(|e| VaultError::Corrupt(format!("Argon2 derivation failed: {e}")))?;
    Ok(kek)
}

fn sqlcipher_hex_pragma(key: &[u8; KEY_LEN]) -> String {
    let mut hex = String::with_capacity(KEY_LEN * 2 + 5);
    hex.push_str("\"x'");
    for byte in key {
        let _ = write!(hex, "{byte:02x}");
    }
    hex.push_str("'\"");
    hex
}

fn u32_field(val: &serde_json::Value, name: &str) -> Result<u32, VaultError> {
    val.get(name)
        .and_then(serde_json::Value::as_u64)
        .and_then(|v| u32::try_from(v).ok())
        .ok_or_else(|| VaultError::Corrupt(format!("missing or invalid {name}")))
}

fn decode_fixed<const N: usize>(
    val: &serde_json::Value,
    name: &str,
) -> Result<[u8; N], VaultError> {
    let text = val
        .get(name)
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| VaultError::Corrupt(format!("missing {name}")))?;
    let bytes = BASE64
        .decode(text)
        .map_err(|e| VaultError::Corrupt(format!("invalid base64 in {name}: {e}")))?;
    bytes
        .try_into()
        .map_err(|_| VaultError::Corrupt(format!("unexpected length for {name}")))
}

#[cfg(test)]
#[expect(
    clippy::disallowed_methods,
    reason = "unit tests create temporary files and run tokio::test"
)]
mod tests {
    use super::*;
    use crate::domain::{Contact, ServiceId};
    use crate::store::{Profile, Store};
    use uuid::Uuid;

    #[test]
    fn header_round_trips_and_verifies_passphrase() {
        let (header, keys) = VaultHeader::create("correct horse battery staple").unwrap();
        let json = header.to_json();
        let parsed = VaultHeader::from_json(&json).unwrap();
        assert_eq!(parsed, header);
        assert!(parsed.verify_passphrase("correct horse battery staple"));
        assert!(!parsed.verify_passphrase("wrong passphrase"));
        assert!(!parsed.verify_passphrase(""));

        let unlocked = parsed.unlock("correct horse battery staple").unwrap();
        assert_eq!(unlocked.faro_db_pragma(), keys.faro_db_pragma());
        assert_eq!(unlocked.presage_db_pragma(), keys.presage_db_pragma());
        assert_ne!(unlocked.faro_db_pragma(), unlocked.presage_db_pragma());

        let plain = b"secret attachment payload";
        let ct = keys.encrypt_media(plain).unwrap();
        assert!(is_encrypted_media(&ct));
        assert_ne!(ct, plain);
        assert_eq!(unlocked.decrypt_media(&ct).unwrap(), plain);
    }

    #[cfg(feature = "sqlite")]
    #[tokio::test]
    async fn encrypt_change_passphrase_and_decrypt_directory_round_trip() {
        set_active_keys(None);
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::from_bases(tmp.path(), tmp.path(), None, None);
        paths.ensure_data_dirs().unwrap();

        // 1. Populate an unencrypted store and media files.
        let alice = ServiceId::Aci(Uuid::from_u128(0xA11CE));
        let store = Store::open(&paths.db_file(), Profile::Signal)
            .await
            .unwrap();
        store
            .upsert_contacts(vec![Contact {
                service_id: alice,
                name: None,
                profile_name: Some("Alice".to_owned()),
                phone: None,
            }])
            .await
            .unwrap();
        store.close().await.unwrap();

        let att_path = paths.attachments_dir().join("photo.jpg");
        paths::write_private_atomic(&att_path, b"jpeg-image-bytes").unwrap();
        assert!(!is_encrypted_media(&std::fs::read(&att_path).unwrap()));

        // 2. Encrypt the directory.
        let enc_summary = encrypt_directory(&paths, "pass-one").await.unwrap();
        assert!(enc_summary.faro_db);
        assert_eq!(enc_summary.media_files, 1);
        assert!(is_encrypted(paths.data_dir()));
        assert!(is_encrypted_media(&std::fs::read(&att_path).unwrap()));
        assert_eq!(paths::read_private(&att_path).unwrap(), b"jpeg-image-bytes");

        // Opening the encrypted store with active keys succeeds and preserves data.
        let store = Store::open(&paths.db_file(), Profile::Signal)
            .await
            .unwrap();
        let convs = store.conversations().await.unwrap();
        assert_eq!(convs.len(), 1);
        store.close().await.unwrap();

        // Without active keys, opening the encrypted SQLite database fails.
        set_active_keys(None);
        assert!(
            Store::open(&paths.db_file(), Profile::Signal)
                .await
                .is_err()
        );

        // 3. Change the passphrase in O(1) time and unlock with the new passphrase.
        assert!(matches!(
            change_passphrase(&paths, "wrong", "pass-two"),
            Err(VaultError::WrongPassphrase)
        ));
        change_passphrase(&paths, "pass-one", "pass-two").unwrap();
        assert!(unlock(paths.data_dir(), "pass-one").is_err());
        unlock(paths.data_dir(), "pass-two").unwrap();

        let store = Store::open(&paths.db_file(), Profile::Signal)
            .await
            .unwrap();
        assert_eq!(store.conversations().await.unwrap().len(), 1);
        store.close().await.unwrap();

        // 4. Decrypt the directory back to plaintext.
        let dec_summary = decrypt_directory(&paths, "pass-two").await.unwrap();
        assert!(dec_summary.faro_db);
        assert_eq!(dec_summary.media_files, 1);
        assert!(!is_encrypted(paths.data_dir()));
        assert!(!is_encrypted_media(&std::fs::read(&att_path).unwrap()));
        assert_eq!(std::fs::read(&att_path).unwrap(), b"jpeg-image-bytes");

        let store = Store::open(&paths.db_file(), Profile::Signal)
            .await
            .unwrap();
        assert_eq!(store.conversations().await.unwrap().len(), 1);
        store.close().await.unwrap();
    }
}
