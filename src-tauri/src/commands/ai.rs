//! AI "second brain" Tauri commands: provider settings, token management, model listing, and the
//! grounded chat that answers questions over the user's own financial data.
//!
//! `ai_chat`/`ai_list_models` are async (they call a remote or local model). As elsewhere in the
//! app, the SQLite mutex is never held across an `.await`: the financial snapshot is gathered first
//! (each helper locks briefly and releases), then the model call runs with no lock held.

use chrono::NaiveDate;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use tauri::State;

use crate::ai::{self, ChatMessage, ToolDef, WireMessage};
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
- Transactions carry a flow (income / fixed / variable / transfer) and a best-guess spending \
category (Groceries, Dining, Transport, etc.). Transfers are internal moves — money sent to the \
user's own accounts, a brokerage or exchange, or a credit-card payment — and are NOT spending; \
never count them as variable spending or income.\n\
- When asked where money goes, use the variable-spending-by-category breakdown and the merchant \
names in the transaction list; infer the likely purpose of a purchase from the merchant.\n\
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

/// Locate the GitHub CLI (`gh`). GUI apps on macOS launch with a minimal PATH that usually omits
/// Homebrew, so check the common install locations before falling back to a PATH lookup.
fn find_gh() -> String {
    if let Ok(p) = std::env::var("GH_PATH") {
        if !p.trim().is_empty() {
            return p;
        }
    }
    for candidate in [
        "/opt/homebrew/bin/gh", // Apple Silicon Homebrew
        "/usr/local/bin/gh",    // Intel Homebrew
        "/opt/local/bin/gh",    // MacPorts
        "/usr/bin/gh",
    ] {
        if std::path::Path::new(candidate).exists() {
            return candidate.to_string();
        }
    }
    "gh".to_string() // last resort: let the OS resolve it via PATH
}

/// Read a token from the user's GitHub CLI session (`gh auth token`).
fn read_github_cli_token() -> Result<String, String> {
    let gh = find_gh();
    let output = std::process::Command::new(&gh)
        .args(["auth", "token"])
        .output()
        .map_err(|e| {
            format!(
                "Couldn't run the GitHub CLI ({gh}): {e}. Install it from https://cli.github.com, \
                 run `gh auth login`, then try again."
            )
        })?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "The GitHub CLI couldn't provide a token. Run `gh auth login` in a terminal, then try \
             again. {}",
            stderr.trim()
        ));
    }
    let token = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if token.is_empty() {
        return Err("The GitHub CLI returned an empty token. Run `gh auth login` and try again.".into());
    }
    Ok(token)
}

/// Pull a token from the local GitHub CLI (`gh auth token`) and store it, so the user never has to
/// create or paste a personal access token. Returns the updated settings (with `has_github_token`).
#[tauri::command]
pub fn ai_github_cli_login(db: State<AppDb>) -> Result<AiSettings, String> {
    let token = read_github_cli_token()?;
    secrets::set_secret(GITHUB_MODELS_TOKEN, token.trim()).map_err(|e| e.to_string())?;
    ai_get_settings(db)
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

/// One tool the model invoked while answering, surfaced to the UI as a "Used tool: …" step.
#[derive(Debug, Serialize)]
pub struct ToolStep {
    pub name: String,
    /// Raw JSON arguments the model passed.
    pub arguments: String,
    /// JSON result returned to the model (truncated for display).
    pub result: String,
}

/// The advisor's reply plus the trace of tools it called to get there.
#[derive(Debug, Serialize)]
pub struct AiChatResponse {
    pub role: String,
    pub content: String,
    pub steps: Vec<ToolStep>,
}

/// Hard cap on agentic round-trips, so a confused model can't loop forever on tool calls.
const MAX_TOOL_ITERATIONS: usize = 6;

#[tauri::command]
pub async fn ai_chat(
    db: State<'_, AppDb>,
    messages: Vec<ChatMessage>,
) -> Result<AiChatResponse, String> {
    // 1) Resolve provider config + token under a short lock.
    let (provider, github_model, ollama_model, ollama_url, include_real_data) = {
        let conn = db.0.lock().map_err(|e| e.to_string())?;
        read_settings(&conn).map_err(|e| e.to_string())?
    };
    let token = secrets::get_secret(GITHUB_MODELS_TOKEN).map_err(|e| e.to_string())?;

    // Resolve the endpoint, key, and model for the chosen provider once.
    let (base_url, api_key, model) = if provider == "ollama" {
        (ollama_url, None, ollama_model)
    } else {
        let token = token.filter(|t| !t.trim().is_empty()).ok_or(
            "Add a GitHub token (with the models:read scope) in AI settings first, or switch to Ollama.",
        )?;
        (ai::GITHUB_MODELS_BASE.to_string(), Some(token), github_model)
    };

    // 2) Privacy mode: never expose tools (they return exact figures). Fall back to the static,
    //    rounded snapshot so only aggregates leave the device — same guarantee as before.
    if !include_real_data {
        let context = build_context(&db, false)?;
        let mut convo = vec![WireMessage::system(context)];
        convo.extend(messages.iter().filter(|m| m.role != "system").map(WireMessage::from));
        let reply = ai::chat_completion_tools(&base_url, api_key.as_deref(), &model, &convo, &[])
            .await
            .map_err(|e| e.to_string())?;
        let content = reply.content.unwrap_or_default();
        if content.trim().is_empty() {
            return Err("The model returned an empty response.".into());
        }
        return Ok(AiChatResponse { role: "assistant".into(), content, steps: vec![] });
    }

    // 3) Agentic mode: advertise the finance tools and let the model query its own data on demand.
    let system = build_system_preamble(&db)?;
    let tools = finance_tools();
    let mut convo = vec![WireMessage::system(system)];
    convo.extend(messages.iter().filter(|m| m.role != "system").map(WireMessage::from));

    let mut steps: Vec<ToolStep> = Vec::new();

    // The loop: ask the model; if it requested tools, run them (each locks the DB briefly, with no
    // lock held across an await), append the results, and ask again — until it returns prose.
    for _ in 0..MAX_TOOL_ITERATIONS {
        let assistant =
            ai::chat_completion_tools(&base_url, api_key.as_deref(), &model, &convo, &tools)
                .await
                .map_err(|e| e.to_string())?;

        let calls = assistant.tool_calls.clone().unwrap_or_default();
        if calls.is_empty() {
            let content = assistant.content.unwrap_or_default();
            if content.trim().is_empty() {
                return Err("The model returned an empty response.".into());
            }
            return Ok(AiChatResponse { role: "assistant".into(), content, steps });
        }

        // Echo the assistant's tool-call turn back, then answer each call with a `tool` message.
        convo.push(assistant);
        for call in calls {
            let args: serde_json::Value =
                serde_json::from_str(&call.function.arguments).unwrap_or_else(|_| serde_json::json!({}));
            let result = execute_finance_tool(&db, &call.function.name, &args)
                .unwrap_or_else(|e| serde_json::json!({ "error": e }));
            let result_str = result.to_string();
            steps.push(ToolStep {
                name: call.function.name.clone(),
                arguments: call.function.arguments.clone(),
                result: truncate_for_display(&result_str, 6000),
            });
            convo.push(WireMessage::tool_result(call.id, call.function.name, result_str));
        }
    }

    // Hit the iteration cap — make one last call with no tools to force a written answer.
    let final_msg = ai::chat_completion_tools(&base_url, api_key.as_deref(), &model, &convo, &[])
        .await
        .map_err(|e| e.to_string())?;
    let content = final_msg.content.unwrap_or_default();
    if content.trim().is_empty() {
        return Err("The model kept calling tools without answering. Try rephrasing.".into());
    }
    Ok(AiChatResponse { role: "assistant".into(), content, steps })
}

/// Cap a tool result for the UI trace without splitting a UTF-8 char boundary.
fn truncate_for_display(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}… (truncated)", &s[..end])
}

// ---------------------------------------------------------------------------
// Agentic mode: system preamble, the finance tool catalog, and the executor that runs each tool
// over the user's own local database.
// ---------------------------------------------------------------------------

/// System message for agentic mode: the standing instructions, a tiny grounding header (date, home
/// currency, account count), and a nudge to gather real figures via tools before answering.
fn build_system_preamble(db: &State<AppDb>) -> Result<String, String> {
    let (today, home_currency, active_accounts) = {
        let conn = db.0.lock().map_err(|e| e.to_string())?;
        let today: String = conn
            .query_row("SELECT strftime('%Y-%m-%d','now')", [], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        let home_currency = get_setting(&conn, "home_currency")
            .map_err(|e| e.to_string())?
            .unwrap_or_else(|| "CAD".into());
        let active_accounts: i64 = conn
            .query_row("SELECT COUNT(*) FROM accounts WHERE is_active = 1", [], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        (today, home_currency, active_accounts)
    };
    Ok(format!(
        "{SYSTEM_PREAMBLE}\n\n\
         ===== HOW TO ANSWER =====\n\
         You can call tools that query the user's own local TrueNorth database. Call whatever tools \
         you need to gather the real figures before answering — never guess at balances, spending, \
         holdings, or debts you can look up. For broad questions (e.g. \"how's my money management?\") \
         combine several tools: cashflow + recurring charges + liabilities + holdings + the goal.\n\
         Today is {today}. The user's home currency is {home_currency}. They have {active_accounts} \
         active account(s). Always label amounts with their currency (USD or CAD).\n\
         Format the final answer as clean GitHub-flavored markdown: short **bold** section headers, \
         tight bullet or numbered lists, and key numbers in bold. Be specific, candid, and practical."
    ))
}

/// The catalog of tools advertised to the model. Schemas are JSON-Schema objects.
fn finance_tools() -> Vec<ToolDef> {
    let no_params = || json!({ "type": "object", "properties": {}, "additionalProperties": false });
    vec![
        ToolDef::function(
            "get_net_worth_summary",
            "Total net worth in USD and CAD, the FX rate date, and the number of active accounts. Use for headline totals.",
            no_params(),
        ),
        ToolDef::function(
            "list_accounts",
            "Every active account with its institution, type, jurisdiction, currency, and current balance (native, USD, and CAD).",
            no_params(),
        ),
        ToolDef::function(
            "get_cashflow",
            "Income vs. fixed vs. variable spending, net savings, savings rate, and variable spending by category over a trailing window. Internal transfers are excluded.",
            json!({
                "type": "object",
                "properties": {
                    "window_days": { "type": "integer", "description": "Trailing window in days (default 30).", "minimum": 1, "maximum": 730 }
                },
                "additionalProperties": false
            }),
        ),
        ToolDef::function(
            "list_transactions",
            "Recent transactions with their flow (income/fixed/variable/transfer), category, account, amount, and currency. Optionally filter by a merchant substring or a flow type.",
            json!({
                "type": "object",
                "properties": {
                    "limit": { "type": "integer", "description": "Max rows (default 40, max 100).", "minimum": 1, "maximum": 100 },
                    "search": { "type": "string", "description": "Case-insensitive substring to match in the description/merchant." },
                    "flow": { "type": "string", "enum": ["income", "fixed", "variable", "transfer"], "description": "Only return this flow type." }
                },
                "additionalProperties": false
            }),
        ),
        ToolDef::function(
            "find_recurring_transactions",
            "Detect likely recurring charges, subscriptions, and bills (and recurring income) by grouping similar merchants over a window. Returns merchant, occurrences, average amount, currency, and rough cadence in days.",
            json!({
                "type": "object",
                "properties": {
                    "window_days": { "type": "integer", "description": "Trailing window in days (default 120).", "minimum": 30, "maximum": 730 }
                },
                "additionalProperties": false
            }),
        ),
        ToolDef::function(
            "get_liabilities",
            "Credit-card, loan, and other debt accounts with their balances. Negative balances are amounts owed. Credit limits are not tracked, so exact utilization can't be computed.",
            no_params(),
        ),
        ToolDef::function(
            "get_holdings",
            "Investment holdings across brokerage accounts: symbol, quantity, average cost, last price, currency, and estimated market value.",
            no_params(),
        ),
        ToolDef::function(
            "get_goal",
            "Progress toward the user's net-worth milestone: target, current value, gap, percent complete, and projected hit-date.",
            no_params(),
        ),
    ]
}

/// Run one finance tool by name, returning a compact JSON value to feed back to the model. Each
/// helper locks the DB only for its own query — never across an `.await` in the agentic loop.
fn execute_finance_tool(
    db: &State<AppDb>,
    name: &str,
    args: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    match name {
        "get_net_worth_summary" => {
            let nw = get_net_worth(db.clone())?;
            Ok(json!({
                "total_usd": round2(nw.total_usd),
                "total_cad": round2(nw.total_cad),
                "fx_rate_date": nw.rate_date,
                "active_accounts": nw.accounts.len(),
            }))
        }
        "list_accounts" => {
            let nw = get_net_worth(db.clone())?;
            let accounts: Vec<_> = nw
                .accounts
                .iter()
                .map(|a| {
                    json!({
                        "name": a.account_name,
                        "institution": a.institution,
                        "type": a.account_type,
                        "jurisdiction": a.jurisdiction,
                        "currency": a.currency,
                        "balance": round2(a.balance),
                        "balance_usd": round2(a.balance_usd),
                        "balance_cad": round2(a.balance_cad),
                        "as_of": a.snapshot_date,
                    })
                })
                .collect();
            Ok(json!({ "accounts": accounts }))
        }
        "get_cashflow" => {
            let window = args.get("window_days").and_then(|v| v.as_i64());
            let cf = get_cashflow_summary(db.clone(), window)?;
            serde_json::to_value(cf).map_err(|e| e.to_string())
        }
        "list_transactions" => {
            let limit = args
                .get("limit")
                .and_then(|v| v.as_i64())
                .unwrap_or(40)
                .clamp(1, 100);
            let search = args.get("search").and_then(|v| v.as_str()).map(str::to_lowercase);
            let flow = args.get("flow").and_then(|v| v.as_str()).map(str::to_lowercase);
            // Fetch extra rows so post-filtering can still reach `limit`.
            let fetch = if search.is_some() || flow.is_some() { 200 } else { limit };
            let mut txns = list_recent_transactions(db.clone(), Some(fetch))?;
            if let Some(q) = &search {
                txns.retain(|t| t.description.to_lowercase().contains(q));
            }
            if let Some(f) = &flow {
                txns.retain(|t| t.flow_type == *f);
            }
            let items: Vec<_> = txns
                .iter()
                .take(limit as usize)
                .map(|t| {
                    json!({
                        "date": t.txn_date,
                        "description": t.description,
                        "amount": round2(t.amount),
                        "currency": t.currency,
                        "flow": t.flow_type,
                        "category": t.category,
                        "account": t.account_name,
                    })
                })
                .collect();
            Ok(json!({ "count": items.len(), "transactions": items }))
        }
        "find_recurring_transactions" => {
            let window = args
                .get("window_days")
                .and_then(|v| v.as_i64())
                .unwrap_or(120)
                .clamp(30, 730);
            let groups = find_recurring(db, window)?;
            Ok(json!({ "window_days": window, "recurring": groups }))
        }
        "get_liabilities" => {
            let nw = get_net_worth(db.clone())?;
            let liabilities: Vec<_> = nw
                .accounts
                .iter()
                .filter(|a| is_liability(&a.account_type, a.balance))
                .map(|a| {
                    json!({
                        "name": a.account_name,
                        "institution": a.institution,
                        "type": a.account_type,
                        "currency": a.currency,
                        "balance": round2(a.balance),
                        "balance_usd": round2(a.balance_usd),
                        "balance_cad": round2(a.balance_cad),
                        "as_of": a.snapshot_date,
                    })
                })
                .collect();
            Ok(json!({
                "liabilities": liabilities,
                "note": "Negative balances are amounts owed. Credit limits are not tracked, so exact utilization can't be computed."
            }))
        }
        "get_holdings" => {
            let holdings = load_holdings(db)?;
            let items: Vec<_> = holdings
                .iter()
                .map(|h| {
                    let market_value = h.last_price.map(|p| round2(p * h.quantity));
                    json!({
                        "account": h.account,
                        "symbol": h.symbol,
                        "quantity": h.quantity,
                        "currency": h.currency,
                        "average_cost": h.average_cost,
                        "last_price": h.last_price,
                        "market_value": market_value,
                    })
                })
                .collect();
            Ok(json!({ "holdings": items }))
        }
        "get_goal" => {
            let goal = get_goal_progress(db.clone())?;
            serde_json::to_value(goal).map_err(|e| e.to_string())
        }
        other => Err(format!("Unknown tool: {other}")),
    }
}

/// True when an account represents money owed rather than money held: a credit/loan/mortgage type,
/// or simply a negative balance.
fn is_liability(account_type: &str, balance: f64) -> bool {
    let t = account_type.to_lowercase();
    t.contains("credit")
        || t.contains("loan")
        || t.contains("mortgage")
        || t.contains("line of credit")
        || t.contains("liability")
        || t.contains("debt")
        || balance < 0.0
}

/// One detected recurring merchant/charge.
#[derive(Debug, Serialize)]
struct RecurringGroup {
    merchant: String,
    occurrences: i64,
    avg_amount: f64,
    currency: String,
    first_date: String,
    last_date: String,
    est_cadence_days: Option<i64>,
}

/// Heuristically group recent transactions into recurring charges (subscriptions, rent, loan
/// payments, salary). Groups by a normalized merchant key and surfaces any merchant seen 2+ times,
/// with its average amount and rough cadence. Best-effort: it offers candidates for the model to
/// reason about, not a guaranteed subscription list.
fn find_recurring(db: &State<AppDb>, window_days: i64) -> Result<Vec<RecurringGroup>, String> {
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare(
            "SELECT t.txn_date, t.description, t.amount, t.currency \
             FROM transactions t JOIN accounts a ON a.id = t.account_id \
             WHERE a.is_active = 1 AND t.txn_date >= date('now', ?1) \
             ORDER BY t.txn_date ASC",
        )
        .map_err(|e| e.to_string())?;
    let offset = format!("-{window_days} days");
    let rows = stmt
        .query_map(params![offset], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, f64>(2)?,
                r.get::<_, String>(3)?,
            ))
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    struct Group {
        descriptions: Vec<String>,
        amounts: Vec<f64>,
        dates: Vec<String>,
        currency: String,
    }
    let mut map: HashMap<String, Group> = HashMap::new();
    for (date, description, amount, currency) in rows {
        let key = normalize_merchant(&description);
        if key.is_empty() {
            continue;
        }
        let g = map.entry(key).or_insert_with(|| Group {
            descriptions: Vec::new(),
            amounts: Vec::new(),
            dates: Vec::new(),
            currency: currency.clone(),
        });
        g.descriptions.push(description);
        g.amounts.push(amount);
        g.dates.push(date);
    }

    let mut groups: Vec<RecurringGroup> = map
        .into_values()
        .filter(|g| g.amounts.len() >= 2)
        .map(|g| {
            let n = g.amounts.len() as f64;
            let avg = g.amounts.iter().sum::<f64>() / n;
            let first = g.dates.iter().min().cloned().unwrap_or_default();
            let last = g.dates.iter().max().cloned().unwrap_or_default();
            let cadence = cadence_days(&first, &last, g.amounts.len());
            RecurringGroup {
                merchant: most_common(&g.descriptions),
                occurrences: g.amounts.len() as i64,
                avg_amount: round2(avg),
                currency: g.currency,
                first_date: first,
                last_date: last,
                est_cadence_days: cadence,
            }
        })
        .collect();

    // Biggest recurring commitments first (magnitude × frequency), capped to keep the result small.
    groups.sort_by(|a, b| {
        let wa = a.avg_amount.abs() * a.occurrences as f64;
        let wb = b.avg_amount.abs() * b.occurrences as f64;
        wb.partial_cmp(&wa).unwrap_or(std::cmp::Ordering::Equal)
    });
    groups.truncate(20);
    Ok(groups)
}

/// Reduce a description to a stable merchant key: lowercase, drop digits and symbols, keep the
/// first few words. "UNITED AIRLINES 0123" and "United Airlines #987" collapse to "united airlines".
fn normalize_merchant(description: &str) -> String {
    let cleaned: String = description
        .chars()
        .map(|c| {
            if c.is_ascii_alphabetic() || c.is_whitespace() {
                c.to_ascii_lowercase()
            } else {
                ' '
            }
        })
        .collect();
    cleaned.split_whitespace().take(3).collect::<Vec<_>>().join(" ")
}

/// The most frequent original description in a group (falls back to the first).
fn most_common(values: &[String]) -> String {
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for v in values {
        *counts.entry(v.as_str()).or_insert(0) += 1;
    }
    counts
        .into_iter()
        .max_by_key(|(_, n)| *n)
        .map(|(v, _)| v.to_string())
        .unwrap_or_default()
}

/// Average days between occurrences, from the first/last date and the count. `None` when the dates
/// don't parse or span no time. Tolerates a trailing time component by reading only `YYYY-MM-DD`.
fn cadence_days(first: &str, last: &str, count: usize) -> Option<i64> {
    if count < 2 {
        return None;
    }
    let parse = |s: &str| NaiveDate::parse_from_str(s.get(0..10).unwrap_or(s), "%Y-%m-%d").ok();
    let span = (parse(last)? - parse(first)?).num_days();
    if span <= 0 {
        return None;
    }
    Some(span / (count as i64 - 1))
}

fn round2(x: f64) -> f64 {
    (x * 100.0).round() / 100.0
}

/// Outcome of an AI "Refine categories" pass.
#[derive(Debug, Serialize)]
pub struct CategorizeResult {
    /// How many transactions had a category written.
    pub categorized: usize,
    /// How many were newly reclassified as internal transfers (only rows with no manual override).
    pub flagged_transfers: usize,
    /// How many transactions were sent to the model.
    pub considered: usize,
    /// The model that produced the labels (for the UI to show).
    pub model: String,
}

struct CatTxn {
    id: i64,
    description: String,
    amount: f64,
    currency: String,
}

#[derive(Deserialize)]
struct CatItem {
    id: i64,
    #[serde(default)]
    category: Option<String>,
    #[serde(default)]
    transfer: Option<bool>,
}

/// Extract the first top-level JSON array from a model reply, tolerating ```json fences or prose
/// around it.
fn extract_json_array(reply: &str) -> Option<&str> {
    let start = reply.find('[')?;
    let end = reply.rfind(']')?;
    if end > start {
        Some(&reply[start..=end])
    } else {
        None
    }
}

/// Ask the configured model to label recent transactions by spending category and flag any that
/// are really internal transfers (moves to the user's own accounts, a brokerage, or a card
/// payment). Results are stored on `transactions.category`; transfer flags are written to
/// `flow_override` only for rows the user hasn't already pinned, so a manual choice always wins.
#[tauri::command]
pub async fn ai_categorize_transactions(
    db: State<'_, AppDb>,
    limit: Option<i64>,
) -> Result<CategorizeResult, String> {
    let want = limit.unwrap_or(75).clamp(1, 200);

    // 1) Gather the most recent transactions under a short lock.
    let (provider, github_model, ollama_model, ollama_url, _include) = {
        let conn = db.0.lock().map_err(|e| e.to_string())?;
        read_settings(&conn).map_err(|e| e.to_string())?
    };
    let txns: Vec<CatTxn> = {
        let conn = db.0.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare(
                "SELECT t.id, t.description, t.amount, t.currency \
                 FROM transactions t JOIN accounts a ON a.id = t.account_id \
                 WHERE a.is_active = 1 \
                 ORDER BY t.txn_date DESC, t.id DESC LIMIT ?1",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![want], |r| {
                Ok(CatTxn {
                    id: r.get(0)?,
                    description: r.get(1)?,
                    amount: r.get(2)?,
                    currency: r.get(3)?,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        rows
    };
    if txns.is_empty() {
        return Err("No transactions to categorize yet. Import or sync some first.".into());
    }
    let considered = txns.len();

    // 2) Build the prompt. Constrain the model to the canonical category set so labels stay
    //    consistent with the local guesser.
    let allowed = crate::commands::cashflow::CATEGORIES.join(", ");
    let system = format!(
        "You label personal-finance transactions. For each transaction you are given, decide the \
single best spending category and whether it is actually an internal transfer (money moved to \
the user's OWN account, a brokerage/investment/crypto account, or a credit-card payment) rather \
than real spending.\n\
Allowed categories (use EXACTLY one of these spellings): {allowed}.\n\
Infer the purpose from the merchant name. Mark transfer=true for moves to brokerages/exchanges \
(Wealthsimple, Questrade, Robinhood, Schwab, Fidelity, Vanguard, Coinbase, etc.), account-to-\
account moves, and card payments; otherwise transfer=false.\n\
Reply with ONLY a JSON array, no prose, no code fences. Each element: \
{{\"id\": <number>, \"category\": \"<one allowed category>\", \"transfer\": <true|false>}}."
    );
    let mut user = String::with_capacity(txns.len() * 48);
    user.push_str("Transactions (id | description | amount currency):\n");
    for t in &txns {
        user.push_str(&format!(
            "{} | {} | {:.2} {}\n",
            t.id, t.description, t.amount, t.currency
        ));
    }

    let messages = vec![
        ChatMessage::system(system),
        ChatMessage { role: "user".into(), content: user },
    ];

    // 3) Call the model with no DB lock held.
    let reply = if provider == "ollama" {
        ai::chat_completion(&ollama_url, None, &ollama_model, &messages)
            .await
            .map_err(|e| e.to_string())?
    } else {
        let token = secrets::get_secret(GITHUB_MODELS_TOKEN)
            .map_err(|e| e.to_string())?
            .filter(|t| !t.trim().is_empty())
            .ok_or(
                "Add a GitHub token (with the models:read scope) in AI settings first, or switch to Ollama.",
            )?;
        ai::chat_completion(ai::GITHUB_MODELS_BASE, Some(&token), &github_model, &messages)
            .await
            .map_err(|e| e.to_string())?
    };

    // 4) Parse the JSON array, ignoring anything around it.
    let json = extract_json_array(&reply)
        .ok_or("The model did not return a JSON array of categories. Try again.")?;
    let items: Vec<CatItem> = serde_json::from_str(json)
        .map_err(|e| format!("Could not parse the model's category response: {e}"))?;

    // 5) Apply: store sanitized categories; flag transfers only where the user hasn't overridden.
    let known: std::collections::HashSet<i64> = txns.iter().map(|t| t.id).collect();
    let mut categorized = 0usize;
    let mut flagged_transfers = 0usize;
    {
        let conn = db.0.lock().map_err(|e| e.to_string())?;
        for item in &items {
            if !known.contains(&item.id) {
                continue;
            }
            if let Some(cat) = item
                .category
                .as_deref()
                .and_then(crate::commands::cashflow::canonical_category)
            {
                let n = conn
                    .execute(
                        "UPDATE transactions SET category = ?1 WHERE id = ?2",
                        params![cat, item.id],
                    )
                    .map_err(|e| e.to_string())?;
                categorized += n;
            }
            if item.transfer == Some(true) {
                let n = conn
                    .execute(
                        "UPDATE transactions SET flow_override = 'transfer' \
                         WHERE id = ?1 AND (flow_override IS NULL OR flow_override = '')",
                        params![item.id],
                    )
                    .map_err(|e| e.to_string())?;
                flagged_transfers += n;
            }
        }
    }

    let model = if provider == "ollama" { ollama_model } else { github_model };
    Ok(CategorizeResult {
        categorized,
        flagged_transfers,
        considered,
        model,
    })
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

        if !cashflow.variable_by_category.is_empty() {
            s.push_str("Variable spending by category (largest first):\n");
            for c in cashflow.variable_by_category.iter().take(8) {
                s.push_str(&format!("  - {}: USD ${:.2}\n", c.category, c.amount.usd));
            }
        }
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
                let cat = t.category.as_deref().unwrap_or("Uncategorized");
                s.push_str(&format!(
                    "  - {} | {} | {:.2} {} | {} | {}\n",
                    t.txn_date, t.description, t.amount, t.currency, t.flow_type, cat
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

#[cfg(test)]
mod agentic_tests {
    use super::*;

    #[test]
    fn normalize_merchant_collapses_noise_and_case() {
        assert_eq!(normalize_merchant("UNITED AIRLINES 0123"), "united airlines");
        assert_eq!(normalize_merchant("United Airlines #987"), "united airlines");
        // Keeps at most the first three words so distinct merchants stay distinct.
        assert_eq!(normalize_merchant("Amazon Web Services Inc"), "amazon web services");
        assert_eq!(normalize_merchant("  123 456  "), "");
    }

    #[test]
    fn most_common_picks_modal_description() {
        let v = vec![
            "NETFLIX.COM".to_string(),
            "NETFLIX.COM".to_string(),
            "Netflix membership".to_string(),
        ];
        assert_eq!(most_common(&v), "NETFLIX.COM");
    }

    #[test]
    fn is_liability_detects_credit_loan_and_negative_balances() {
        assert!(is_liability("credit_card", 0.0));
        assert!(is_liability("Credit", 500.0));
        assert!(is_liability("auto_loan", 0.0));
        assert!(is_liability("chequing", -25.0)); // overdrawn cash is still money owed
        assert!(!is_liability("savings", 1000.0));
        assert!(!is_liability("brokerage", 0.0));
    }

    #[test]
    fn cadence_days_spreads_occurrences_over_the_span() {
        // Three monthly hits across ~60 days → ~30-day cadence.
        assert_eq!(cadence_days("2025-01-01", "2025-03-02", 3), Some(30));
        // Tolerates a trailing time component.
        assert_eq!(cadence_days("2025-01-01T00:00:00Z", "2025-01-31", 2), Some(30));
        // Single occurrence or zero span yields nothing.
        assert_eq!(cadence_days("2025-01-01", "2025-01-01", 1), None);
        assert_eq!(cadence_days("2025-01-01", "2025-01-01", 2), None);
        assert_eq!(cadence_days("bad-date", "2025-01-31", 2), None);
    }

    #[test]
    fn round2_rounds_to_cents() {
        assert_eq!(round2(1234.5678), 1234.57);
        assert_eq!(round2(-0.005), -0.01);
        assert_eq!(round2(10.0), 10.0);
    }

    #[test]
    fn truncate_for_display_caps_long_strings_safely() {
        let s = "a".repeat(50);
        let out = truncate_for_display(&s, 10);
        assert!(out.starts_with(&"a".repeat(10)));
        assert!(out.contains("truncated"));
        // Short strings pass through untouched.
        assert_eq!(truncate_for_display("short", 10), "short");
        // Never panics on a multi-byte boundary right at the cap.
        let mb = "✓✓✓✓✓"; // 3 bytes each
        let _ = truncate_for_display(mb, 4);
    }

    #[test]
    fn finance_tools_are_well_formed() {
        let tools = finance_tools();
        assert_eq!(tools.len(), 8);
        for t in &tools {
            assert_eq!(t.kind, "function");
            assert!(!t.function.name.is_empty());
            assert!(!t.function.description.is_empty());
            assert_eq!(t.function.parameters["type"], "object");
        }
        let names: Vec<&str> = tools.iter().map(|t| t.function.name.as_str()).collect();
        assert!(names.contains(&"get_cashflow"));
        assert!(names.contains(&"find_recurring_transactions"));
        assert!(names.contains(&"get_liabilities"));
    }
}
