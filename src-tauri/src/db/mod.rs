use std::sync::Mutex;

use rusqlite::Connection;
use tauri::{App, Manager, Runtime};

mod schema;
pub use schema::{apply_schema, seed_defaults};

pub const DB_FILE: &str = "finance-second-brain.db";

/// Shared application state holding the SQLite connection.
pub struct AppDb(pub Mutex<Connection>);

/// Open (or create) the database, apply the schema, and register AppDb state.
pub fn setup_database<R: Runtime>(app: &App<R>) -> Result<(), Box<dyn std::error::Error>> {
    let data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("Cannot resolve app data dir: {e}"))?;

    std::fs::create_dir_all(&data_dir)?;
    let db_path = data_dir.join(DB_FILE);

    let conn = Connection::open(&db_path)
        .map_err(|e| format!("Failed to open database at {}: {e}", db_path.display()))?;

    apply_schema(&conn)?;
    seed_defaults(&conn)?;

    app.manage(AppDb(Mutex::new(conn)));
    Ok(())
}
