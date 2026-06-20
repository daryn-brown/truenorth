//! Database encryption key management.
//!
//! The SQLite database is encrypted at rest with SQLCipher. The 256-bit key is
//! generated on first run and stored in the OS keychain (macOS Keychain / Windows
//! Credential Manager) via the `keyring` crate — never on disk in the clear.
//!
//! The raw-key form `x'<hex>'` is used so SQLCipher consumes the 32 bytes directly
//! without a KDF; the same literal must be used everywhere the DB is keyed (open and
//! migration `ATTACH`), so the key is threaded through as a hex string.

use std::path::Path;

use keyring::Entry;
use rand::RngCore;
use rusqlite::Connection;

/// Keychain service identifier — matches the app's bundle identifier.
const KEY_SERVICE: &str = "com.darynbrown.finance-second-brain";
/// Keychain entry name for the database encryption key.
const KEY_ACCOUNT: &str = "db-encryption-key";

/// Errors that can occur while resolving or applying the encryption key.
#[derive(Debug, thiserror::Error)]
pub enum CryptoError {
    #[error("Keychain error: {0}")]
    Keychain(#[from] keyring::Error),

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

/// Fetch the DB encryption key from the OS keychain, creating and persisting a new
/// random key on first run. Returns the key as a 64-character hex string.
pub fn get_or_create_key() -> Result<String, CryptoError> {
    let entry = Entry::new(KEY_SERVICE, KEY_ACCOUNT)?;
    match entry.get_password() {
        Ok(key) => Ok(key),
        Err(keyring::Error::NoEntry) => {
            let key = generate_key_hex();
            entry.set_password(&key)?;
            Ok(key)
        }
        Err(e) => Err(CryptoError::Keychain(e)),
    }
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
}
