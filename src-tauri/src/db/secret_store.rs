//! Local-file secret store ("open mode").
//!
//! By design choice, TrueNorth stores its secrets — including the SQLCipher database key and the
//! connector tokens — in a JSON file in the app data directory instead of the OS keychain
//! (macOS Keychain / Windows Credential Manager). This removes every keychain authorization
//! prompt: the app never has to ask for the laptop password to read a secret.
//!
//! The tradeoff is deliberate and explicit: because the database key now sits next to the
//! database, the at-rest encryption no longer protects against someone with access to your
//! files. This is acceptable for a single-user, local-first app where the owner has prioritised
//! a frictionless experience over at-rest encryption. The file is still written `0600`
//! (owner-only) on Unix as a minimal safeguard.
//!
//! On the first launch after switching to this store, [`migrate_from_keychain`] copies any
//! secrets still living in the OS keychain into the file once — the single remaining keychain
//! prompt — so an existing encrypted database (and any connector logins) keep working. After
//! that the keychain is never read again.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use thiserror::Error;

/// Absolute path to the secrets file, set once at startup via [`init`].
static SECRETS_PATH: OnceLock<PathBuf> = OnceLock::new();
/// Serialises read-modify-write cycles so concurrent commands can't clobber the file.
static FILE_LOCK: Mutex<()> = Mutex::new(());

/// Marker key written once the legacy keychain migration has run, so it never repeats.
const MIGRATION_MARKER: &str = "_migrated_from_keychain";

#[derive(Debug, Error)]
pub enum SecretStoreError {
    #[error("Secret store was not initialised before use")]
    NotInitialised,

    #[error("Filesystem error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Secret store file is corrupt: {0}")]
    Parse(#[from] serde_json::Error),
}

/// Point the store at `<data_dir>/secrets.json`. Call once on startup before any secret access.
pub fn init(data_dir: &Path) {
    // Ignore the error from a redundant second call; the path is stable for the process.
    let _ = SECRETS_PATH.set(data_dir.join("secrets.json"));
}

fn path() -> Result<&'static Path, SecretStoreError> {
    SECRETS_PATH
        .get()
        .map(PathBuf::as_path)
        .ok_or(SecretStoreError::NotInitialised)
}

/// Load the secrets map, treating a missing file as an empty store.
fn load_map(path: &Path) -> Result<BTreeMap<String, String>, SecretStoreError> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(BTreeMap::new()),
        Err(e) => Err(e.into()),
    }
}

/// Persist the secrets map atomically (write temp + rename) with owner-only permissions.
fn store_map(path: &Path, map: &BTreeMap<String, String>) -> Result<(), SecretStoreError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let body = serde_json::to_vec_pretty(map)?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, &body)?;
    set_owner_only(&tmp)?;
    std::fs::rename(&tmp, path)?;
    set_owner_only(path)?;
    Ok(())
}

#[cfg(unix)]
fn set_owner_only(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
}

#[cfg(not(unix))]
fn set_owner_only(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

/// Read a secret, returning `None` when no entry exists.
pub fn get(account: &str) -> Result<Option<String>, SecretStoreError> {
    let _guard = FILE_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let map = load_map(path()?)?;
    Ok(map.get(account).cloned())
}

/// Store (or overwrite) a secret.
pub fn set(account: &str, value: &str) -> Result<(), SecretStoreError> {
    let _guard = FILE_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let p = path()?;
    let mut map = load_map(p)?;
    map.insert(account.to_string(), value.to_string());
    store_map(p, &map)
}

/// Delete a secret. Succeeds silently when the entry is already absent.
pub fn delete(account: &str) -> Result<(), SecretStoreError> {
    let _guard = FILE_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let p = path()?;
    let mut map = load_map(p)?;
    if map.remove(account).is_some() {
        store_map(p, &map)?;
    }
    Ok(())
}

/// One-time migration of secrets still held in the OS keychain into the file store.
///
/// For each `account`, if the file doesn't already have it, the legacy keychain entry under
/// `service` is read and copied across. Existing entries trigger one keychain prompt apiece on
/// this first launch only; absent entries return `NoEntry` without prompting. A marker is then
/// written so this never runs again. Best-effort: a denied or failed read is skipped rather than
/// aborting startup.
pub fn migrate_from_keychain(service: &str, accounts: &[&str]) -> Result<(), SecretStoreError> {
    let _guard = FILE_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let p = path()?;
    let mut map = load_map(p)?;
    if map.contains_key(MIGRATION_MARKER) {
        return Ok(());
    }
    for &account in accounts {
        if map.contains_key(account) {
            continue;
        }
        if let Ok(entry) = keyring::Entry::new(service, account) {
            if let Ok(value) = entry.get_password() {
                map.insert(account.to_string(), value);
            }
        }
    }
    map.insert(MIGRATION_MARKER.to_string(), "1".to_string());
    store_map(p, &map)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh_store() {
        // Each test process gets a unique secrets file; init() is a no-op if already set, so the
        // first test to run fixes the path. Tests therefore share one file but use unique keys.
        let dir = std::env::temp_dir().join(format!("tn-secret-store-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        init(&dir);
    }

    fn unique_key(label: &str) -> String {
        format!(
            "{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        )
    }

    #[test]
    fn set_get_delete_roundtrip() {
        fresh_store();
        let k = unique_key("roundtrip");
        assert_eq!(get(&k).unwrap(), None);
        set(&k, "value-123").unwrap();
        assert_eq!(get(&k).unwrap().as_deref(), Some("value-123"));
        set(&k, "value-456").unwrap();
        assert_eq!(get(&k).unwrap().as_deref(), Some("value-456"));
        delete(&k).unwrap();
        assert_eq!(get(&k).unwrap(), None);
        // Deleting an absent key is a no-op.
        delete(&k).unwrap();
    }
}
