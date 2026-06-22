//! Minimal SimpleFIN Bridge client.
//!
//! SimpleFIN is intentionally simple — there is no request signing or OAuth. The flow is:
//!
//! 1. The user creates a **setup token** at their SimpleFIN server (e.g. the SimpleFIN Bridge)
//!    and pastes it into TrueNorth. The token is a Base64-encoded **claim URL**.
//! 2. We POST to the claim URL once to receive a persistent **access URL** that embeds HTTP
//!    Basic credentials (`https://user:pass@host/simplefin`). The access URL is the only secret
//!    and is stored in the OS keychain.
//! 3. We GET `{access-url}/accounts` (Basic auth) to read balances + holdings.
//!
//! Responses are parsed defensively from `serde_json::Value`: the SimpleFIN Bridge embeds an
//! `org` object and a `holdings` array per account (extensions over the core protocol), while the
//! draft v2 protocol uses a separate `connections` list — we support both shapes.

use std::collections::HashMap;

use base64::Engine;
use serde_json::Value;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SimpleFinError {
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("URL error: {0}")]
    Url(String),

    #[error("SimpleFIN claim failed ({status}): {message}")]
    Claim { status: u16, message: String },

    #[error("SimpleFIN API error ({status}): {message}")]
    Api { status: u16, message: String },

    #[error("Unexpected SimpleFIN response: {0}")]
    Parse(String),
}

impl SimpleFinError {
    /// True when SimpleFIN rejected our credentials (HTTP 401/403) — used to surface a clear
    /// "reconnect" message and to advise that a compromised token should be disabled.
    pub fn is_auth(&self) -> bool {
        matches!(
            self,
            SimpleFinError::Api { status, .. } | SimpleFinError::Claim { status, .. }
                if *status == 401 || *status == 403
        )
    }
}

/// One holding (investment position) within a SimpleFIN account.
#[derive(Debug, Clone, PartialEq)]
pub struct SimpleFinHolding {
    pub symbol: String,
    pub shares: f64,
    pub market_value: Option<f64>,
    pub cost_basis: Option<f64>,
    pub currency: Option<String>,
}

/// One transaction within a SimpleFIN account. SimpleFIN reports `amount` as a signed string:
/// negative is money out (spending), positive is money in (income/refunds).
#[derive(Debug, Clone, PartialEq)]
pub struct SimpleFinTransaction {
    /// SimpleFIN transaction id — stored as `connector_ref` for dedup across syncs.
    pub id: String,
    /// Signed amount in the account's currency (negative = outflow).
    pub amount: f64,
    pub description: String,
    /// `posted` (or `transacted_at`) as a UNIX epoch timestamp (seconds).
    pub posted: Option<i64>,
    pub memo: Option<String>,
}

/// One account as reported by SimpleFIN.
#[derive(Debug, Clone, PartialEq)]
pub struct SimpleFinAccount {
    /// SimpleFIN account id — stored as `connector_ref`.
    pub id: String,
    pub name: String,
    pub currency: String,
    /// Current balance in `currency`. The figure that flows into net worth.
    pub balance: Option<f64>,
    /// `balance-date` as a UNIX epoch timestamp (seconds).
    pub balance_date: Option<i64>,
    /// Institution / connection name, best-effort.
    pub institution: Option<String>,
    pub holdings: Vec<SimpleFinHolding>,
    pub transactions: Vec<SimpleFinTransaction>,
}

/// The parsed `/accounts` response: the accounts plus any user-facing errors SimpleFIN returned.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SimpleFinAccountSet {
    pub accounts: Vec<SimpleFinAccount>,
    pub errors: Vec<String>,
}

/// Exchange a setup token for a persistent access URL.
///
/// The setup token is Base64 of a claim URL; we POST to it once and the response body is the
/// access URL (with embedded Basic credentials). A 403 means the token was already claimed or is
/// invalid — the caller should tell the user to disable it.
pub async fn claim_access_url(setup_token: &str) -> Result<String, SimpleFinError> {
    let claim_url = decode_setup_token(setup_token)?;
    let resp = reqwest::Client::new().post(&claim_url).send().await?;
    let status = resp.status();
    let text = resp.text().await?;
    if !status.is_success() {
        return Err(SimpleFinError::Claim {
            status: status.as_u16(),
            message: extract_error_message(&text),
        });
    }
    let access_url = text.trim().to_string();
    reqwest::Url::parse(&access_url)
        .map_err(|e| SimpleFinError::Parse(format!("claim did not return an access URL: {e}")))?;
    Ok(access_url)
}

/// Base64-decode a setup token into its claim URL, accepting standard or URL-safe alphabets.
fn decode_setup_token(token: &str) -> Result<String, SimpleFinError> {
    let token = token.trim();
    if token.is_empty() {
        return Err(SimpleFinError::Parse("setup token is empty".into()));
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(token)
        .or_else(|_| base64::engine::general_purpose::URL_SAFE.decode(token))
        .map_err(|e| SimpleFinError::Parse(format!("invalid setup token: {e}")))?;
    let url = String::from_utf8(bytes)
        .map_err(|e| SimpleFinError::Parse(format!("invalid setup token: {e}")))?
        .trim()
        .to_string();
    reqwest::Url::parse(&url)
        .map_err(|e| SimpleFinError::Parse(format!("setup token did not decode to a URL: {e}")))?;
    Ok(url)
}

pub struct SimpleFinClient {
    access_url: String,
    http: reqwest::Client,
}

impl SimpleFinClient {
    pub fn new(access_url: impl Into<String>) -> Self {
        Self {
            access_url: access_url.into(),
            http: reqwest::Client::new(),
        }
    }

    /// Fetch accounts with balances, holdings, and recent transactions. A `start-date` bounds the
    /// transaction window (the trailing ~120 days) so payloads stay small; balances and holdings
    /// are always current regardless of the window.
    pub async fn fetch_accounts(&self) -> Result<SimpleFinAccountSet, SimpleFinError> {
        let (endpoint, user, pass) = accounts_endpoint(&self.access_url)?;
        // SimpleFIN expects `start-date` as UNIX epoch seconds.
        let start_date = (chrono::Utc::now() - chrono::Duration::days(120))
            .timestamp()
            .to_string();
        let resp = self
            .http
            .get(&endpoint)
            .basic_auth(user, (!pass.is_empty()).then_some(pass))
            .query(&[("start-date", start_date.as_str())])
            .send()
            .await?;
        let status = resp.status();
        let text = resp.text().await?;
        if !status.is_success() {
            return Err(SimpleFinError::Api {
                status: status.as_u16(),
                message: extract_error_message(&text),
            });
        }
        let v: Value =
            serde_json::from_str(&text).map_err(|e| SimpleFinError::Parse(e.to_string()))?;
        Ok(parse_account_set(&v))
    }

    /// Validate the access URL by fetching accounts. Used right after claiming.
    pub async fn check(&self) -> Result<(), SimpleFinError> {
        self.fetch_accounts().await.map(|_| ())
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Split an access URL into the `/accounts` endpoint plus the Basic-auth username + password it
/// embeds. SimpleFIN access URLs look like `https://user:pass@host/simplefin`.
fn accounts_endpoint(access_url: &str) -> Result<(String, String, String), SimpleFinError> {
    let mut url =
        reqwest::Url::parse(access_url.trim()).map_err(|e| SimpleFinError::Url(e.to_string()))?;
    let user = url.username().to_string();
    let pass = url.password().unwrap_or("").to_string();
    // Strip the embedded credentials; we send them via the Authorization header instead.
    let _ = url.set_username("");
    let _ = url.set_password(None);
    let base = url.as_str().trim_end_matches('/').to_string();
    Ok((format!("{base}/accounts"), user, pass))
}

/// Parse a numeric string ("100.23") or a bare JSON number into an `f64`.
fn parse_decimal(v: &Value) -> Option<f64> {
    match v {
        Value::String(s) => s.trim().parse::<f64>().ok(),
        _ => v.as_f64(),
    }
}

fn parse_account_set(v: &Value) -> SimpleFinAccountSet {
    let mut errors = Vec::new();
    // Bridge uses `errors` (strings); draft v2 uses `errlist` (objects with `msg`). Support both.
    for key in ["errors", "errlist"] {
        if let Some(arr) = v.get(key).and_then(Value::as_array) {
            for e in arr {
                if let Some(s) = e.as_str() {
                    errors.push(s.to_string());
                } else if let Some(msg) = e.get("msg").and_then(Value::as_str) {
                    errors.push(msg.to_string());
                }
            }
        }
    }

    // Draft v2 separates connections; map conn_id -> name for institution lookup.
    let mut conn_names: HashMap<String, String> = HashMap::new();
    if let Some(arr) = v.get("connections").and_then(Value::as_array) {
        for c in arr {
            if let (Some(id), Some(name)) = (
                c.get("conn_id").and_then(Value::as_str),
                c.get("name").and_then(Value::as_str),
            ) {
                conn_names.insert(id.to_string(), name.to_string());
            }
        }
    }

    let accounts = v
        .get("accounts")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|a| parse_account(a, &conn_names))
                .collect()
        })
        .unwrap_or_default();

    SimpleFinAccountSet { accounts, errors }
}

fn parse_account(v: &Value, conn_names: &HashMap<String, String>) -> Option<SimpleFinAccount> {
    let id = v.get("id").and_then(Value::as_str)?.to_string();
    let name = v
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("Account")
        .to_string();
    let currency = v
        .get("currency")
        .and_then(Value::as_str)
        .unwrap_or("USD")
        .to_string();

    // Institution: Bridge embeds `org.name`/`org.domain`; the protocol uses `conn_name` or a
    // `conn_id` resolved against the connections list.
    let institution = v
        .get("org")
        .and_then(|o| o.get("name"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| {
            v.get("conn_name")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .or_else(|| {
            v.get("conn_id")
                .and_then(Value::as_str)
                .and_then(|id| conn_names.get(id).cloned())
        })
        .or_else(|| {
            v.get("org")
                .and_then(|o| o.get("domain"))
                .and_then(Value::as_str)
                .map(str::to_string)
        });

    let holdings = v
        .get("holdings")
        .and_then(Value::as_array)
        .map(|arr| arr.iter().filter_map(parse_holding).collect())
        .unwrap_or_default();

    let transactions = v
        .get("transactions")
        .and_then(Value::as_array)
        .map(|arr| arr.iter().filter_map(parse_transaction).collect())
        .unwrap_or_default();

    Some(SimpleFinAccount {
        id,
        name,
        currency,
        balance: v.get("balance").and_then(parse_decimal),
        balance_date: v.get("balance-date").and_then(Value::as_i64),
        institution,
        holdings,
        transactions,
    })
}

/// Parse one SimpleFIN transaction. Requires an `id` and a numeric `amount`; the description
/// falls back to `payee` and then a placeholder, and the date prefers `posted` over
/// `transacted_at`.
fn parse_transaction(v: &Value) -> Option<SimpleFinTransaction> {
    let id = v.get("id").and_then(Value::as_str)?.to_string();
    let amount = v.get("amount").and_then(parse_decimal)?;
    let description = v
        .get("description")
        .and_then(Value::as_str)
        .or_else(|| v.get("payee").and_then(Value::as_str))
        .unwrap_or("Transaction")
        .to_string();
    let posted = v
        .get("posted")
        .and_then(Value::as_i64)
        .or_else(|| v.get("transacted_at").and_then(Value::as_i64));
    let memo = v.get("memo").and_then(Value::as_str).map(str::to_string);
    Some(SimpleFinTransaction {
        id,
        amount,
        description,
        posted,
        memo,
    })
}

fn parse_holding(v: &Value) -> Option<SimpleFinHolding> {
    let symbol = v
        .get("symbol")
        .and_then(Value::as_str)
        .or_else(|| v.get("description").and_then(Value::as_str))?
        .to_string();
    Some(SimpleFinHolding {
        symbol,
        shares: v.get("shares").and_then(parse_decimal).unwrap_or(0.0),
        market_value: v.get("market_value").and_then(parse_decimal),
        cost_basis: v.get("cost_basis").and_then(parse_decimal),
        currency: v.get("currency").and_then(Value::as_str).map(str::to_string),
    })
}

/// Pull a human-readable message out of an error body, falling back to the raw text.
fn extract_error_message(text: &str) -> String {
    if let Ok(v) = serde_json::from_str::<Value>(text) {
        // A top-level `errors`/`errlist` array, or a single `{ "msg": ... }` object.
        for key in ["errors", "errlist"] {
            if let Some(arr) = v.get(key).and_then(Value::as_array) {
                let msgs: Vec<String> = arr
                    .iter()
                    .filter_map(|e| {
                        e.as_str()
                            .map(str::to_string)
                            .or_else(|| e.get("msg").and_then(Value::as_str).map(str::to_string))
                    })
                    .collect();
                if !msgs.is_empty() {
                    return msgs.join("; ");
                }
            }
        }
        if let Some(msg) = v.get("msg").and_then(Value::as_str) {
            return msg.to_string();
        }
    }
    if text.trim().is_empty() {
        "no response body".to_string()
    } else {
        text.trim().to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn decodes_demo_setup_token() {
        // Base64 of https://bridge.simplefin.org/simplefin/claim/demo
        let token = "aHR0cHM6Ly9icmlkZ2Uuc2ltcGxlZmluLm9yZy9zaW1wbGVmaW4vY2xhaW0vZGVtbw==";
        assert_eq!(
            decode_setup_token(token).unwrap(),
            "https://bridge.simplefin.org/simplefin/claim/demo"
        );
    }

    #[test]
    fn rejects_garbage_and_empty_tokens() {
        assert!(decode_setup_token("").is_err());
        assert!(decode_setup_token("not base64!!!").is_err());
        // Valid base64 but not a URL.
        let not_a_url = base64::engine::general_purpose::STANDARD.encode("hello world");
        assert!(decode_setup_token(&not_a_url).is_err());
    }

    #[test]
    fn splits_access_url_into_endpoint_and_credentials() {
        let (endpoint, user, pass) =
            accounts_endpoint("https://abc123:secretpw@bridge.simplefin.org/simplefin").unwrap();
        assert_eq!(endpoint, "https://bridge.simplefin.org/simplefin/accounts");
        assert_eq!(user, "abc123");
        assert_eq!(pass, "secretpw");
    }

    #[test]
    fn parses_bridge_account_with_org_and_holdings() {
        let v = json!({
            "errors": [],
            "accounts": [{
                "id": "act-1",
                "name": "Brokerage",
                "currency": "USD",
                "balance": "1234.56",
                "balance-date": 978366153i64,
                "org": { "name": "Wealthsimple", "domain": "wealthsimple.com" },
                "holdings": [{
                    "symbol": "AAPL",
                    "shares": "10",
                    "market_value": "1500.00",
                    "cost_basis": "1000.00",
                    "currency": "USD"
                }]
            }]
        });
        let set = parse_account_set(&v);
        assert!(set.errors.is_empty());
        assert_eq!(set.accounts.len(), 1);
        let acc = &set.accounts[0];
        assert_eq!(acc.id, "act-1");
        assert_eq!(acc.institution.as_deref(), Some("Wealthsimple"));
        assert_eq!(acc.balance, Some(1234.56));
        assert_eq!(acc.balance_date, Some(978366153));
        assert_eq!(acc.holdings.len(), 1);
        let h = &acc.holdings[0];
        assert_eq!(h.symbol, "AAPL");
        assert_eq!(h.shares, 10.0);
        assert_eq!(h.market_value, Some(1500.0));
        assert_eq!(h.cost_basis, Some(1000.0));
    }

    #[test]
    fn parses_protocol_account_with_connections_list() {
        let v = json!({
            "errlist": [{ "code": "con.auth", "msg": "Authentication required" }],
            "connections": [{ "conn_id": "CON-1", "name": "My Bank - Jill" }],
            "accounts": [{
                "id": "2930002",
                "name": "Savings",
                "conn_id": "CON-1",
                "currency": "CAD",
                "balance": "100.23",
                "balance-date": 978366153i64
            }]
        });
        let set = parse_account_set(&v);
        assert_eq!(set.errors, vec!["Authentication required".to_string()]);
        assert_eq!(set.accounts.len(), 1);
        let acc = &set.accounts[0];
        assert_eq!(acc.currency, "CAD");
        assert_eq!(acc.institution.as_deref(), Some("My Bank - Jill"));
        assert_eq!(acc.balance, Some(100.23));
        assert!(acc.holdings.is_empty());
    }

    #[test]
    fn account_missing_id_is_skipped() {
        let v = json!({ "accounts": [{ "name": "no id" }] });
        assert!(parse_account_set(&v).accounts.is_empty());
    }

    #[test]
    fn parses_account_transactions_with_signed_amounts() {
        let v = json!({
            "accounts": [{
                "id": "act-1",
                "name": "Everyday Chequing",
                "currency": "CAD",
                "balance": "100.00",
                "transactions": [
                    {
                        "id": "txn-1",
                        "posted": 1700000000i64,
                        "amount": "-42.50",
                        "description": "GROCERY STORE"
                    },
                    {
                        // No `description` — falls back to `payee`; `transacted_at` dates it.
                        "id": "txn-2",
                        "transacted_at": 1700100000i64,
                        "amount": "2500.00",
                        "payee": "MICROSOFT PAYROLL",
                        "memo": "bi-weekly"
                    },
                    // Missing amount — skipped.
                    { "id": "txn-3", "description": "no amount" }
                ]
            }]
        });
        let set = parse_account_set(&v);
        let txns = &set.accounts[0].transactions;
        assert_eq!(txns.len(), 2);
        assert_eq!(txns[0].id, "txn-1");
        assert_eq!(txns[0].amount, -42.50);
        assert_eq!(txns[0].description, "GROCERY STORE");
        assert_eq!(txns[0].posted, Some(1700000000));
        assert_eq!(txns[1].description, "MICROSOFT PAYROLL");
        assert_eq!(txns[1].amount, 2500.00);
        assert_eq!(txns[1].posted, Some(1700100000));
        assert_eq!(txns[1].memo.as_deref(), Some("bi-weekly"));
    }
}
