use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::db::AppDb;

// ---------------------------------------------------------------------------
// Payload + result types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct ImportSnapshot {
    /// ISO date YYYY-MM-DD.
    pub snapshot_date: String,
    pub balance: f64,
}

#[derive(Debug, Deserialize)]
pub struct ImportAccount {
    pub name: String,
    pub institution: String,
    pub account_type: String,
    pub currency: String,
    pub jurisdiction: String,
    pub notes: Option<String>,
    #[serde(default)]
    pub snapshots: Vec<ImportSnapshot>,
}

#[derive(Debug, Deserialize)]
pub struct ImportPayload {
    pub accounts: Vec<ImportAccount>,
}

#[derive(Debug, Default, Serialize)]
pub struct ImportSummary {
    pub accounts_created: usize,
    pub accounts_matched: usize,
    pub snapshots_imported: usize,
    pub errors: Vec<String>,
}

// ---------------------------------------------------------------------------
// Command
// ---------------------------------------------------------------------------

/// Bulk-import accounts and historical balance snapshots.
///
/// Idempotent: accounts are matched on (institution, name) case-insensitively and reused
/// if present, otherwise created. Snapshots upsert on (account_id, snapshot_date), so
/// re-importing the same data does not create duplicates.
#[tauri::command]
pub fn import_data(db: State<AppDb>, payload: ImportPayload) -> Result<ImportSummary, String> {
    let mut conn = db.0.lock().map_err(|e| e.to_string())?;
    run_import(&mut conn, payload).map_err(|e| e.to_string())
}

fn run_import(conn: &mut Connection, payload: ImportPayload) -> rusqlite::Result<ImportSummary> {
    let mut summary = ImportSummary::default();
    let tx = conn.transaction()?;

    for acc in payload.accounts {
        let name = acc.name.trim();
        let institution = acc.institution.trim();
        if name.is_empty() || institution.is_empty() {
            summary
                .errors
                .push("Skipped an account with empty name or institution.".to_string());
            continue;
        }

        // Reuse an existing account (case-insensitive) or create a new one.
        let existing: Option<i64> = tx
            .query_row(
                "SELECT id FROM accounts \
                 WHERE lower(institution) = lower(?1) AND lower(name) = lower(?2) \
                 LIMIT 1",
                params![institution, name],
                |r| r.get(0),
            )
            .optional()?;

        let account_id = match existing {
            Some(id) => {
                summary.accounts_matched += 1;
                id
            }
            None => {
                tx.execute(
                    "INSERT INTO accounts \
                     (name, institution, account_type, currency, jurisdiction, notes) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![
                        name,
                        institution,
                        acc.account_type,
                        acc.currency,
                        acc.jurisdiction,
                        acc.notes,
                    ],
                )?;
                summary.accounts_created += 1;
                tx.last_insert_rowid()
            }
        };

        // Snapshots take the account's authoritative currency.
        let currency: String =
            tx.query_row("SELECT currency FROM accounts WHERE id = ?1", params![account_id], |r| {
                r.get(0)
            })?;

        for snap in acc.snapshots {
            if snap.snapshot_date.trim().is_empty() {
                summary
                    .errors
                    .push(format!("Skipped a snapshot for '{name}' with an empty date."));
                continue;
            }
            tx.execute(
                "INSERT OR REPLACE INTO balance_snapshots \
                 (account_id, snapshot_date, balance, currency, source) \
                 VALUES (?1, ?2, ?3, ?4, 'import')",
                params![account_id, snap.snapshot_date.trim(), snap.balance, currency],
            )?;
            summary.snapshots_imported += 1;
        }
    }

    tx.commit()?;
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::apply_schema;

    fn payload() -> ImportPayload {
        ImportPayload {
            accounts: vec![ImportAccount {
                name: "Chequing".into(),
                institution: "Scotiabank".into(),
                account_type: "chequing".into(),
                currency: "CAD".into(),
                jurisdiction: "CA".into(),
                notes: None,
                snapshots: vec![
                    ImportSnapshot {
                        snapshot_date: "2025-01-01".into(),
                        balance: 1000.0,
                    },
                    ImportSnapshot {
                        snapshot_date: "2025-02-01".into(),
                        balance: 1500.0,
                    },
                ],
            }],
        }
    }

    #[test]
    fn import_creates_then_matches_idempotently() {
        let mut conn = Connection::open_in_memory().unwrap();
        apply_schema(&conn).unwrap();

        let first = run_import(&mut conn, payload()).unwrap();
        assert_eq!(first.accounts_created, 1);
        assert_eq!(first.accounts_matched, 0);
        assert_eq!(first.snapshots_imported, 2);

        // Re-importing the same data must not duplicate the account or snapshots.
        let second = run_import(&mut conn, payload()).unwrap();
        assert_eq!(second.accounts_created, 0);
        assert_eq!(second.accounts_matched, 1);

        let accounts: i64 = conn
            .query_row("SELECT count(*) FROM accounts", [], |r| r.get(0))
            .unwrap();
        let snapshots: i64 = conn
            .query_row("SELECT count(*) FROM balance_snapshots", [], |r| r.get(0))
            .unwrap();
        assert_eq!(accounts, 1);
        assert_eq!(snapshots, 2);
    }

    #[test]
    fn import_skips_empty_account_names() {
        let mut conn = Connection::open_in_memory().unwrap();
        apply_schema(&conn).unwrap();

        let bad = ImportPayload {
            accounts: vec![ImportAccount {
                name: "  ".into(),
                institution: "Bank".into(),
                account_type: "savings".into(),
                currency: "USD".into(),
                jurisdiction: "US".into(),
                notes: None,
                snapshots: vec![],
            }],
        };
        let summary = run_import(&mut conn, bad).unwrap();
        assert_eq!(summary.accounts_created, 0);
        assert_eq!(summary.errors.len(), 1);
    }
}
