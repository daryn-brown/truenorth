//! GitHub Copilot SDK transport for the finance advisor.
//!
//! The SDK runs a bundled, version-matched Copilot CLI process. TrueNorth starts it in
//! `ClientMode::Empty`, which disables ambient coding-agent capabilities, memory, and telemetry.
//! Each advisor session explicitly exposes only TrueNorth's read-only finance tools and is
//! permanently deleted after the answer.

use std::path::Path;
use std::time::Duration;

use github_copilot_sdk::{
    Client, ClientMode, ClientOptions, InfiniteSessionConfig, MessageOptions, SessionConfig,
    SystemMessageConfig, Tool, ToolSet,
};
use sha2::{Digest, Sha256};
use tokio::sync::Mutex;

use super::{AiError, ModelInfo};

pub const DEFAULT_COPILOT_MODEL: &str = "auto";
pub const RUNTIME_STATE_DIR_NAME: &str = "copilot-runtime";
const AUTH_GUIDANCE: &str =
    "If authentication failed, run `gh auth login` with the GitHub account that has your Copilot \
     subscription.";

/// Lazily started Copilot runtime shared by advisor requests.
#[derive(Default)]
pub struct CopilotRuntime {
    client: Mutex<Option<ActiveClient>>,
}

struct ActiveClient {
    client: Client,
    token_fingerprint: [u8; 32],
}

/// Remove the SDK's private state before authentication so an interrupted prior run cannot leave
/// financial prompts on disk merely because the user later logged out or lost entitlement.
pub fn clear_runtime_state(state_dir: &Path) -> Result<(), AiError> {
    let metadata = match std::fs::symlink_metadata(state_dir) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(AiError::Message(format!(
                "Could not inspect the private Copilot runtime directory at {}: {error}",
                state_dir.display()
            )));
        }
    };
    if metadata.file_type().is_symlink() {
        return Err(AiError::Message(format!(
            "Refusing to clean the private Copilot runtime directory because {} is a symbolic link.",
            state_dir.display()
        )));
    }
    let result = if metadata.is_dir() {
        std::fs::remove_dir_all(state_dir)
    } else {
        std::fs::remove_file(state_dir)
    };
    result.map_err(|error| {
        AiError::Message(format!(
            "Could not clear the private Copilot runtime directory at {}: {error}",
            state_dir.display()
        ))
    })
}

impl CopilotRuntime {
    /// Return the active client, starting the bundled runtime with the supplied GitHub token when
    /// this is the first Copilot request in the app process. A changed local `gh` login replaces the
    /// old runtime rather than continuing to use the previous account.
    pub async fn connect(&self, github_token: &str, state_dir: &Path) -> Result<Client, AiError> {
        let token_fingerprint: [u8; 32] = Sha256::digest(github_token.as_bytes()).into();
        let mut slot = self.client.lock().await;
        if let Some(active) = slot.as_ref() {
            if active.token_fingerprint == token_fingerprint {
                return Ok(active.client.clone());
            }
        }
        if let Some(active) = slot.take() {
            let cleanup_errors = shutdown_and_clear(&active.client, state_dir).await;
            if !cleanup_errors.is_empty() {
                return Err(AiError::Message(format!(
                    "The GitHub login changed, but the previous Copilot runtime could not be \
                     replaced cleanly: {}",
                    cleanup_errors.join("; ")
                )));
            }
        }

        std::fs::create_dir_all(state_dir).map_err(|e| {
            AiError::Message(format!(
                "Could not create the private Copilot runtime directory at {}: {e}",
                state_dir.display()
            ))
        })?;

        let client = match Client::start(
            ClientOptions::new()
                .with_github_token(github_token)
                .with_use_logged_in_user(false)
                .with_base_directory(state_dir)
                .with_mode(ClientMode::Empty)
                .with_session_idle_timeout_seconds(300),
        )
        .await
        {
            Ok(client) => client,
            Err(error) => {
                let primary = copilot_error("Could not start GitHub Copilot", error);
                return Err(with_cleanup_error(
                    primary,
                    clear_runtime_state(state_dir).err(),
                ));
            }
        };

        let auth = match client.get_auth_status().await {
            Ok(auth) => auth,
            Err(error) => {
                let primary = copilot_error("Could not check GitHub Copilot authentication", error);
                return Err(with_cleanup_errors(
                    primary,
                    shutdown_and_clear(&client, state_dir).await,
                ));
            }
        };
        if !auth.is_authenticated {
            let detail = auth
                .status_message
                .unwrap_or_else(|| "GitHub did not accept the current login.".to_string());
            let primary = AiError::Message(format!(
                "GitHub Copilot is not authenticated. {detail} Run `gh auth login` with the GitHub \
                 account that has your Copilot subscription, then try again."
            ));
            return Err(with_cleanup_errors(
                primary,
                shutdown_and_clear(&client, state_dir).await,
            ));
        }
        if let Err(error) = delete_persisted_sessions(&client).await {
            return Err(with_cleanup_errors(
                error,
                shutdown_and_clear(&client, state_dir).await,
            ));
        }

        *slot = Some(ActiveClient {
            client: client.clone(),
            token_fingerprint,
        });
        Ok(client)
    }
}

async fn shutdown_and_clear(client: &Client, state_dir: &Path) -> Vec<String> {
    let mut errors = Vec::new();
    if let Err(error) = client.stop().await {
        client.force_stop();
        errors.push(format!("runtime shutdown failed: {error}"));
    }
    if let Err(error) = clear_runtime_state(state_dir) {
        errors.push(error.to_string());
    }
    errors
}

fn with_cleanup_error(primary: AiError, cleanup_error: Option<AiError>) -> AiError {
    with_cleanup_errors(
        primary,
        cleanup_error
            .into_iter()
            .map(|error| error.to_string())
            .collect(),
    )
}

fn with_cleanup_errors(primary: AiError, cleanup_errors: Vec<String>) -> AiError {
    if cleanup_errors.is_empty() {
        primary
    } else {
        AiError::Message(format!(
            "{primary} Private Copilot runtime cleanup also failed: {}",
            cleanup_errors.join("; ")
        ))
    }
}

/// Verify authentication and return the account plus the number of models available to it.
pub async fn status(client: &Client) -> Result<(Option<String>, usize), AiError> {
    let auth = client
        .get_auth_status()
        .await
        .map_err(|e| copilot_error("Could not check GitHub Copilot authentication", e))?;
    if !auth.is_authenticated {
        return Err(AiError::Message(auth.status_message.unwrap_or_else(|| {
            "GitHub Copilot is not authenticated.".to_string()
        })));
    }
    let models = client
        .list_models()
        .await
        .map_err(|e| copilot_error("Could not load GitHub Copilot models", e))?;
    Ok((auth.login, models.len()))
}

/// List models available through the authenticated Copilot subscription.
pub async fn list_models(client: &Client) -> Result<Vec<ModelInfo>, AiError> {
    let mut models: Vec<ModelInfo> = client
        .list_models()
        .await
        .map_err(|e| copilot_error("Could not load GitHub Copilot models", e))?
        .into_iter()
        .map(|model| ModelInfo {
            id: model.id,
            name: model.name,
        })
        .collect();
    models.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    Ok(models)
}

/// Run one isolated advisor turn. The caller supplies the complete financial system prompt and any
/// read-only custom tools. The transient SDK session is deleted after each turn, so TrueNorth's
/// encrypted chat tables remain the only durable conversation history.
pub async fn complete(
    client: &Client,
    configured_model: &str,
    system_message: String,
    prompt: String,
    tools: Vec<Tool>,
) -> Result<String, AiError> {
    let model = resolve_model(client, configured_model).await?;
    let available_tools = ToolSet::new()
        .add_custom("*")
        .map_err(|e| copilot_error("Could not configure Copilot tools", e))?
        .into_vec();
    let excluded_tools = ToolSet::new()
        .add_builtin("*")
        .and_then(|set| set.add_mcp("*"))
        .map_err(|e| copilot_error("Could not isolate Copilot tools", e))?
        .into_vec();

    let mut config = SessionConfig::default();
    config.model = model;
    config.client_name = Some("TrueNorth".to_string());
    config.system_message = Some(
        SystemMessageConfig::new()
            .with_mode("replace")
            .with_content(system_message),
    );
    config.tools = Some(tools);
    config.available_tools = Some(available_tools);
    config.excluded_tools = Some(excluded_tools);
    config.enable_config_discovery = Some(false);
    config.enable_session_store = Some(false);
    config.enable_session_telemetry = Some(false);
    config.enable_skills = Some(false);
    config.enable_file_hooks = Some(false);
    config.enable_host_git_operations = Some(false);
    config.infinite_sessions = Some(InfiniteSessionConfig::new().with_enabled(false));

    let session = client
        .create_session(config)
        .await
        .map_err(|e| copilot_error("Could not create the GitHub Copilot advisor session", e))?;
    let session_id = session.id().clone();

    let response = session
        .send_and_wait(MessageOptions::new(prompt).with_wait_timeout(Duration::from_secs(180)))
        .await;
    let mut cleanup_errors = Vec::new();
    if let Err(error) = session.disconnect().await {
        cleanup_errors.push(format!("disconnect failed: {error}"));
    }
    if let Err(error) = client.delete_session(&session_id).await {
        cleanup_errors.push(format!("on-disk deletion failed: {error}"));
    }
    let cleanup_detail = cleanup_errors.join("; ");

    let event = match response {
        Ok(event) if cleanup_errors.is_empty() => event,
        Ok(_) => {
            return Err(AiError::Message(format!(
                "GitHub Copilot answered, but its private advisor session could not be cleaned up \
                 completely ({cleanup_detail}). The response was discarded."
            )));
        }
        Err(error) if cleanup_errors.is_empty() => {
            return Err(copilot_error(
                "GitHub Copilot could not answer the question",
                error,
            ));
        }
        Err(error) => {
            return Err(AiError::Message(format!(
                "GitHub Copilot could not answer the question: {error}. Session cleanup also failed \
                 ({cleanup_detail}). {AUTH_GUIDANCE}"
            )));
        }
    };

    let content = event
        .and_then(|event| {
            event
                .data
                .get("content")
                .and_then(|value| value.as_str())
                .map(str::to_owned)
        })
        .unwrap_or_default();
    if content.trim().is_empty() {
        return Err(AiError::Message(
            "GitHub Copilot returned an empty response. Try again or select another model.".into(),
        ));
    }
    Ok(content)
}

async fn delete_persisted_sessions(client: &Client) -> Result<(), AiError> {
    let sessions = client
        .list_sessions(None)
        .await
        .map_err(|e| copilot_error("Could not inspect prior GitHub Copilot session data", e))?;
    let mut failures = Vec::new();
    for session in sessions {
        if let Err(error) = client.delete_session(&session.session_id).await {
            failures.push(error.to_string());
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(AiError::Message(format!(
            "Could not delete {} prior GitHub Copilot advisor session(s): {}",
            failures.len(),
            failures.join("; ")
        )))
    }
}

async fn resolve_model(client: &Client, configured: &str) -> Result<Option<String>, AiError> {
    let configured = configured.trim();
    if configured.is_empty() || configured == DEFAULT_COPILOT_MODEL {
        return Ok(None);
    }

    let available = client
        .list_models()
        .await
        .map_err(|e| copilot_error("Could not verify the selected GitHub Copilot model", e))?;
    if available.iter().any(|model| model.id == configured) {
        Ok(Some(configured.to_string()))
    } else {
        Err(AiError::Message(format!(
            "The GitHub Copilot model `{configured}` is not available to this account. Choose \
             \"auto\" or load an available model in AI settings."
        )))
    }
}

fn copilot_error(context: &str, error: impl std::fmt::Display) -> AiError {
    AiError::Message(format!("{context}: {error}. {AUTH_GUIDANCE}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_state_cleanup_removes_nested_session_data() {
        let unique = format!(
            "truenorth-copilot-cleanup-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let state_dir = std::env::temp_dir().join(unique);
        let nested = state_dir.join("session-state");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("history.json"), b"sensitive").unwrap();

        clear_runtime_state(&state_dir).unwrap();

        assert!(!state_dir.exists());
        clear_runtime_state(&state_dir).unwrap();
    }
}
