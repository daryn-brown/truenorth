//! Model-agnostic AI layer — the "second brain" advisor.
//!
//! A single OpenAI-compatible client talks to whichever provider the user picked:
//! * **GitHub Models** (`https://models.github.ai/inference`) — free frontier models via a GitHub
//!   PAT with the `models:read` scope, used as the bearer token. Model ids are `publisher/model`
//!   (e.g. `openai/gpt-4o-mini`).
//! * **Ollama** (`http://localhost:11434/v1`) — a local, fully-private fallback; no token.
//!
//! The same `/chat/completions` request shape works for both. The financial context that grounds
//! each answer is assembled by [`crate::commands::ai`] from the user's own local database and sent
//! as a system message; this module only owns the transport and the provider quirks.

use std::time::Duration;

use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// GitHub Models OpenAI-compatible inference base URL.
pub const GITHUB_MODELS_BASE: &str = "https://models.github.ai/inference";
/// GitHub Models catalog endpoint (lists available `publisher/model` ids).
pub const GITHUB_MODELS_CATALOG: &str = "https://models.github.ai/catalog/models";
/// Default local Ollama OpenAI-compatible base URL.
pub const OLLAMA_DEFAULT_BASE: &str = "http://localhost:11434/v1";
/// Sensible default model for each provider.
pub const DEFAULT_GITHUB_MODEL: &str = "openai/gpt-4o-mini";
pub const DEFAULT_OLLAMA_MODEL: &str = "llama3.1";

#[derive(Debug, Error)]
pub enum AiError {
    #[error("Network error talking to the AI provider: {0}")]
    Http(#[from] reqwest::Error),

    #[error("{0}")]
    Message(String),
}

/// One chat message in the OpenAI-compatible schema. `role` is "system", "user", or "assistant".
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

impl ChatMessage {
    pub fn system(content: impl Into<String>) -> Self {
        Self { role: "system".into(), content: content.into() }
    }
}

/// A model id + display name, for the picker.
#[derive(Debug, Clone, Serialize)]
pub struct ModelInfo {
    pub id: String,
    pub name: String,
}

#[derive(Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: &'a [ChatMessage],
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
}

#[derive(Deserialize)]
struct ChatCompletion {
    choices: Vec<Choice>,
}

#[derive(Deserialize)]
struct Choice {
    message: ChoiceMessage,
}

#[derive(Deserialize)]
struct ChoiceMessage {
    content: Option<String>,
}

fn http_client() -> Result<Client, AiError> {
    Client::builder()
        // LLM responses can take a while; give them room but don't hang forever.
        .timeout(Duration::from_secs(120))
        .build()
        .map_err(AiError::from)
}

/// Turn a non-2xx provider response into an actionable message.
fn friendly_http_error(status: StatusCode, body: &str) -> String {
    // GitHub Models "no_access" (usually 403) means the token in use can't reach the model, so it
    // gets a token-focused message regardless of the status code.
    if let Some(msg) = no_access_message(body) {
        return msg;
    }
    // Ollama "model 'x' not found" (404) just means the model isn't pulled locally.
    if let Some(msg) = ollama_missing_model_message(body) {
        return msg;
    }
    let snippet: String = body.chars().take(300).collect();
    match status.as_u16() {
        401 | 403 => format!(
            "The AI provider rejected the request (HTTP {status}). For GitHub Models, check that \
             your token is valid and has the `models:read` scope. {snippet}"
        ),
        404 => format!(
            "Model or endpoint not found (HTTP {status}). Check the selected model id. {snippet}"
        ),
        429 => format!(
            "Rate limited by the AI provider (HTTP {status}). The GitHub Models free tier has \
             per-minute and per-day limits — wait a moment and try again, or switch models."
        ),
        500..=599 => format!("The AI provider had a server error (HTTP {status}). {snippet}"),
        _ => format!("AI provider error (HTTP {status}). {snippet}"),
    }
}

/// Detect GitHub Models "no access" responses (usually HTTP 403, `code: no_access`). In practice
/// this means the *token* in use can't reach GitHub Models, not that the account lacks a tier, so
/// the message points at fixing the token. Returns `None` for any other error.
fn no_access_message(body: &str) -> Option<String> {
    if !body.contains("no_access") && !body.contains("No access to model") {
        return None;
    }
    // Best-effort: pull the model id out of "No access to model: <id>".
    let model = body
        .split("No access to model:")
        .nth(1)
        .and_then(|rest| rest.split(['"', '}']).next())
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let target = match model {
        Some(m) => format!("the model `{m}`"),
        None => "that model".to_string(),
    };
    Some(format!(
        "GitHub Models rejected {target} with `no_access`. This almost always means the GitHub \
         token in use doesn't have GitHub Models access (not a tier limit on your account). In \
         Settings, click \"Use my GitHub CLI login\" for a token that works automatically, or paste \
         a token from an account that can use this model."
    ))
}

/// Detect Ollama's "model 'x' not found" (HTTP 404) and tell the user to pull it. Returns `None`
/// when the body doesn't carry an extractable model name.
fn ollama_missing_model_message(body: &str) -> Option<String> {
    if !body.contains("not found") || !body.contains("model") {
        return None;
    }
    // Body looks like: {"error":{"message":"model 'llama3.1' not found", ...}}
    let model = body
        .split("model")
        .nth(1)
        .and_then(|rest| rest.split('\'').nth(1))
        .map(str::trim)
        .filter(|s| !s.is_empty())?;
    Some(format!(
        "Ollama doesn't have the model `{model}` installed. Run `ollama pull {model}` in a terminal \
         (or pick an already-installed model in Settings), then try again."
    ))
}

/// POST to an OpenAI-compatible `/chat/completions` endpoint and return the assistant's text.
pub async fn chat_completion(
    base_url: &str,
    api_key: Option<&str>,
    model: &str,
    messages: &[ChatMessage],
) -> Result<String, AiError> {
    let url = format!("{}/chat/completions", base_url.trim_end_matches('/'));
    let mut req = http_client()?.post(&url).json(&ChatRequest {
        model,
        messages,
        temperature: Some(0.2),
    });
    if let Some(key) = api_key {
        req = req.bearer_auth(key);
    }

    let resp = req.send().await.map_err(map_connect_error)?;
    let status = resp.status();
    let body = resp.text().await?;
    if !status.is_success() {
        return Err(AiError::Message(friendly_http_error(status, &body)));
    }

    let parsed: ChatCompletion = serde_json::from_str(&body)
        .map_err(|e| AiError::Message(format!("Unexpected response from the model API: {e}")))?;
    let content = parsed
        .choices
        .into_iter()
        .next()
        .and_then(|c| c.message.content)
        .unwrap_or_default();
    if content.trim().is_empty() {
        return Err(AiError::Message("The model returned an empty response.".into()));
    }
    Ok(content)
}

/// List available GitHub Models from the catalog (best-effort; used to populate the picker).
pub async fn list_github_models(api_key: &str) -> Result<Vec<ModelInfo>, AiError> {
    let resp = http_client()?
        .get(GITHUB_MODELS_CATALOG)
        .bearer_auth(api_key)
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(map_connect_error)?;
    let status = resp.status();
    let body = resp.text().await?;
    if !status.is_success() {
        return Err(AiError::Message(friendly_http_error(status, &body)));
    }
    let raw: serde_json::Value =
        serde_json::from_str(&body).map_err(|e| AiError::Message(e.to_string()))?;
    // The catalog is an array of model objects; be tolerant about the exact field names.
    let items = raw.as_array().cloned().unwrap_or_default();
    let mut models = Vec::new();
    for item in items {
        if let Some(id) = item.get("id").and_then(|v| v.as_str()) {
            let name = item
                .get("name")
                .and_then(|v| v.as_str())
                .or_else(|| item.get("friendly_name").and_then(|v| v.as_str()))
                .unwrap_or(id);
            models.push(ModelInfo { id: id.to_string(), name: name.to_string() });
        }
    }
    Ok(models)
}

/// List locally-installed Ollama models via its OpenAI-compatible `/models` endpoint.
pub async fn list_ollama_models(base_url: &str) -> Result<Vec<ModelInfo>, AiError> {
    let url = format!("{}/models", base_url.trim_end_matches('/'));
    let resp = http_client()?.get(&url).send().await.map_err(map_connect_error)?;
    let status = resp.status();
    let body = resp.text().await?;
    if !status.is_success() {
        return Err(AiError::Message(friendly_http_error(status, &body)));
    }
    let raw: serde_json::Value =
        serde_json::from_str(&body).map_err(|e| AiError::Message(e.to_string()))?;
    let items = raw
        .get("data")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let mut models = Vec::new();
    for item in items {
        if let Some(id) = item.get("id").and_then(|v| v.as_str()) {
            models.push(ModelInfo { id: id.to_string(), name: id.to_string() });
        }
    }
    Ok(models)
}

/// Give a clearer message for the common "Ollama isn't running" / connection-refused case.
fn map_connect_error(e: reqwest::Error) -> AiError {
    if e.is_connect() {
        AiError::Message(
            "Couldn't reach the AI provider. If you're using Ollama, make sure it's running \
             (`ollama serve`). For GitHub Models, check your internet connection."
                .into(),
        )
    } else if e.is_timeout() {
        AiError::Message(
            "The AI provider timed out. Try again or pick a smaller/faster model.".into(),
        )
    } else {
        AiError::Http(e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_access_message_extracts_model_id() {
        let body = r#"{"error":{"code":"no_access","message":"No access to model: openai/gpt-5","details":"No access to model: openai/gpt-5"}}"#;
        let msg = no_access_message(body).expect("should detect no_access");
        assert!(msg.contains("openai/gpt-5"), "names the gated model: {msg}");
        assert!(
            msg.contains("GitHub CLI login"),
            "points at the token fix: {msg}"
        );
    }

    #[test]
    fn no_access_message_handles_unknown_model_shape() {
        let body = r#"{"error":{"code":"no_access"}}"#;
        let msg = no_access_message(body).expect("should detect no_access by code");
        assert!(msg.contains("that model"), "falls back gracefully: {msg}");
    }

    #[test]
    fn ollama_missing_model_message_extracts_and_suggests_pull() {
        let body = r#"{"error":{"message":"model 'llama3.1' not found","type":"not_found_error"}}"#;
        let msg = ollama_missing_model_message(body).expect("should detect missing model");
        assert!(msg.contains("llama3.1"), "names the missing model: {msg}");
        assert!(msg.contains("ollama pull llama3.1"), "suggests the pull command: {msg}");
    }

    #[test]
    fn ollama_missing_model_message_ignores_other_errors() {
        assert!(ollama_missing_model_message(r#"{"error":{"message":"bad request"}}"#).is_none());
        assert!(ollama_missing_model_message("").is_none());
    }

    #[test]
    fn no_access_message_ignores_other_errors() {
        assert!(no_access_message(r#"{"error":{"code":"rate_limited"}}"#).is_none());
        assert!(no_access_message("").is_none());
    }
}
