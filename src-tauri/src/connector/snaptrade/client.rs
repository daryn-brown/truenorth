//! Minimal SnapTrade API client.
//!
//! SnapTrade publishes SDKs for several languages but not Rust, so this is a hand-rolled
//! client covering exactly the read-only endpoints TrueNorth needs:
//!
//! - `GET  /api/v1/snapTrade/listUsers`            — validate the API key pair.
//! - `POST /api/v1/snapTrade/registerUser`         — create the (single) SnapTrade user.
//! - `POST /api/v1/snapTrade/login`                — get a connection-portal redirect URL.
//! - `GET  /api/v1/accounts`                       — list connected accounts + balances.
//! - `GET  /api/v1/accounts/{id}/positions`        — list positions for one account.
//! - `DELETE /api/v1/snapTrade/deleteUser`         — disconnect / self-heal.
//!
//! Every request is signed (see [`sign`]). Responses are parsed defensively from
//! `serde_json::Value` because brokerage payloads vary in shape between institutions.

use reqwest::Method;
use serde_json::{json, Value};
use thiserror::Error;

use super::sign;

const DEFAULT_BASE_URL: &str = "https://api.snaptrade.com";

#[derive(Debug, Error)]
pub enum SnapTradeError {
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("URL error: {0}")]
    Url(String),

    #[error("SnapTrade API error ({status}): {message}")]
    Api { status: u16, message: String },

    #[error("Unexpected SnapTrade response: {0}")]
    Parse(String),
}

impl SnapTradeError {
    /// True when the API rejected our credentials (HTTP 401/403) — used to surface a clear
    /// "check your API key" message and to detect the register-user self-heal case.
    pub fn is_auth(&self) -> bool {
        matches!(self, SnapTradeError::Api { status, .. } if *status == 401 || *status == 403)
    }

    /// The HTTP status code, when the error came from an API response.
    pub fn status(&self) -> Option<u16> {
        match self {
            SnapTradeError::Api { status, .. } => Some(*status),
            _ => None,
        }
    }
}

/// One brokerage account as reported by SnapTrade.
#[derive(Debug, Clone, PartialEq)]
pub struct SnapAccount {
    /// SnapTrade account UUID — stored as `connector_ref`.
    pub id: String,
    pub name: Option<String>,
    pub number: Option<String>,
    pub institution_name: Option<String>,
    /// Brokerage-specific account type string (e.g. "TFSA", "Individual", "401K").
    pub raw_type: Option<String>,
    /// Total account value in `currency`. The figure that flows into net worth.
    pub balance_total: Option<f64>,
    pub currency: Option<String>,
}

/// One position (holding) within an account.
#[derive(Debug, Clone, PartialEq)]
pub struct SnapPosition {
    pub symbol: String,
    pub units: f64,
    pub price: Option<f64>,
    pub average_purchase_price: Option<f64>,
    pub currency: Option<String>,
}

pub struct SnapTradeClient {
    base_url: String,
    client_id: String,
    consumer_key: String,
    http: reqwest::Client,
}

impl SnapTradeClient {
    pub fn new(client_id: impl Into<String>, consumer_key: impl Into<String>) -> Self {
        Self {
            base_url: DEFAULT_BASE_URL.to_string(),
            client_id: client_id.into(),
            consumer_key: consumer_key.into(),
            http: reqwest::Client::new(),
        }
    }

    #[cfg(test)]
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    /// Build, sign, and send a request, returning the parsed JSON body.
    ///
    /// `clientId` + `timestamp` are always appended to the query string; `extra_query` adds
    /// endpoint-specific params (e.g. `userId`/`userSecret`) in the given order. The signature
    /// is computed over the *exact* path and query string we send, so byte-consistency is
    /// guaranteed regardless of percent-encoding.
    async fn send(
        &self,
        method: Method,
        path: &str,
        extra_query: &[(&str, &str)],
        body: Option<Value>,
    ) -> Result<Value, SnapTradeError> {
        let timestamp = chrono::Utc::now().timestamp().to_string();

        let mut url = reqwest::Url::parse(&format!("{}{}", self.base_url, path))
            .map_err(|e| SnapTradeError::Url(e.to_string()))?;
        {
            let mut pairs = url.query_pairs_mut();
            pairs.append_pair("clientId", &self.client_id);
            pairs.append_pair("timestamp", &timestamp);
            for (k, v) in extra_query {
                pairs.append_pair(k, v);
            }
        }

        let query = url.query().unwrap_or("").to_string();
        let signature = sign::signature(&self.consumer_key, body.as_ref(), url.path(), &query);

        let mut req = self
            .http
            .request(method, url)
            .header("Signature", signature);
        if let Some(ref b) = body {
            req = req.json(b);
        }

        let resp = req.send().await?;
        let status = resp.status();
        let text = resp.text().await?;

        if !status.is_success() {
            return Err(SnapTradeError::Api {
                status: status.as_u16(),
                message: extract_error_message(&text),
            });
        }
        if text.trim().is_empty() {
            return Ok(Value::Null);
        }
        serde_json::from_str(&text).map_err(|e| SnapTradeError::Parse(e.to_string()))
    }

    /// Validate the `clientId` + `consumerKey` pair without needing a user. Lists registered
    /// SnapTrade users; a 401/403 means the API key is wrong.
    pub async fn check_credentials(&self) -> Result<(), SnapTradeError> {
        self.send(Method::GET, "/api/v1/snapTrade/listUsers", &[], None)
            .await?;
        Ok(())
    }

    /// List the SnapTrade user IDs registered under this API key. For a personal (`PERS-`) key
    /// this is the single user auto-provisioned at signup; the UI uses it to prefill the User ID
    /// so the user only needs to paste their User Secret.
    pub async fn list_users(&self) -> Result<Vec<String>, SnapTradeError> {
        let v = self
            .send(Method::GET, "/api/v1/snapTrade/listUsers", &[], None)
            .await?;
        let arr = v
            .as_array()
            .ok_or_else(|| SnapTradeError::Parse("listUsers: expected an array".into()))?;
        Ok(arr
            .iter()
            .filter_map(|u| u.as_str().map(str::to_string))
            .collect())
    }

    /// Register `user_id` and return the generated `userSecret`.
    pub async fn register_user(&self, user_id: &str) -> Result<String, SnapTradeError> {
        let body = json!({ "userId": user_id });
        let v = self
            .send(
                Method::POST,
                "/api/v1/snapTrade/registerUser",
                &[],
                Some(body),
            )
            .await?;
        v.get("userSecret")
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| SnapTradeError::Parse("registerUser: missing userSecret".into()))
    }

    /// Delete `user_id` and all of its SnapTrade data. Used for disconnect + self-heal.
    pub async fn delete_user(&self, user_id: &str) -> Result<(), SnapTradeError> {
        self.send(
            Method::DELETE,
            "/api/v1/snapTrade/deleteUser",
            &[("userId", user_id)],
            None,
        )
        .await?;
        Ok(())
    }

    /// Get a connection-portal redirect URL where the user authorizes a brokerage.
    /// We request a **read-only** connection so trading scopes are never granted.
    pub async fn login_link(
        &self,
        user_id: &str,
        user_secret: &str,
    ) -> Result<String, SnapTradeError> {
        let body = json!({ "connectionType": "read" });
        let v = self
            .send(
                Method::POST,
                "/api/v1/snapTrade/login",
                &[("userId", user_id), ("userSecret", user_secret)],
                Some(body),
            )
            .await?;
        v.get("redirectURI")
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| SnapTradeError::Parse("login: missing redirectURI".into()))
    }

    /// List all accounts connected by the user, including current balances.
    pub async fn list_accounts(
        &self,
        user_id: &str,
        user_secret: &str,
    ) -> Result<Vec<SnapAccount>, SnapTradeError> {
        let v = self
            .send(
                Method::GET,
                "/api/v1/accounts",
                &[("userId", user_id), ("userSecret", user_secret)],
                None,
            )
            .await?;
        let arr = v
            .as_array()
            .ok_or_else(|| SnapTradeError::Parse("accounts: expected an array".into()))?;
        Ok(arr.iter().filter_map(parse_account).collect())
    }

    /// List positions (holdings) for one account.
    pub async fn account_positions(
        &self,
        user_id: &str,
        user_secret: &str,
        account_id: &str,
    ) -> Result<Vec<SnapPosition>, SnapTradeError> {
        let path = format!("/api/v1/accounts/{account_id}/positions");
        let v = self
            .send(
                Method::GET,
                &path,
                &[("userId", user_id), ("userSecret", user_secret)],
                None,
            )
            .await?;
        let arr = v
            .as_array()
            .ok_or_else(|| SnapTradeError::Parse("positions: expected an array".into()))?;
        Ok(arr.iter().filter_map(parse_position).collect())
    }
}

// ---------------------------------------------------------------------------
// Defensive JSON parsing helpers
// ---------------------------------------------------------------------------

/// Pull a human-readable message out of a SnapTrade error body, falling back to the raw text.
fn extract_error_message(text: &str) -> String {
    if let Ok(v) = serde_json::from_str::<Value>(text) {
        if let Some(detail) = v.get("detail").and_then(Value::as_str) {
            return detail.to_string();
        }
        if let Some(message) = v.get("message").and_then(Value::as_str) {
            return message.to_string();
        }
    }
    if text.trim().is_empty() {
        "no response body".to_string()
    } else {
        text.trim().to_string()
    }
}

/// Interpret a value as an ISO currency code, accepting either a bare string ("USD") or an
/// object with a `code` field (`{ "id": …, "code": "USD", "name": … }`).
fn currency_code(v: &Value) -> Option<String> {
    if let Some(s) = v.as_str() {
        return Some(s.to_string());
    }
    v.get("code").and_then(Value::as_str).map(str::to_string)
}

fn parse_account(v: &Value) -> Option<SnapAccount> {
    let id = v.get("id").and_then(Value::as_str)?.to_string();

    let balance = v.get("balance").and_then(|b| b.get("total"));
    let balance_total = balance.and_then(|t| {
        t.get("amount")
            .and_then(Value::as_f64)
            .or_else(|| t.as_f64())
    });
    let currency = balance
        .and_then(|t| t.get("currency"))
        .and_then(currency_code)
        .or_else(|| v.get("currency").and_then(currency_code));

    // `raw_type` may live at the top level or inside the free-form `meta` object.
    let raw_type = v
        .get("raw_type")
        .and_then(Value::as_str)
        .or_else(|| {
            v.get("meta")
                .and_then(|m| m.get("type"))
                .and_then(Value::as_str)
        })
        .or_else(|| {
            v.get("meta")
                .and_then(|m| m.get("brokerage_account_type"))
                .and_then(Value::as_str)
        })
        .map(str::to_string);

    Some(SnapAccount {
        id,
        name: v.get("name").and_then(Value::as_str).map(str::to_string),
        number: v.get("number").and_then(Value::as_str).map(str::to_string),
        institution_name: v
            .get("institution_name")
            .and_then(Value::as_str)
            .map(str::to_string),
        raw_type,
        balance_total,
        currency,
    })
}

fn parse_position(v: &Value) -> Option<SnapPosition> {
    let symbol = position_symbol(v)?;
    let units = v.get("units").and_then(Value::as_f64).unwrap_or(0.0);
    Some(SnapPosition {
        symbol,
        units,
        price: v.get("price").and_then(Value::as_f64),
        average_purchase_price: v.get("average_purchase_price").and_then(Value::as_f64),
        currency: position_currency(v),
    })
}

/// Extract the ticker from a position. SnapTrade nests it as
/// `position.symbol.symbol.symbol` (a brokerage symbol wrapping a universal symbol), but
/// shapes vary, so we try the known fallbacks.
fn position_symbol(v: &Value) -> Option<String> {
    let sym = v.get("symbol")?;
    if let Some(universal) = sym.get("symbol") {
        if let Some(s) = universal.as_str() {
            return Some(s.to_string());
        }
        if let Some(s) = universal.get("symbol").and_then(Value::as_str) {
            return Some(s.to_string());
        }
        if let Some(s) = universal.get("raw_symbol").and_then(Value::as_str) {
            return Some(s.to_string());
        }
    }
    if let Some(s) = sym.get("raw_symbol").and_then(Value::as_str) {
        return Some(s.to_string());
    }
    sym.as_str().map(str::to_string)
}

fn position_currency(v: &Value) -> Option<String> {
    let sym = v.get("symbol")?;
    let universal = sym.get("symbol").unwrap_or(sym);
    universal.get("currency").and_then(currency_code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_account_with_nested_balance_and_currency_object() {
        let v = json!({
            "id": "acc-123",
            "name": "Robinhood Individual",
            "number": "****1234",
            "institution_name": "Robinhood",
            "raw_type": "Individual",
            "balance": { "total": { "amount": 1234.56, "currency": "USD" } }
        });
        let acc = parse_account(&v).unwrap();
        assert_eq!(acc.id, "acc-123");
        assert_eq!(acc.name.as_deref(), Some("Robinhood Individual"));
        assert_eq!(acc.institution_name.as_deref(), Some("Robinhood"));
        assert_eq!(acc.raw_type.as_deref(), Some("Individual"));
        assert_eq!(acc.balance_total, Some(1234.56));
        assert_eq!(acc.currency.as_deref(), Some("USD"));
    }

    #[test]
    fn parses_account_currency_from_object_and_meta_type() {
        let v = json!({
            "id": "acc-xyz",
            "meta": { "type": "TFSA" },
            "balance": { "total": { "amount": 500.0, "currency": { "id": "c", "code": "CAD" } } }
        });
        let acc = parse_account(&v).unwrap();
        assert_eq!(acc.raw_type.as_deref(), Some("TFSA"));
        assert_eq!(acc.currency.as_deref(), Some("CAD"));
        assert_eq!(acc.balance_total, Some(500.0));
    }

    #[test]
    fn account_missing_id_is_skipped() {
        let v = json!({ "name": "no id here" });
        assert!(parse_account(&v).is_none());
    }

    #[test]
    fn parses_position_with_deeply_nested_symbol() {
        let v = json!({
            "symbol": {
                "symbol": {
                    "symbol": "AAPL",
                    "raw_symbol": "AAPL",
                    "currency": { "code": "USD" }
                }
            },
            "units": 10.0,
            "price": 150.25,
            "average_purchase_price": 100.0
        });
        let pos = parse_position(&v).unwrap();
        assert_eq!(pos.symbol, "AAPL");
        assert_eq!(pos.units, 10.0);
        assert_eq!(pos.price, Some(150.25));
        assert_eq!(pos.average_purchase_price, Some(100.0));
        assert_eq!(pos.currency.as_deref(), Some("USD"));
    }

    #[test]
    fn parses_position_with_canadian_ticker() {
        let v = json!({
            "symbol": { "symbol": { "symbol": "VAB.TO", "currency": "CAD" } },
            "units": 3.5,
            "price": 24.1
        });
        let pos = parse_position(&v).unwrap();
        assert_eq!(pos.symbol, "VAB.TO");
        assert_eq!(pos.units, 3.5);
        assert_eq!(pos.currency.as_deref(), Some("CAD"));
        assert_eq!(pos.average_purchase_price, None);
    }

    #[test]
    fn extracts_error_detail_message() {
        let body = r#"{"detail":"User not found","status_code":404}"#;
        assert_eq!(extract_error_message(body), "User not found");
    }
}
