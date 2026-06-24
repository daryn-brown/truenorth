//! AI "second brain" Tauri commands: provider settings, token management, model listing, and the
//! grounded chat that answers questions over the user's own financial data.
//!
//! `ai_chat`/`ai_list_models` are async (they call a remote or local model). As elsewhere in the
//! app, the SQLite mutex is never held across an `.await`: the financial snapshot is gathered first
//! (each helper locks briefly and releases), then the model call runs with no lock held.

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::ai::{self, ChatMessage};
use crate::commands::cashflow::{get_cashflow_summary, list_recent_transactions};
use crate::commands::goals::get_goal_progress;
use crate::commands::net_worth::get_net_worth;
use crate::db::secrets::{self, GITHUB_MODELS_TOKEN};
use crate::db::AppDb;

// Non-secret settings live in app_settings; the GitHub token is a secret in the secret store.
const SETTING_PROVIDER: &str = "ai_provider";
const SETTING_GITHUB_MODEL: &str = "ai_github_model";
const SETTING_OLLAMA_MODEL: &str = "ai_ollama_model";
const SETTING_OLLAMA_URL: &str = "ai_ollama_url";
const SETTING_INCLUDE_REAL_DATA: &str = "ai_include_real_data";

/// Instructions prepended to every conversation, ahead of the live financial snapshot.
const SYSTEM_PREAMBLE: &str = "You are TrueNorth's built-in financial advisor — a knowledgeable, \
candid assistant embedded in the user's local, cross-border (US + Canada) personal-finance app. \
Answer questions using the financial snapshot below, which comes from the user's own private \
database.\n\
Guidelines:\n\
- Ground every claim in the snapshot. If the data needed to answer isn't present, say so plainly \
and suggest what to add, import, or sync.\n\
- Do the math carefully and show the key numbers you used. Always state the currency — this user \
holds both USD and CAD.\n\
- Be concise and practical: clear observations, comparisons, and tradeoffs, not generic boilerplate.\n\
- You are an educational tool, not a licensed financial or tax advisor; note significant caveats \
briefly when they matter, without disclaiming every sentence.\n\
- Never invent balances, transactions, or accounts that are not in the snapshot.";

// ---------------------------------------------------------------------------
// app_settings helpers
// ---------------------------------------------------------------------------

fn get_setting(conn: &Connection, key: &str) -> rusqlite::Result<Option<String>> {
    conn.query_row(
        "SELECT value FROM app_settings WHERE key = ?1",
        params![key],
        |r| r.get(0),
    )
    .optional()
}

fn set_setting(conn: &Connection, key: &str, value: &str) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO app_settings (key, value, updated_at) \
         VALUES (?1, ?2, strftime('%Y-%m-%dT%H:%M:%SZ', 'now')) \
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
        params![key, value],
    )?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Serialisable types returned to / accepted from the frontend
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct AiSettings {
    /// "github" (GitHub Models) or "ollama" (local).
    pub provider: String,
    pub github_model: String,
    pub ollama_model: String,
    pub ollama_url: String,
    /// When true, exact balances/transactions are sent to the model; when false, only rounded
    /// aggregates are sent (privacy mode for the free GitHub tier).
    pub include_real_data: bool,
    /// Whether a GitHub Models token is stored (the token itself is never returned).
    pub has_github_token: bool,
}

#[derive(Debug, Deserialize)]
pub struct AiSettingsInput {
    pub provider: String,
    pub github_model: String,
    pub ollama_model: String,
    pub ollama_url: String,
    pub include_real_data: bool,
}

/// Read the five settings, applying defaults for anything not yet stored.
fn read_settings(conn: &Connection) -> rusqlite::Result<(String, String, String, String, bool)> {
    let provider = get_setting(conn, SETTING_PROVIDER)?.unwrap_or_else(|| "github".into());
    let github_model =
        get_setting(conn, SETTING_GITHUB_MODEL)?.unwrap_or_else(|| ai::DEFAULT_GITHUB_MODEL.into());
    let ollama_model =
        get_setting(conn, SETTING_OLLAMA_MODEL)?.unwrap_or_else(|| ai::DEFAULT_OLLAMA_MODEL.into());
    let ollama_url =
        get_setting(conn, SETTING_OLLAMA_URL)?.unwrap_or_else(|| ai::OLLAMA_DEFAULT_BASE.into());
    // Default to sending real data — the user chose this for the best answers.
    let include_real_data = get_setting(conn, SETTING_INCLUDE_REAL_DATA)?
        .map(|v| v != "0")
        .unwrap_or(true);
    Ok((provider, github_model, ollama_model, ollama_url, include_real_data))
}

#[tauri::command]
pub fn ai_get_settings(db: State<AppDb>) -> Result<AiSettings, String> {
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    let (provider, github_model, ollama_model, ollama_url, include_real_data) =
        read_settings(&conn).map_err(|e| e.to_string())?;
    let has_github_token = secrets::get_secret(GITHUB_MODELS_TOKEN)
        .map_err(|e| e.to_string())?
        .map(|t| !t.trim().is_empty())
        .unwrap_or(false);
    Ok(AiSettings {
        provider,
        github_model,
        ollama_model,
        ollama_url,
        include_real_data,
        has_github_token,
    })
}

#[tauri::command]
pub fn ai_save_settings(db: State<AppDb>, settings: AiSettingsInput) -> Result<AiSettings, String> {
    let provider = if settings.provider == "ollama" { "ollama" } else { "github" };
    {
        let conn = db.0.lock().map_err(|e| e.to_string())?;
        set_setting(&conn, SETTING_PROVIDER, provider).map_err(|e| e.to_string())?;

        let github_model = settings.github_model.trim();
        set_setting(
            &conn,
            SETTING_GITHUB_MODEL,
            if github_model.is_empty() { ai::DEFAULT_GITHUB_MODEL } else { github_model },
        )
        .map_err(|e| e.to_string())?;

        let ollama_model = settings.ollama_model.trim();
        set_setting(
            &conn,
            SETTING_OLLAMA_MODEL,
            if ollama_model.is_empty() { ai::DEFAULT_OLLAMA_MODEL } else { ollama_model },
        )
        .map_err(|e| e.to_string())?;

        let ollama_url = settings.ollama_url.trim();
        set_setting(
            &conn,
            SETTING_OLLAMA_URL,
            if ollama_url.is_empty() { ai::OLLAMA_DEFAULT_BASE } else { ollama_url },
        )
        .map_err(|e| e.to_string())?;

        set_setting(
            &conn,
            SETTING_INCLUDE_REAL_DATA,
            if settings.include_real_data { "1" } else { "0" },
        )
        .map_err(|e| e.to_string())?;
    }
    ai_get_settings(db)
}

/// Store (or, with an empty string, clear) the GitHub Models token. Returns whether a token is now
/// stored. The token is held in the secret store, never returned to the frontend.
#[tauri::command]
pub fn ai_set_github_token(token: String) -> Result<bool, String> {
    let trimmed = token.trim();
    if trimmed.is_empty() {
        secrets::delete_secret(GITHUB_MODELS_TOKEN).map_err(|e| e.to_string())?;
        Ok(false)
    } else {
        secrets::set_secret(GITHUB_MODELS_TOKEN, trimmed).map_err(|e| e.to_string())?;
        Ok(true)
    }
}

#[tauri::command]
pub async fn ai_list_models(db: State<'_, AppDb>) -> Result<Vec<ai::ModelInfo>, String> {
    let (provider, ollama_url, token) = {
        let conn = db.0.lock().map_err(|e| e.to_string())?;
        let provider =
            get_setting(&conn, SETTING_PROVIDER).map_err(|e| e.to_string())?.unwrap_or_else(|| "github".into());
        let ollama_url = get_setting(&conn, SETTING_OLLAMA_URL)
            .map_err(|e| e.to_string())?
            .unwrap_or_else(|| ai::OLLAMA_DEFAULT_BASE.into());
        let token = secrets::get_secret(GITHUB_MODELS_TOKEN).map_err(|e| e.to_string())?;
        (provider, ollama_url, token)
    };

    if provider == "ollama" {
        ai::list_ollama_models(&ollama_url).await.map_err(|e| e.to_string())
    } else {
        let token = token
            .filter(|t| !t.trim().is_empty())
            .ok_or("Add a GitHub token first to list models.")?;
        ai::list_github_models(&token).await.map_err(|e| e.to_string())
    }
}

#[tauri::command]
pub async fn ai_chat(
    db: State<'_, AppDb>,
    messages: Vec<ChatMessage>,
) -> Result<ChatMessage, String> {
    // 1) Resolve provider config + token under a short lock.
    let (provider, github_model, ollama_model, ollama_url, include_real_data) = {
        let conn = db.0.lock().map_err(|e| e.to_string())?;
        read_settings(&conn).map_err(|e| e.to_string())?
    };
    let token = secrets::get_secret(GITHUB_MODELS_TOKEN).map_err(|e| e.to_string())?;

    // 2) Build the grounding context from the user's own data (each helper locks briefly).
    let context = build_context(&db, include_real_data)?;

    // 3) Compose: system grounding first, then the conversation so far (drop any client-sent
    //    system messages so the snapshot can't be overridden).
    let mut full = Vec::with_capacity(messages.len() + 1);
    full.push(ChatMessage::system(context));
    full.extend(messages.into_iter().filter(|m| m.role != "system"));

    // 4) Dispatch to the chosen provider. No DB lock is held across this await.
    let answer = if provider == "ollama" {
        ai::chat_completion(&ollama_url, None, &ollama_model, &full)
            .await
            .map_err(|e| e.to_string())?
    } else {
        let token = token.filter(|t| !t.trim().is_empty()).ok_or(
            "Add a GitHub token (with the models:read scope) in AI settings first, or switch to Ollama.",
        )?;
        ai::chat_completion(ai::GITHUB_MODELS_BASE, Some(&token), &github_model, &full)
            .await
            .map_err(|e| e.to_string())?
    };

    Ok(ChatMessage { role: "assistant".into(), content: answer })
}

// ---------------------------------------------------------------------------
// Financial context ("RAG over your own SQLite")
// ---------------------------------------------------------------------------

struct HoldingRow {
    account: String,
    symbol: String,
    quantity: f64,
    average_cost: Option<f64>,
    currency: String,
    last_price: Option<f64>,
}

fn load_holdings(db: &State<AppDb>) -> Result<Vec<HoldingRow>, String> {
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare(
            "SELECT a.name, h.symbol, h.quantity, h.average_cost, h.currency, h.last_price \
             FROM holdings h JOIN accounts a ON a.id = h.account_id \
             WHERE a.is_active = 1 \
             ORDER BY a.name, h.symbol \
             LIMIT 100",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| {
            Ok(HoldingRow {
                account: r.get(0)?,
                symbol: r.get(1)?,
                quantity: r.get(2)?,
                average_cost: r.get(3)?,
                currency: r.get(4)?,
                last_price: r.get(5)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    Ok(rows)
}

/// Round to the nearest $1,000 for the privacy-mode aggregate view.
fn round_thousands(value: f64) -> f64 {
    (value / 1000.0).round() * 1000.0
}

/// Assemble the system message: instructions + a snapshot of the user's finances. With
/// `include_real`, exact balances/holdings/transactions are included; otherwise only rounded
/// aggregates (so the free GitHub tier never sees exact figures).
fn build_context(db: &State<AppDb>, include_real: bool) -> Result<String, String> {
    let net_worth = get_net_worth(db.clone())?;
    let cashflow = get_cashflow_summary(db.clone(), None)?;
    let goal = get_goal_progress(db.clone())?;

    let mut s = String::with_capacity(2048);
    s.push_str(SYSTEM_PREAMBLE);
    s.push_str("\n\n===== FINANCIAL SNAPSHOT (the user's own data) =====\n");

    if include_real {
        match net_worth.rate_date {
            Some(ref d) => s.push_str(&format!(
                "\nNet worth: USD ${:.2} / CAD ${:.2} (FX as of {})\n",
                net_worth.total_usd, net_worth.total_cad, d
            )),
            None => s.push_str(&format!(
                "\nNet worth: USD ${:.2} / CAD ${:.2} (no FX rate stored)\n",
                net_worth.total_usd, net_worth.total_cad
            )),
        }

        s.push_str("Accounts:\n");
        if net_worth.accounts.is_empty() {
            s.push_str("  (none yet)\n");
        }
        for a in &net_worth.accounts {
            let asof = a.snapshot_date.as_deref().unwrap_or("no snapshot");
            s.push_str(&format!(
                "  - {} [{}, {}, {}]: {:.2} {} (≈ USD ${:.2} / CAD ${:.2}; as of {})\n",
                a.account_name,
                a.institution,
                a.account_type,
                a.jurisdiction,
                a.balance,
                a.currency,
                a.balance_usd,
                a.balance_cad,
                asof
            ));
        }

        s.push_str(&format!(
            "\nCashflow (last {} days, since {}): income USD ${:.2}, fixed USD ${:.2}, \
             variable USD ${:.2}, net savings USD ${:.2}, savings rate {:.0}%\n",
            cashflow.window_days,
            cashflow.since,
            cashflow.income.usd,
            cashflow.fixed.usd,
            cashflow.variable.usd,
            cashflow.net_savings.usd,
            cashflow.savings_rate * 100.0
        ));

        s.push_str(&format!(
            "Goal: target USD ${:.0}, current USD ${:.2} ({:.0}% there, gap USD ${:.2}){}\n",
            goal.target_usd,
            goal.current_usd,
            goal.progress * 100.0,
            goal.gap_usd,
            match goal.projected_date {
                Some(ref d) => format!(", projected {d}"),
                None => String::new(),
            }
        ));

        let holdings = load_holdings(db)?;
        if !holdings.is_empty() {
            s.push_str("\nHoldings:\n");
            for h in &holdings {
                let avg = h
                    .average_cost
                    .map(|c| format!("{c:.2}"))
                    .unwrap_or_else(|| "—".into());
                let last = h
                    .last_price
                    .map(|c| format!("{c:.2}"))
                    .unwrap_or_else(|| "—".into());
                s.push_str(&format!(
                    "  - {}: {:.4} {} (last {} {}, avg cost {})\n",
                    h.account, h.quantity, h.symbol, last, h.currency, avg
                ));
            }
        }

        let txns = list_recent_transactions(db.clone(), Some(40))?;
        if !txns.is_empty() {
            s.push_str(&format!("\nRecent transactions (latest {}):\n", txns.len()));
            for t in &txns {
                s.push_str(&format!(
                    "  - {} | {} | {:.2} {} | {}\n",
                    t.txn_date, t.description, t.amount, t.currency, t.flow_type
                ));
            }
        }
    } else {
        // Privacy mode: rounded aggregates only — no exact balances, holdings, or transactions.
        s.push_str(&format!(
            "\nNet worth: about USD ${:.0} (rounded to the nearest $1,000)\n",
            round_thousands(net_worth.total_usd)
        ));
        s.push_str(&format!("Active accounts: {}\n", net_worth.accounts.len()));
        s.push_str(&format!(
            "Savings rate (last {} days): {:.0}%\n",
            cashflow.window_days,
            cashflow.savings_rate * 100.0
        ));
        s.push_str(&format!(
            "Goal progress: {:.0}% toward the USD ${:.0} milestone\n",
            goal.progress * 100.0,
            goal.target_usd
        ));
        s.push_str("(Privacy mode: exact balances, holdings, and transactions are withheld.)\n");
    }

    s.push_str("\n===== END SNAPSHOT =====\n");
    Ok(s)
}
