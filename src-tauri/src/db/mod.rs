use std::sync::Mutex;

use rusqlite::Connection;
use tauri::{App, Manager, Runtime};

pub mod crypto;
mod schema;
pub mod secrets;
pub use schema::{apply_schema, seed_defaults};

// Kept unchanged across the "TrueNorth" rebrand: renaming this file (or the app's bundle
// identifier, which determines the app-data directory) would orphan the existing encrypted
// database. The on-disk name is an internal detail and not shown to the user.
pub const DB_FILE: &str = "finance-second-brain.db";

/// Shared application state holding the SQLite connection.
pub struct AppDb(pub Mutex<Connection>);

/// Open (or create) the encrypted database, apply the schema, and register AppDb state.
///
/// The database is encrypted at rest with SQLCipher and the key lives in the OS keychain.
/// On launch the existing file (if any) is reconciled with the current key:
/// * a legacy pre-encryption plaintext database is encrypted in place;
/// * a database that can't be decrypted with the current key (e.g. the keychain entry was
///   lost or the file is corrupt) is set aside — not deleted — so the app starts fresh
///   instead of aborting.
pub fn setup_database<R: Runtime>(app: &App<R>) -> Result<(), Box<dyn std::error::Error>> {
    let data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("Cannot resolve app data dir: {e}"))?;

    std::fs::create_dir_all(&data_dir)?;
    let db_path = data_dir.join(DB_FILE);

    let key = crypto::get_or_create_key()?;

    // Reconcile any existing database file with the current key.
    if db_path.exists() {
        if crypto::is_plaintext_sqlite(&db_path)? {
            // Legacy unencrypted database — encrypt it in place, preserving the data.
            crypto::migrate_plaintext_to_encrypted(&db_path, &key)?;
        } else if !crypto::is_encrypted_with_key(&db_path, &key)? {
            // Encrypted with a key we don't have, or corrupt. We can't read it, so rather
            // than abort on launch we move it aside (kept for recovery) and start fresh.
            let backup = crypto::quarantine_unreadable_db(&db_path)?;
            eprintln!(
                "warning: existing database could not be decrypted with the current key; \
                 preserved it at {} and starting a fresh database.",
                backup.display()
            );
        }
    }

    let conn = Connection::open(&db_path)
        .map_err(|e| format!("Failed to open database at {}: {e}", db_path.display()))?;

    crypto::apply_key(&conn, &key)?;
    apply_schema(&conn)?;
    seed_defaults(&conn)?;

    app.manage(AppDb(Mutex::new(conn)));
    Ok(())
}
