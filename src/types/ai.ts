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

/** One tool the advisor invoked while answering, surfaced as a "Used tool" step. */
export interface ToolStep {
  name: string;
  /** Raw JSON arguments the model passed. */
  arguments: string;
  /** JSON result returned to the model (truncated for display). */
  result: string;
}

export interface ChatMessage {
  role: ChatRole;
  content: string;
  /** Tool-call trace for assistant turns; absent for user/system messages. */
  steps?: ToolStep[];
}

/** Reply from `ai_chat`: the assistant's answer plus the tools it called to get there. */
export interface AiChatResponse {
  role: ChatRole;
  content: string;
  steps: ToolStep[];
}

/** A saved conversation (mirrors the Rust `ChatThread`). */
export interface ChatThread {
  id: number;
  title: string;
  created_at: string;
  updated_at: string;
}

/** A persisted message loaded from a thread (mirrors the Rust `StoredMessage`). */
export interface StoredMessage {
  id: number;
  role: ChatRole;
  content: string;
  steps: ToolStep[];
  created_at: string;
}

export interface ModelInfo {
  id: string;
  name: string;
}
