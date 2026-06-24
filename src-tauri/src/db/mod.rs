use std::sync::Mutex;

use rusqlite::Connection;
use tauri::{App, Manager, Runtime};

pub mod crypto;
mod schema;
pub mod secret_store;
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
/// The database is encrypted at rest with SQLCipher. In "open mode" the key lives in a local
/// file ([`secret_store`]) rather than the OS keychain, so the app never prompts for the laptop
/// password. On launch the existing file (if any) is reconciled with the current key:
/// * a legacy pre-encryption plaintext database is encrypted in place;
/// * a database that can't be decrypted with the current key (e.g. the stored key was
///   lost or the file is corrupt) is set aside — not deleted — so the app starts fresh
///   instead of aborting.
pub fn setup_database<R: Runtime>(app: &App<R>) -> Result<(), Box<dyn std::error::Error>> {
    let data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("Cannot resolve app data dir: {e}"))?;

    std::fs::create_dir_all(&data_dir)?;
    let db_path = data_dir.join(DB_FILE);

    // "Open mode": secrets (the SQLCipher key + connector tokens) live in a local file in the app
    // data directory rather than the OS keychain, so the app never prompts for the laptop password.
    secret_store::init(&data_dir);
    // One-time only: pull any secrets still held in the OS keychain (from before open mode) into the
    // file store. This preserves an existing encrypted database — its key is migrated rather than
    // lost — and any connector logins. It is the single remaining keychain prompt; afterwards the
    // keychain is never read again. Absent entries don't prompt, so fresh installs see no prompt.
    secret_store::migrate_from_keychain(
        crypto::KEY_SERVICE,
        &[
            crypto::KEY_ACCOUNT,
            secrets::SNAPTRADE_CONSUMER_KEY,
            secrets::SNAPTRADE_USER_SECRET,
            secrets::SIMPLEFIN_ACCESS_URL,
            secrets::QUESTRADE_REFRESH_TOKEN,
        ],
    )
    .map_err(|e| format!("Secret migration failed: {e}"))?;

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
