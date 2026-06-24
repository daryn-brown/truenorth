//! Database encryption key management.
//!
//! The SQLite database is encrypted with SQLCipher and keyed with a 256-bit value generated on
//! first run. In "open mode" that key is stored in a local file (`secrets.json` in the app data
//! directory) via [`crate::db::secret_store`] rather than the OS keychain, so unlocking the
//! database never prompts for the laptop password. The tradeoff is documented on the secret store:
//! the key sits beside the database, so the encryption no longer protects against file-level access.
//!
//! The raw-key form `x'<hex>'` is used so SQLCipher consumes the 32 bytes directly
//! without a KDF; the same literal must be used everywhere the DB is keyed (open and
//! migration `ATTACH`), so the key is threaded through as a hex string.

use std::path::{Path, PathBuf};

use rand::RngCore;
use rusqlite::Connection;

use crate::db::secret_store;

/// Keychain service identifier. Deliberately kept as the original bundle identifier
/// (the app was renamed to "TrueNorth" but the identifier is unchanged) so the existing
/// encryption key — and therefore the encrypted database — remains readable after the rebrand.
/// Shared with [`crate::db::secrets`] so connector secrets land in the same keychain service.
pub(crate) const KEY_SERVICE: &str = "com.darynbrown.finance-second-brain";
/// Keychain/secret-store entry name for the database encryption key.
pub(crate) const KEY_ACCOUNT: &str = "db-encryption-key";
/// The 16-byte magic header that begins every plaintext SQLite database file.
const SQLITE_MAGIC: &[u8; 16] = b"SQLite format 3\0";

/// Errors that can occur while resolving or applying the encryption key.
#[derive(Debug, thiserror::Error)]
pub enum CryptoError {
    #[error("Secret store error: {0}")]
    Secret(#[from] secret_store::SecretStoreError),

    #[error("Database error: {0}")]
    Db(#[from] rusqlite::Error),

    #[error("Filesystem error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Failed to migrate legacy plaintext database: {0}")]
    Migration(String),
}

/// Generate a fresh random 256-bit key, hex-encoded (64 chars).
fn generate_key_hex() -> String {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

/// Fetch the DB encryption key from the local secret store, creating and persisting a new
/// random key on first run. Returns the key as a 64-character hex string.
///
/// In "open mode" the key lives in `secrets.json` in the app data directory rather than the OS
/// keychain, so reading it never prompts for the laptop password. Existing keychain-held keys are
/// migrated into the file once at startup (see [`secret_store::migrate_from_keychain`]).
pub fn get_or_create_key() -> Result<String, CryptoError> {
    if let Some(key) = secret_store::get(KEY_ACCOUNT)? {
        return Ok(key);
    }
    let key = generate_key_hex();
    secret_store::set(KEY_ACCOUNT, &key)?;
    Ok(key)
}

/// Apply the SQLCipher key to a freshly opened connection. MUST run before any other
/// statement on the connection.
pub fn apply_key(conn: &Connection, key_hex: &str) -> rusqlite::Result<()> {
    conn.execute_batch(&format!("PRAGMA key = \"x'{key_hex}'\";"))
}

/// Whether the file at `path` can be opened and read using `key_hex` (i.e. it is already
/// an encrypted database keyed with our key). Returns false for a plaintext or
/// wrong-key file.
pub fn is_encrypted_with_key(path: &Path, key_hex: &str) -> Result<bool, rusqlite::Error> {
    let conn = Connection::open(path)?;
    apply_key(&conn, key_hex)?;
    Ok(conn
        .query_row("SELECT count(*) FROM sqlite_master", [], |r| {
            r.get::<_, i64>(0)
        })
        .is_ok())
}

/// Best-effort migration of a legacy **plaintext** SQLite database to an encrypted one,
/// in place, using SQLCipher's `sqlcipher_export`. The original file is replaced only
/// after the encrypted copy is fully written.
pub fn migrate_plaintext_to_encrypted(db_path: &Path, key_hex: &str) -> Result<(), CryptoError> {
    let tmp_path = db_path.with_extension("db.migrating");
    if tmp_path.exists() {
        std::fs::remove_file(&tmp_path)?;
    }

    // Open the existing file as plaintext (no key). If this fails to read, it isn't a
    // plaintext SQLite DB we can migrate.
    let conn = Connection::open(db_path)?;
    conn.query_row("SELECT count(*) FROM sqlite_master", [], |r| {
        r.get::<_, i64>(0)
    })
    .map_err(|e| CryptoError::Migration(format!("source is not a readable plaintext DB: {e}")))?;

    let safe_tmp = tmp_path.display().to_string().replace('\'', "''");
    conn.execute_batch(&format!(
        "ATTACH DATABASE '{safe_tmp}' AS encrypted KEY \"x'{key_hex}'\"; \
         SELECT sqlcipher_export('encrypted'); \
         DETACH DATABASE encrypted;"
    ))
    .map_err(|e| CryptoError::Migration(e.to_string()))?;
    drop(conn);

    // Swap the encrypted copy in for the plaintext original.
    std::fs::rename(&tmp_path, db_path)?;
    Ok(())
}

/// Whether the file at `path` begins with the plaintext SQLite header. A SQLCipher
/// (encrypted) database does not — its header bytes are part of the ciphertext — so this
/// reliably distinguishes a legacy *unencrypted* DB from an encrypted one without a key.
pub fn is_plaintext_sqlite(path: &Path) -> Result<bool, std::io::Error> {
    use std::io::Read;
    let mut head = [0u8; 16];
    match std::fs::File::open(path)?.read_exact(&mut head) {
        Ok(()) => Ok(&head == SQLITE_MAGIC),
        // A file shorter than the header can't be a SQLite DB we should migrate.
        Err(ref e) if e.kind() == std::io::ErrorKind::UnexpectedEof => Ok(false),
        Err(e) => Err(e),
    }
}

/// Move an unreadable database — one encrypted with a key we no longer have, or corrupt —
/// aside so the app can start fresh instead of aborting on launch. The original bytes are
/// preserved (renamed, never deleted) so they can be recovered if the key turns up. Any
/// WAL/SHM sidecars are moved too. Returns the path the database was preserved at.
pub fn quarantine_unreadable_db(db_path: &Path) -> Result<PathBuf, CryptoError> {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let backup = db_path.with_extension(format!("db.unreadable-{ts}"));
    std::fs::rename(db_path, &backup)?;
    for ext in ["db-wal", "db-shm"] {
        let side = db_path.with_extension(ext);
        if side.exists() {
            let _ = std::fs::rename(
                &side,
                db_path.with_extension(format!("{ext}.unreadable-{ts}")),
            );
        }
    }
    Ok(backup)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema::apply_schema;

    #[test]
    fn generated_key_is_64_hex_chars() {
        let key = generate_key_hex();
        assert_eq!(key.len(), 64);
        assert!(key.chars().all(|c| c.is_ascii_hexdigit()));
        // Two successive keys must differ.
        assert_ne!(key, generate_key_hex());
    }

    #[test]
    fn encrypted_in_memory_db_roundtrips_with_key() {
        let key = generate_key_hex();
        let conn = Connection::open_in_memory().unwrap();
        apply_key(&conn, &key).unwrap();
        apply_schema(&conn).unwrap();

        conn.execute(
            "INSERT INTO accounts (name, institution, account_type, currency, jurisdiction) \
             VALUES ('Acct', 'Bank', 'savings', 'CAD', 'CA')",
            [],
        )
        .unwrap();
        let count: i64 = conn
            .query_row("SELECT count(*) FROM accounts", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn migrates_plaintext_db_to_encrypted() {
        let unique = format!(
            "fsb-migrate-{}-{}.db",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let db_path = std::env::temp_dir().join(unique);
        let _ = std::fs::remove_file(&db_path);

        // 1) Create a plaintext DB with one account.
        {
            let conn = Connection::open(&db_path).unwrap();
            apply_schema(&conn).unwrap();
            conn.execute(
                "INSERT INTO accounts (name, institution, account_type, currency, jurisdiction) \
                 VALUES ('Legacy', 'OldBank', 'chequing', 'USD', 'US')",
                [],
            )
            .unwrap();
        }

        let key = generate_key_hex();
        assert!(!is_encrypted_with_key(&db_path, &key).unwrap());

        // 2) Migrate it.
        migrate_plaintext_to_encrypted(&db_path, &key).unwrap();

        // 3) Now it is readable only with the key, and the data survived.
        assert!(is_encrypted_with_key(&db_path, &key).unwrap());
        let conn = Connection::open(&db_path).unwrap();
        apply_key(&conn, &key).unwrap();
        let name: String = conn
            .query_row("SELECT name FROM accounts LIMIT 1", [], |r| r.get(0))
            .unwrap();
        assert_eq!(name, "Legacy");

        let _ = std::fs::remove_file(&db_path);
    }

    fn unique_suffix() -> String {
        format!(
            "{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        )
    }

    #[test]
    fn detects_plaintext_vs_encrypted_header() {
        let dir = std::env::temp_dir();
        let plain = dir.join(format!("fsb-plain-{}.db", unique_suffix()));
        let enc = dir.join(format!("fsb-enc-{}.db", unique_suffix()));

        {
            let conn = Connection::open(&plain).unwrap();
            apply_schema(&conn).unwrap();
        }
        assert!(is_plaintext_sqlite(&plain).unwrap());

        let key = generate_key_hex();
        {
            let conn = Connection::open(&enc).unwrap();
            apply_key(&conn, &key).unwrap();
            apply_schema(&conn).unwrap();
        }
        assert!(!is_plaintext_sqlite(&enc).unwrap());

        let _ = std::fs::remove_file(&plain);
        let _ = std::fs::remove_file(&enc);
    }

    #[test]
    fn unreadable_db_is_quarantined_not_deleted() {
        let dir = std::env::temp_dir();
        let db = dir.join(format!("fsb-foreign-{}.db", unique_suffix()));

        // Encrypt with key A.
        let key_a = generate_key_hex();
        {
            let conn = Connection::open(&db).unwrap();
            apply_key(&conn, &key_a).unwrap();
            apply_schema(&conn).unwrap();
        }

        // A different key cannot read it, and it is not plaintext — the exact state that
        // previously crashed the app on launch.
        let key_b = generate_key_hex();
        assert!(!is_plaintext_sqlite(&db).unwrap());
        assert!(!is_encrypted_with_key(&db, &key_b).unwrap());

        // It is moved aside, not lost.
        let backup = quarantine_unreadable_db(&db).unwrap();
        assert!(!db.exists());
        assert!(backup.exists());

        let _ = std::fs::remove_file(&backup);
    }
}
