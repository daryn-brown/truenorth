//! Minimal Questrade REST API client.
//!
//! Questrade uses a manual OAuth2 **refresh-token** flow (no client secret for personal apps):
//!
//! 1. The user enables the API in their Questrade **API Centre**, registers a personal app, runs a
//!    **manual authorization**, and copies the resulting **refresh token** into TrueNorth.
//! 2. We POST that refresh token to `login.questrade.com/oauth2/token` to receive a short-lived
//!    **access token**, the account-specific **API server** URL, and a **new refresh token**.
//!    Refresh tokens are single-use: each refresh rotates it, so the caller must persist the new
//!    one immediately. An unused refresh token expires after ~7 days.
//! 3. We GET `{api_server}v1/accounts`, `…/balances`, and `…/positions` (Bearer auth) to read the
//!    full account value (cash **and** equity) plus individual positions.
//!
//! The refresh token is the only durable secret and is stored in the OS keychain; the access token
//! lives only in memory for the duration of a sync. Responses are parsed defensively from
//! `serde_json::Value`.

use serde_json::Value;
use thiserror::Error;

/// Questrade's OAuth2 token endpoint. The account-specific data host (`api_server`) is returned in
/// the token response and differs per user, so it is never hard-coded.
pub const TOKEN_URL: &str = "https://login.questrade.com/oauth2/token";

#[derive(Debug, Error)]
pub enum QuestradeError {
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("URL error: {0}")]
    Url(String),

    #[error("Questrade authorization failed ({status}): {message}")]
    Auth { status: u16, message: String },

    #[error("Questrade API error ({status}): {message}")]
    Api { status: u16, message: String },

    #[error("Unexpected Questrade response: {0}")]
    Parse(String),
}

impl QuestradeError {
    /// True when Questrade rejected our credentials — used to surface a clear "re-authorize in the
    /// API Centre" message. A failed token refresh is always treated as an auth problem because the
    /// refresh token is the only thing that can be wrong there.
    pub fn is_auth(&self) -> bool {
        matches!(self, QuestradeError::Auth { .. })
            || matches!(
                self,
                QuestradeError::Api { status, .. } if *status == 401 || *status == 403
            )
    }
}

/// The result of exchanging a refresh token: a usable access token + data host, plus the **rotated**
/// refresh token the caller must persist for next time.
#[derive(Debug, Clone, PartialEq)]
pub struct QuestradeTokens {
    pub access_token: String,
    /// Account-specific API host, e.g. `https://api01.iq.questrade.com/` (keeps its trailing slash).
    pub api_server: String,
    /// The new single-use refresh token. Persist this — the one we sent is now spent.
    pub refresh_token: String,
    pub expires_in: i64,
}

/// One Questrade account (the `/accounts` list entry).
#[derive(Debug, Clone, PartialEq)]
pub struct QuestradeAccount {
    /// Account number — stored as `connector_ref`.
    pub number: String,
    /// Questrade account type, e.g. "TFSA", "RRSP", "Margin", "Cash", "FHSA".
    pub account_type: String,
    pub status: Option<String>,
}

/// One per-currency or combined balance row from `/accounts/{id}/balances`.
#[derive(Debug, Clone, PartialEq)]
pub struct QuestradeBalance {
    pub currency: String,
    pub cash: Option<f64>,
    pub market_value: Option<f64>,
    /// `cash + marketValue` — the full account value in `currency`. The figure for net worth.
    pub total_equity: Option<f64>,
}

/// One position (holding) from `/accounts/{id}/positions`.
#[derive(Debug, Clone, PartialEq)]
pub struct QuestradePosition {
    pub symbol: String,
    pub open_quantity: f64,
    pub current_price: Option<f64>,
    pub average_entry_price: Option<f64>,
    pub current_market_value: Option<f64>,
}

/// Exchange a refresh token for an access token + data host (and a rotated refresh token).
///
/// Any non-success response means the refresh token is invalid or expired, so it is surfaced as a
/// [`QuestradeError::Auth`] regardless of the exact status code.
pub async fn refresh_access_token(refresh_token: &str) -> Result<QuestradeTokens, QuestradeError> {
    let refresh_token = refresh_token.trim();
    if refresh_token.is_empty() {
        return Err(QuestradeError::Parse("refresh token is empty".into()));
    }

    let client = reqwest::Client::new();
    let request = build_token_request(&client, refresh_token)?;
    let resp = client.execute(request).await?;
    let status = resp.status();
    let text = resp.text().await?;
    if !status.is_success() {
        return Err(QuestradeError::Auth {
            status: status.as_u16(),
            message: extract_error_message(&text),
        });
    }
    let v: Value = serde_json::from_str(&text).map_err(|e| QuestradeError::Parse(e.to_string()))?;
    parse_tokens(&v)
}

/// Build the OAuth2 refresh-token request.
///
/// Questrade's login server rejects a body-less `POST` with **HTTP 411 (Length Required)**, so the
/// parameters must be sent as a form-encoded **body** (which carries a `Content-Length`) rather than
/// in the query string. Sending them via `.query(...)` produces a body-less POST and the 411 that
/// previously surfaced as a misleading "refresh token expired / already used" error.
fn build_token_request(
    client: &reqwest::Client,
    refresh_token: &str,
) -> Result<reqwest::Request, reqwest::Error> {
    client
        .post(TOKEN_URL)
        .form(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
        ])
        .build()
}

pub struct QuestradeClient {
    /// Normalised data host with no trailing slash.
    api_server: String,
    access_token: String,
    http: reqwest::Client,
}

impl QuestradeClient {
    pub fn new(api_server: impl Into<String>, access_token: impl Into<String>) -> Self {
        let api_server = api_server.into().trim_end_matches('/').to_string();
        Self {
            api_server,
            access_token: access_token.into(),
            http: reqwest::Client::new(),
        }
    }

    async fn get(&self, path: &str) -> Result<Value, QuestradeError> {
        let url = format!("{}/{}", self.api_server, path.trim_start_matches('/'));
        let resp = self
            .http
            .get(&url)
            .bearer_auth(&self.access_token)
            .send()
            .await?;
        let status = resp.status();
        let text = resp.text().await?;
        if !status.is_success() {
            return Err(QuestradeError::Api {
                status: status.as_u16(),
                message: extract_error_message(&text),
            });
        }
        serde_json::from_str(&text).map_err(|e| QuestradeError::Parse(e.to_string()))
    }

    /// List the user's accounts.
    pub async fn list_accounts(&self) -> Result<Vec<QuestradeAccount>, QuestradeError> {
        let v = self.get("v1/accounts").await?;
        Ok(v.get("accounts")
            .and_then(Value::as_array)
            .map(|arr| arr.iter().filter_map(parse_account).collect())
            .unwrap_or_default())
    }

    /// Fetch an account's balances. Returns the **combined** balances (all holdings expressed in each
    /// currency) when present, falling back to the per-currency balances.
    pub async fn account_balances(
        &self,
        number: &str,
    ) -> Result<Vec<QuestradeBalance>, QuestradeError> {
        let v = self.get(&format!("v1/accounts/{number}/balances")).await?;
        let arr = v
            .get("combinedBalances")
            .and_then(Value::as_array)
            .or_else(|| v.get("perCurrencyBalances").and_then(Value::as_array));
        Ok(arr
            .map(|a| a.iter().filter_map(parse_balance).collect())
            .unwrap_or_default())
    }

    /// List an account's open positions.
    pub async fn account_positions(
        &self,
        number: &str,
    ) -> Result<Vec<QuestradePosition>, QuestradeError> {
        let v = self.get(&format!("v1/accounts/{number}/positions")).await?;
        Ok(v.get("positions")
            .and_then(Value::as_array)
            .map(|arr| arr.iter().filter_map(parse_position).collect())
            .unwrap_or_default())
    }

    /// Validate the access token by listing accounts. Used right after a refresh.
    pub async fn check(&self) -> Result<(), QuestradeError> {
        self.list_accounts().await.map(|_| ())
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Parse a numeric string ("100.23") or a bare JSON number into an `f64`.
fn parse_decimal(v: &Value) -> Option<f64> {
    match v {
        Value::String(s) => s.trim().parse::<f64>().ok(),
        _ => v.as_f64(),
    }
}

fn parse_tokens(v: &Value) -> Result<QuestradeTokens, QuestradeError> {
    let access_token = v
        .get("access_token")
        .and_then(Value::as_str)
        .ok_or_else(|| QuestradeError::Parse("token response missing access_token".into()))?
        .to_string();
    let api_server = v
        .get("api_server")
        .and_then(Value::as_str)
        .ok_or_else(|| QuestradeError::Parse("token response missing api_server".into()))?
        .to_string();
    let refresh_token = v
        .get("refresh_token")
        .and_then(Value::as_str)
        .ok_or_else(|| QuestradeError::Parse("token response missing refresh_token".into()))?
        .to_string();
    reqwest::Url::parse(&api_server).map_err(|e| {
        QuestradeError::Parse(format!("token response has an invalid api_server: {e}"))
    })?;
    Ok(QuestradeTokens {
        access_token,
        api_server,
        refresh_token,
        expires_in: v.get("expires_in").and_then(Value::as_i64).unwrap_or(0),
    })
}

fn parse_account(v: &Value) -> Option<QuestradeAccount> {
    let number = v.get("number").and_then(Value::as_str)?.to_string();
    Some(QuestradeAccount {
        number,
        account_type: v
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        status: v.get("status").and_then(Value::as_str).map(str::to_string),
    })
}

fn parse_balance(v: &Value) -> Option<QuestradeBalance> {
    let currency = v.get("currency").and_then(Value::as_str)?.to_string();
    Some(QuestradeBalance {
        currency,
        cash: v.get("cash").and_then(parse_decimal),
        market_value: v.get("marketValue").and_then(parse_decimal),
        total_equity: v.get("totalEquity").and_then(parse_decimal),
    })
}

fn parse_position(v: &Value) -> Option<QuestradePosition> {
    let symbol = v.get("symbol").and_then(Value::as_str)?.to_string();
    Some(QuestradePosition {
        symbol,
        open_quantity: v.get("openQuantity").and_then(parse_decimal).unwrap_or(0.0),
        current_price: v.get("currentPrice").and_then(parse_decimal),
        average_entry_price: v.get("averageEntryPrice").and_then(parse_decimal),
        current_market_value: v.get("currentMarketValue").and_then(parse_decimal),
    })
}

/// Pull a human-readable message out of a Questrade error body (`{ "code", "message" }`), falling
/// back to the raw text.
fn extract_error_message(text: &str) -> String {
    if let Ok(v) = serde_json::from_str::<Value>(text) {
        if let Some(msg) = v.get("message").and_then(Value::as_str) {
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
    fn token_request_is_form_encoded_not_query() {
        // Regression guard for the HTTP 411 bug: the refresh-token params must travel in a
        // form-encoded body (with a Content-Length), never in the query string. A body-less POST
        // is rejected by Questrade's server with "411 Length Required" before the token is checked.
        let client = reqwest::Client::new();
        let req = build_token_request(&client, "my-refresh-token").unwrap();

        assert_eq!(req.method(), reqwest::Method::POST);
        assert_eq!(req.url().as_str(), TOKEN_URL);
        assert!(
            req.url().query().is_none(),
            "params must not be in the query string"
        );
        assert_eq!(
            req.headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok()),
            Some("application/x-www-form-urlencoded"),
        );
        let body = req
            .body()
            .and_then(|b| b.as_bytes())
            .map(|b| String::from_utf8_lossy(b).into_owned())
            .expect("request must carry a body");
        assert!(body.contains("grant_type=refresh_token"), "body: {body}");
        assert!(body.contains("refresh_token=my-refresh-token"), "body: {body}");
    }

    #[test]
    fn parses_token_response_and_keeps_rotated_refresh_token() {
        let v = json!({
            "access_token": "abc123",
            "token_type": "Bearer",
            "expires_in": 1800,
            "refresh_token": "new-refresh",
            "api_server": "https://api01.iq.questrade.com/"
        });
        let t = parse_tokens(&v).unwrap();
        assert_eq!(t.access_token, "abc123");
        assert_eq!(t.api_server, "https://api01.iq.questrade.com/");
        assert_eq!(t.refresh_token, "new-refresh");
        assert_eq!(t.expires_in, 1800);
    }

    #[test]
    fn token_response_missing_fields_is_an_error() {
        assert!(parse_tokens(&json!({ "access_token": "a", "api_server": "https://x/" })).is_err());
        assert!(parse_tokens(
            &json!({ "access_token": "a", "refresh_token": "r", "api_server": "not a url" })
        )
        .is_err());
    }

    #[test]
    fn parses_accounts() {
        let v = json!({
            "accounts": [
                { "type": "TFSA", "number": "26598145", "status": "Active" },
                { "number": "11111111" }
            ],
            "userId": 3000124
        });
        let accounts: Vec<QuestradeAccount> = v
            .get("accounts")
            .and_then(Value::as_array)
            .unwrap()
            .iter()
            .filter_map(parse_account)
            .collect();
        assert_eq!(accounts.len(), 2);
        assert_eq!(accounts[0].number, "26598145");
        assert_eq!(accounts[0].account_type, "TFSA");
        assert_eq!(accounts[0].status.as_deref(), Some("Active"));
        // Missing type defaults to empty; missing status is None.
        assert_eq!(accounts[1].account_type, "");
        assert_eq!(accounts[1].status, None);
    }

    #[test]
    fn account_without_number_is_skipped() {
        assert!(parse_account(&json!({ "type": "TFSA" })).is_none());
    }

    #[test]
    fn parses_balances_with_total_equity() {
        let v = json!({
            "perCurrencyBalances": [
                { "currency": "CAD", "cash": 1519.56, "marketValue": 0.0, "totalEquity": 1519.56 }
            ],
            "combinedBalances": [
                { "currency": "CAD", "cash": 1519.56, "marketValue": 24480.44, "totalEquity": 26000.0 },
                { "currency": "USD", "cash": 1100.0, "marketValue": 17700.0, "totalEquity": 18800.0 }
            ]
        });
        let arr = v.get("combinedBalances").and_then(Value::as_array).unwrap();
        let balances: Vec<QuestradeBalance> = arr.iter().filter_map(parse_balance).collect();
        assert_eq!(balances.len(), 2);
        assert_eq!(balances[0].currency, "CAD");
        assert_eq!(balances[0].total_equity, Some(26000.0));
        assert_eq!(balances[0].market_value, Some(24480.44));
    }

    #[test]
    fn parses_positions() {
        let v = json!({
            "positions": [{
                "symbol": "VFV.TO",
                "symbolId": 38738,
                "openQuantity": 100.0,
                "currentMarketValue": 12000.0,
                "currentPrice": 120.0,
                "averageEntryPrice": 95.5,
                "isRealTime": true
            }]
        });
        let arr = v.get("positions").and_then(Value::as_array).unwrap();
        let positions: Vec<QuestradePosition> = arr.iter().filter_map(parse_position).collect();
        assert_eq!(positions.len(), 1);
        let p = &positions[0];
        assert_eq!(p.symbol, "VFV.TO");
        assert_eq!(p.open_quantity, 100.0);
        assert_eq!(p.current_price, Some(120.0));
        assert_eq!(p.average_entry_price, Some(95.5));
        assert_eq!(p.current_market_value, Some(12000.0));
    }

    #[test]
    fn numeric_strings_parse_like_numbers() {
        assert_eq!(parse_decimal(&json!("1519.56")), Some(1519.56));
        assert_eq!(parse_decimal(&json!(26000.0)), Some(26000.0));
        assert_eq!(parse_decimal(&json!("nope")), None);
    }

    #[test]
    fn extracts_error_message_from_questrade_body() {
        assert_eq!(
            extract_error_message(r#"{ "code": 1017, "message": "Access token is invalid" }"#),
            "Access token is invalid"
        );
        assert_eq!(extract_error_message(""), "no response body");
        assert_eq!(extract_error_message("  raw  "), "raw");
    }

    #[test]
    fn auth_errors_are_flagged() {
        assert!(QuestradeError::Auth {
            status: 400,
            message: "bad".into()
        }
        .is_auth());
        assert!(QuestradeError::Api {
            status: 401,
            message: "x".into()
        }
        .is_auth());
        assert!(!QuestradeError::Api {
            status: 500,
            message: "x".into()
        }
        .is_auth());
    }
}
