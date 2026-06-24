// Types for the AI "second brain" advisor. Field names mirror the Rust structs in
// `src-tauri/src/commands/ai.rs` (serde serialises them as snake_case).

export type AiProvider = "github" | "ollama";

export interface AiSettings {
  provider: AiProvider;
  github_model: string;
  ollama_model: string;
  ollama_url: string;
  /** When true, exact balances/transactions are sent to the model; false sends rounded aggregates. */
  include_real_data: boolean;
  /** Whether a GitHub Models token is stored (the token itself is never returned to the UI). */
  has_github_token: boolean;
}

/** Shape accepted by `ai_save_settings` (no token, no derived `has_github_token`). */
export interface AiSettingsInput {
  provider: AiProvider;
  github_model: string;
  ollama_model: string;
  ollama_url: string;
  include_real_data: boolean;
}

export type ChatRole = "system" | "user" | "assistant";

export interface ChatMessage {
  role: ChatRole;
  content: string;
}

export interface ModelInfo {
  id: string;
  name: string;
}
