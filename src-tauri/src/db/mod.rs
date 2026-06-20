use std::sync::Mutex;

use rusqlite::Connection;
use tauri::{App, Manager, Runtime};

pub mod crypto;
mod schema;
pub use schema::{apply_schema, seed_defaults};

pub const DB_FILE: &str = "finance-second-brain.db";

/// Shared application state holding the SQLite connection.
pub struct AppDb(pub Mutex<Connection>);

/// Open (or create) the encrypted database, apply the schema, and register AppDb state.
///
/// The database is encrypted at rest with SQLCipher. The key lives in the OS keychain;
/// a legacy plaintext database from before encryption was enabled is migrated in place.
pub fn setup_database<R: Runtime>(app: &App<R>) -> Result<(), Box<dyn std::error::Error>> {
    let data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("Cannot resolve app data dir: {e}"))?;

    std::fs::create_dir_all(&data_dir)?;
    let db_path = data_dir.join(DB_FILE);

    let key = crypto::get_or_create_key()?;

    // Migrate a pre-encryption plaintext database, if one is present.
    if db_path.exists() && !crypto::is_encrypted_with_key(&db_path, &key)? {
        crypto::migrate_plaintext_to_encrypted(&db_path, &key)?;
    }

    let conn = Connection::open(&db_path)
        .map_err(|e| format!("Failed to open database at {}: {e}", db_path.display()))?;

    crypto::apply_key(&conn, &key)?;
    apply_schema(&conn)?;
    seed_defaults(&conn)?;

    app.manage(AppDb(Mutex::new(conn)));
    Ok(())
}
