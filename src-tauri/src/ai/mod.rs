//! Model-agnostic AI layer — the "second brain" advisor.
//!
//! Ollama uses the OpenAI-compatible transport in this module. GitHub Copilot uses its official
//! SDK in [`copilot`], which authenticates against the user's Copilot subscription and runs in an
//! isolated mode with only TrueNorth's read-only finance tools.
//!
//! The financial context that grounds each answer is assembled by [`crate::commands::ai`] from the
//! user's own local database and sent as a system message; this module owns provider transports.

use std::time::Duration;

use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub mod copilot;

/// Default local Ollama OpenAI-compatible base URL.
pub const OLLAMA_DEFAULT_BASE: &str = "http://localhost:11434/v1";
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

// ---------------------------------------------------------------------------
// Tool-calling (function-calling) wire types
//
// The agentic advisor lets the model pull specific financial data on demand instead of working
// from one fixed snapshot. These mirror Ollama's OpenAI-compatible tool-calling schema.
// ---------------------------------------------------------------------------

/// A tool the model may call, advertised in the request. `parameters` is a JSON-Schema object.
#[derive(Debug, Clone, Serialize)]
pub struct ToolDef {
    #[serde(rename = "type")]
    pub kind: &'static str, // always "function"
    pub function: FunctionSchema,
}

#[derive(Debug, Clone, Serialize)]
pub struct FunctionSchema {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

impl ToolDef {
    /// Build a function tool from a name, description, and JSON-Schema parameter object.
    pub fn function(
        name: impl Into<String>,
        description: impl Into<String>,
        parameters: serde_json::Value,
    ) -> Self {
        Self {
            kind: "function",
            function: FunctionSchema {
                name: name.into(),
                description: description.into(),
                parameters,
            },
        }
    }
}

/// One tool call the model requested in an assistant turn.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    #[serde(default)]
    pub id: String,
    #[serde(rename = "type", default = "default_tool_type")]
    pub kind: String,
    pub function: FunctionCall,
}

fn default_tool_type() -> String {
    "function".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionCall {
    pub name: String,
    /// Raw JSON string of arguments, exactly as the model produced it.
    #[serde(default)]
    pub arguments: String,
}

/// A message in the OpenAI tool-calling schema. Unlike [`ChatMessage`] (the simple role/content
/// pair exchanged with the frontend), this carries the extra fields needed to drive a tool loop:
/// an assistant turn's `tool_calls`, and a `tool` turn's `tool_call_id` + `name`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WireMessage {
    pub role: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

impl WireMessage {
    pub fn system(content: impl Into<String>) -> Self {
        Self::text("system", content)
    }

    fn text(role: &str, content: impl Into<String>) -> Self {
        Self {
            role: role.into(),
            content: Some(content.into()),
            tool_calls: None,
            tool_call_id: None,
            name: None,
        }
    }

    /// A `tool` result message answering a specific `tool_call_id`.
    pub fn tool_result(tool_call_id: impl Into<String>, name: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: "tool".into(),
            content: Some(content.into()),
            tool_calls: None,
            tool_call_id: Some(tool_call_id.into()),
            name: Some(name.into()),
        }
    }
}

impl From<&ChatMessage> for WireMessage {
    fn from(m: &ChatMessage) -> Self {
        WireMessage::text(&m.role, m.content.clone())
    }
}

#[derive(Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: &'a [ChatMessage],
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
}

#[derive(Serialize)]
struct ToolChatRequest<'a> {
    model: &'a str,
    messages: &'a [WireMessage],
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<&'a [ToolDef]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<&'static str>,
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
        // Ollama is restricted to loopback; never proxy or redirect financial prompts off-device.
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        // LLM responses can take a while; give them room but don't hang forever.
        .timeout(Duration::from_secs(120))
        .build()
        .map_err(AiError::from)
}

/// Turn a non-2xx provider response into an actionable message.
fn friendly_http_error(status: StatusCode, body: &str) -> String {
    // Ollama "model 'x' not found" (404) just means the model isn't pulled locally.
    if let Some(msg) = ollama_missing_model_message(body) {
        return msg;
    }
    let snippet: String = body.chars().take(300).collect();
    match status.as_u16() {
        401 | 403 => format!("The AI provider rejected the request (HTTP {status}). {snippet}"),
        404 => format!(
            "Model or endpoint not found (HTTP {status}). Check the selected model id. {snippet}"
        ),
        410 => format!(
            "The AI provider reported this endpoint has been retired (HTTP {status}). {snippet}"
        ),
        429 => format!(
            "Rate limited by the AI provider (HTTP {status}). Wait a moment and try again."
        ),
        500..=599 => format!("The AI provider had a server error (HTTP {status}). {snippet}"),
        _ => format!("AI provider error (HTTP {status}). {snippet}"),
    }
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

/// POST to an OpenAI-compatible `/chat/completions` endpoint with `tools` advertised, returning the
/// full assistant message — which may carry `tool_calls` instead of (or alongside) `content`. The
/// caller drives the agentic loop: execute any requested calls, append the results as `tool`
/// messages, and call again until the model returns a plain answer. Passing an empty `tools` slice
/// makes this a normal completion (no tool advertising).
pub async fn chat_completion_tools(
    base_url: &str,
    api_key: Option<&str>,
    model: &str,
    messages: &[WireMessage],
    tools: &[ToolDef],
) -> Result<WireMessage, AiError> {
    let url = format!("{}/chat/completions", base_url.trim_end_matches('/'));
    let tools_opt = if tools.is_empty() { None } else { Some(tools) };
    let mut req = http_client()?.post(&url).json(&ToolChatRequest {
        model,
        messages,
        tools: tools_opt,
        tool_choice: tools_opt.map(|_| "auto"),
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

    let parsed: ToolChatCompletion = serde_json::from_str(&body)
        .map_err(|e| AiError::Message(format!("Unexpected response from the model API: {e}")))?;
    parsed
        .choices
        .into_iter()
        .next()
        .map(|c| c.message)
        .ok_or_else(|| AiError::Message("The model returned no choices.".into()))
}

#[derive(Deserialize)]
struct ToolChatCompletion {
    choices: Vec<ToolChoice>,
}

#[derive(Deserialize)]
struct ToolChoice {
    message: WireMessage,
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
            "Couldn't reach Ollama. Make sure it is running (`ollama serve`)."
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

}
