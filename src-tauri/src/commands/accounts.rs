use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::db::AppDb;

// ---------------------------------------------------------------------------
// Serialisable types returned to the frontend
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct AccountRow {
    pub id: i64,
    pub name: String,
    pub institution: String,
    pub account_type: String,
    pub currency: String,
    pub jurisdiction: String,
    pub connector_kind: String,
    pub connector_ref: Option<String>,
    pub is_active: bool,
    pub notes: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub latest_balance: Option<f64>,
    pub latest_balance_date: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct AddAccountPayload {
    pub name: String,
    pub institution: String,
    pub account_type: String,
    pub currency: String,
    pub jurisdiction: String,
    pub notes: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct BalanceSnapshotRow {
    pub id: i64,
    pub account_id: i64,
    pub snapshot_date: String,
    pub balance: f64,
    pub currency: String,
}

#[derive(Debug, Deserialize)]
pub struct AddBalanceSnapshotPayload {
    pub account_id: i64,
    pub balance: f64,
    pub snapshot_date: String,
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

/// List all active accounts with their latest balance snapshot joined in.
#[tauri::command]
pub fn list_accounts(db: State<AppDb>) -> Result<Vec<AccountRow>, String> {
    let conn = db.0.lock().map_err(|e| e.to_string())?;

    let mut stmt = conn
        .prepare(
            r#"
            SELECT
                a.id, a.name, a.institution, a.account_type,
                a.currency, a.jurisdiction, a.connector_kind, a.connector_ref,
                a.is_active, a.notes, a.created_at, a.updated_at,
                bs.balance        AS latest_balance,
                bs.snapshot_date  AS latest_balance_date
            FROM accounts a
            LEFT JOIN balance_snapshots bs ON bs.id = (
                SELECT id FROM balance_snapshots
                WHERE account_id = a.id
                ORDER BY snapshot_date DESC
                LIMIT 1
            )
            WHERE a.is_active = 1
            ORDER BY a.institution, a.name
            "#,
        )
        .map_err(|e| e.to_string())?;

    let rows = stmt
        .query_map([], |r| {
            Ok(AccountRow {
                id: r.get(0)?,
                name: r.get(1)?,
                institution: r.get(2)?,
                account_type: r.get(3)?,
                currency: r.get(4)?,
                jurisdiction: r.get(5)?,
                connector_kind: r.get(6)?,
                connector_ref: r.get(7)?,
                is_active: r.get::<_, i64>(8)? != 0,
                notes: r.get(9)?,
                created_at: r.get(10)?,
                updated_at: r.get(11)?,
                latest_balance: r.get(12)?,
                latest_balance_date: r.get(13)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    Ok(rows)
}

/// Add a new manually-managed account.
#[tauri::command]
pub fn add_account(
    db: State<AppDb>,
    payload: AddAccountPayload,
) -> Result<AccountRow, String> {
    if payload.name.trim().is_empty() {
        return Err("Account name cannot be empty.".into());
    }
    if payload.institution.trim().is_empty() {
        return Err("Institution cannot be empty.".into());
    }

    let conn = db.0.lock().map_err(|e| e.to_string())?;

    conn.execute(
        "INSERT INTO accounts (name, institution, account_type, currency, jurisdiction, notes) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            payload.name.trim(),
            payload.institution.trim(),
            payload.account_type,
            payload.currency,
            payload.jurisdiction,
            payload.notes,
        ],
    )
    .map_err(|e| e.to_string())?;

    let id = conn.last_insert_rowid();

    let row = conn
        .query_row(
            "SELECT id, name, institution, account_type, currency, jurisdiction, \
             connector_kind, connector_ref, is_active, notes, created_at, updated_at \
             FROM accounts WHERE id = ?1",
            params![id],
            |r| {
                Ok(AccountRow {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    institution: r.get(2)?,
                    account_type: r.get(3)?,
                    currency: r.get(4)?,
                    jurisdiction: r.get(5)?,
                    connector_kind: r.get(6)?,
                    connector_ref: r.get(7)?,
                    is_active: r.get::<_, i64>(8)? != 0,
                    notes: r.get(9)?,
                    created_at: r.get(10)?,
                    updated_at: r.get(11)?,
                    latest_balance: None,
                    latest_balance_date: None,
                })
            },
        )
        .map_err(|e| e.to_string())?;

    Ok(row)
}

/// Soft-delete an account (set is_active = 0).
#[tauri::command]
pub fn delete_account(db: State<AppDb>, account_id: i64) -> Result<(), String> {
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    conn.execute(
        "UPDATE accounts SET is_active = 0, updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now') \
         WHERE id = ?1",
        params![account_id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Upsert a balance snapshot for an account.
#[tauri::command]
pub fn add_balance_snapshot(
    db: State<AppDb>,
    payload: AddBalanceSnapshotPayload,
) -> Result<BalanceSnapshotRow, String> {
    let conn = db.0.lock().map_err(|e| e.to_string())?;

    // Look up the account's currency to store alongside the snapshot.
    let currency: String = conn
        .query_row(
            "SELECT currency FROM accounts WHERE id = ?1 AND is_active = 1",
            params![payload.account_id],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Account {} not found.", payload.account_id))?;

    conn.execute(
        "INSERT OR REPLACE INTO balance_snapshots \
         (account_id, snapshot_date, balance, currency, source) \
         VALUES (?1, ?2, ?3, ?4, 'manual')",
        params![payload.account_id, payload.snapshot_date, payload.balance, currency],
    )
    .map_err(|e| e.to_string())?;

    let id = conn.last_insert_rowid();

    Ok(BalanceSnapshotRow {
        id,
        account_id: payload.account_id,
        snapshot_date: payload.snapshot_date,
        balance: payload.balance,
        currency,
    })
}
