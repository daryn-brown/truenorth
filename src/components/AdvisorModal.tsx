import { useCallback, useEffect, useRef, useState } from "react";
import type { AiProvider, AiSettings, ChatMessage, ModelInfo } from "../types/ai";
import MarkdownMessage from "./MarkdownMessage";
import ToolTrace from "./ToolTrace";
import {
  aiChat,
  aiGetSettings,
  aiGithubCliLogin,
  aiListModels,
  aiSaveSettings,
  aiSetGithubToken,
} from "../hooks/useFinanceApi";

interface Props {
  isOpen: boolean;
  onClose: () => void;
}

const inputClass =
  "w-full rounded-lg border border-slate-600 bg-slate-800 px-3 py-2 text-sm text-slate-200 placeholder-slate-600 focus:outline-none focus:ring-2 focus:ring-indigo-500";

const GITHUB_TOKEN_SETTINGS = "https://github.com/settings/tokens";

const SUGGESTIONS = [
  "How am I tracking toward my $100k goal?",
  "What's my savings rate, and where is most of my money going?",
  "Summarize my net worth across USD and CAD.",
  "Which spending should I watch this month?",
];

// Free, broadly-available default model, offered as a one-click reset in Settings.
const RECOMMENDED_GITHUB_MODEL = "openai/gpt-4o-mini";

/**
 * True when a GitHub error looks like the token can't reach GitHub Models (no_access, a rejected
 * request, or a missing scope). The fix is a working token, which the one-click GitHub CLI login
 * provides without creating or pasting a PAT.
 */
const isGithubAccessError = (msg: string) =>
  /no_access|access to (the )?model|models:read|rejected the request|doesn't have access/i.test(
    msg,
  );

export default function AdvisorModal({ isOpen, onClose }: Props) {
  const [settings, setSettings] = useState<AiSettings | null>(null);
  const [showSettings, setShowSettings] = useState(false);
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [input, setInput] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Editable settings draft (mirrors `settings` until saved).
  const [provider, setProvider] = useState<AiProvider>("github");
  const [githubModel, setGithubModel] = useState("");
  const [ollamaModel, setOllamaModel] = useState("");
  const [ollamaUrl, setOllamaUrl] = useState("");
  const [includeRealData, setIncludeRealData] = useState(true);
  const [tokenInput, setTokenInput] = useState("");
  const [savingSettings, setSavingSettings] = useState(false);
  const [savingToken, setSavingToken] = useState(false);
  const [cliBusy, setCliBusy] = useState(false);
  const [models, setModels] = useState<ModelInfo[]>([]);
  const [loadingModels, setLoadingModels] = useState(false);

  const scrollRef = useRef<HTMLDivElement>(null);

  const applySettings = useCallback((s: AiSettings) => {
    setSettings(s);
    setProvider(s.provider);
    setGithubModel(s.github_model);
    setOllamaModel(s.ollama_model);
    setOllamaUrl(s.ollama_url);
    setIncludeRealData(s.include_real_data);
  }, []);

  // Load settings whenever the modal opens.
  useEffect(() => {
    if (!isOpen) return;
    setError(null);
    aiGetSettings()
      .then((s) => {
        applySettings(s);
        // Nudge first-time users straight to setup when GitHub is selected but unconfigured.
        if (s.provider === "github" && !s.has_github_token) setShowSettings(true);
      })
      .catch((e) => setError(String(e)));
  }, [isOpen, applySettings]);

  // Keep the latest message in view.
  useEffect(() => {
    scrollRef.current?.scrollTo({ top: scrollRef.current.scrollHeight, behavior: "smooth" });
  }, [messages, busy]);

  const runChat = useCallback(async (convo: ChatMessage[]) => {
    setBusy(true);
    setError(null);
    try {
      const reply = await aiChat(convo);
      setMessages((m) => [...m, reply]);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }, []);

  const send = useCallback(
    async (text: string) => {
      const content = text.trim();
      if (!content || busy) return;
      const next: ChatMessage[] = [...messages, { role: "user", content }];
      setMessages(next);
      setInput("");
      await runChat(next);
    },
    [messages, busy, runChat],
  );

  // After a failed send the conversation already ends with the user's turn (no assistant reply was
  // appended), so retry just re-runs it as-is.
  const retry = useCallback(async () => {
    if (busy || messages.length === 0) return;
    await runChat(messages);
  }, [busy, messages, runChat]);

  // Reuse the local GitHub CLI session (`gh auth token`) as the token — nothing to create or
  // paste. Returns whether a working token was stored.
  const useGithubCliLogin = useCallback(async () => {
    if (cliBusy) return false;
    setCliBusy(true);
    setError(null);
    try {
      const s = await aiGithubCliLogin();
      applySettings(s);
      return true;
    } catch (e) {
      setError(String(e));
      return false;
    } finally {
      setCliBusy(false);
    }
  }, [cliBusy, applySettings]);

  // One-click recovery from a token/no-access error: grab a CLI token, then retry the last message.
  const cliLoginAndRetry = useCallback(async () => {
    if (await useGithubCliLogin()) await retry();
  }, [useGithubCliLogin, retry]);

  const handleSaveSettings = async () => {
    setSavingSettings(true);
    setError(null);
    try {
      const saved = await aiSaveSettings({
        provider,
        github_model: githubModel,
        ollama_model: ollamaModel,
        ollama_url: ollamaUrl,
        include_real_data: includeRealData,
      });
      applySettings(saved);
      const needsToken = saved.provider === "github" && !saved.has_github_token;
      if (!needsToken) setShowSettings(false);
    } catch (e) {
      setError(String(e));
    } finally {
      setSavingSettings(false);
    }
  };

  const handleSaveToken = async () => {
    setSavingToken(true);
    setError(null);
    try {
      const has = await aiSetGithubToken(tokenInput);
      setTokenInput("");
      setSettings((s) => (s ? { ...s, has_github_token: has } : s));
    } catch (e) {
      setError(String(e));
    } finally {
      setSavingToken(false);
    }
  };

  const handleLoadModels = async () => {
    setLoadingModels(true);
    setError(null);
    try {
      setModels(await aiListModels());
    } catch (e) {
      setError(String(e));
    } finally {
      setLoadingModels(false);
    }
  };

  if (!isOpen) return null;

  const githubReady = settings?.has_github_token ?? false;
  const canChat = provider === "ollama" || githubReady;

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm"
      onClick={(e) => e.target === e.currentTarget && onClose()}
    >
      <div className="flex h-[80vh] w-full max-w-2xl flex-col rounded-2xl border border-slate-700 bg-slate-900 shadow-2xl">
        {/* Header */}
        <div className="flex items-center justify-between border-b border-slate-800 px-5 py-3">
          <div className="flex items-center gap-2">
            <span className="text-base font-semibold text-white">🧠 Finance brain</span>
            <span className="rounded-full border border-slate-700 bg-slate-800 px-2 py-0.5 text-[10px] uppercase tracking-wider text-slate-400">
              {provider === "github" ? "GitHub Models" : "Ollama · local"}
            </span>
          </div>
          <div className="flex items-center gap-1">
            <button
              onClick={() => setShowSettings((v) => !v)}
              title="AI provider settings"
              className="rounded-lg border border-slate-700 px-2.5 py-1 text-xs text-slate-400 hover:bg-slate-800"
            >
              ⚙️ Settings
            </button>
            <button
              onClick={onClose}
              className="rounded-lg border border-slate-700 px-2.5 py-1 text-xs text-slate-400 hover:bg-slate-800"
            >
              ✕
            </button>
          </div>
        </div>

        {error && (
          <div className="mx-5 mt-3 rounded-lg border border-red-700/50 bg-red-900/20 px-3 py-2 text-xs text-red-300">
            <p>{error}</p>
            {provider === "github" && isGithubAccessError(error) && messages.length > 0 && (
              <button
                onClick={() => void cliLoginAndRetry()}
                disabled={busy || cliBusy}
                className="mt-2 rounded-md border border-red-600/60 bg-red-800/40 px-2.5 py-1 font-medium text-red-100 hover:bg-red-800/70 disabled:opacity-50"
              >
                {cliBusy ? "Signing in…" : "Use my GitHub CLI login and retry"}
              </button>
            )}
          </div>
        )}

        {showSettings ? (
          <SettingsPanel
            provider={provider}
            setProvider={setProvider}
            githubModel={githubModel}
            setGithubModel={setGithubModel}
            ollamaModel={ollamaModel}
            setOllamaModel={setOllamaModel}
            ollamaUrl={ollamaUrl}
            setOllamaUrl={setOllamaUrl}
            includeRealData={includeRealData}
            setIncludeRealData={setIncludeRealData}
            tokenInput={tokenInput}
            setTokenInput={setTokenInput}
            hasToken={githubReady}
            onSaveToken={handleSaveToken}
            savingToken={savingToken}
            onUseCliLogin={useGithubCliLogin}
            cliBusy={cliBusy}
            models={models}
            loadingModels={loadingModels}
            onLoadModels={handleLoadModels}
            onSave={handleSaveSettings}
            saving={savingSettings}
          />
        ) : (
          <>
            {/* Messages */}
            <div ref={scrollRef} className="flex-1 space-y-3 overflow-y-auto px-5 py-4">
              {messages.length === 0 ? (
                <div className="flex h-full flex-col items-center justify-center gap-4 text-center">
                  <p className="max-w-sm text-sm text-slate-400">
                    Ask anything about your finances. I read your accounts, net worth, cashflow,
                    goal, holdings, and recent transactions to answer.
                  </p>
                  <div className="grid w-full max-w-md gap-2">
                    {SUGGESTIONS.map((q) => (
                      <button
                        key={q}
                        disabled={!canChat}
                        onClick={() => void send(q)}
                        className="rounded-lg border border-slate-700 bg-slate-800/50 px-3 py-2 text-left text-xs text-slate-300 hover:bg-slate-800 disabled:opacity-40"
                      >
                        {q}
                      </button>
                    ))}
                  </div>
                  {!canChat && (
                    <p className="text-xs text-amber-400">
                      Add a GitHub token in Settings to start (or switch to Ollama).
                    </p>
                  )}
                </div>
              ) : (
                messages.map((m, i) => <Bubble key={i} message={m} />)
              )}
              {busy && (
                <div className="flex items-center gap-2 text-xs text-slate-500">
                  <span className="h-2 w-2 animate-pulse rounded-full bg-indigo-400" />
                  Thinking…
                </div>
              )}
            </div>

            {/* Composer */}
            <div className="border-t border-slate-800 px-5 py-3">
              <div className="flex items-end gap-2">
                <textarea
                  value={input}
                  onChange={(e) => setInput(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter" && !e.shiftKey) {
                      e.preventDefault();
                      void send(input);
                    }
                  }}
                  placeholder={canChat ? "Ask about your money…" : "Configure a provider in Settings first"}
                  rows={1}
                  disabled={!canChat || busy}
                  className={`${inputClass} max-h-32 resize-none disabled:opacity-50`}
                />
                <button
                  onClick={() => void send(input)}
                  disabled={!canChat || busy || !input.trim()}
                  className="rounded-lg bg-indigo-600 px-4 py-2 text-sm font-medium text-white hover:bg-indigo-500 disabled:opacity-40"
                >
                  Send
                </button>
              </div>
              <p className="mt-2 text-[11px] text-slate-500">
                {provider === "github" ? (
                  <>
                    {includeRealData
                      ? "Your real balances & transactions are sent to GitHub Models to answer."
                      : "Privacy mode: only rounded aggregates are sent to GitHub Models."}
                  </>
                ) : (
                  "Running locally via Ollama — nothing leaves your device."
                )}{" "}
                Educational only, not licensed financial advice.
              </p>
            </div>
          </>
        )}
      </div>
    </div>
  );
}

function Bubble({ message }: { message: ChatMessage }) {
  const isUser = message.role === "user";
  if (isUser) {
    return (
      <div className="flex justify-end">
        <div className="max-w-[85%] whitespace-pre-wrap rounded-2xl bg-indigo-600 px-3.5 py-2 text-sm text-white">
          {message.content}
        </div>
      </div>
    );
  }
  return (
    <div className="flex justify-start">
      <div className="max-w-[85%] rounded-2xl border border-slate-700 bg-slate-800 px-3.5 py-2 text-slate-200">
        <MarkdownMessage content={message.content} />
        {message.steps && message.steps.length > 0 && <ToolTrace steps={message.steps} />}
      </div>
    </div>
  );
}

interface SettingsProps {
  provider: AiProvider;
  setProvider: (p: AiProvider) => void;
  githubModel: string;
  setGithubModel: (v: string) => void;
  ollamaModel: string;
  setOllamaModel: (v: string) => void;
  ollamaUrl: string;
  setOllamaUrl: (v: string) => void;
  includeRealData: boolean;
  setIncludeRealData: (v: boolean) => void;
  tokenInput: string;
  setTokenInput: (v: string) => void;
  hasToken: boolean;
  onSaveToken: () => void;
  savingToken: boolean;
  onUseCliLogin: () => void;
  cliBusy: boolean;
  models: ModelInfo[];
  loadingModels: boolean;
  onLoadModels: () => void;
  onSave: () => void;
  saving: boolean;
}

function SettingsPanel(p: SettingsProps) {
  const isGithub = p.provider === "github";
  return (
    <div className="flex-1 space-y-5 overflow-y-auto px-5 py-4">
      {/* Provider */}
      <div>
        <label className="mb-1.5 block text-xs font-medium text-slate-400">Provider</label>
        <div className="grid grid-cols-2 gap-1 rounded-lg border border-slate-700 bg-slate-800/60 p-1">
          <button
            onClick={() => p.setProvider("github")}
            className={`rounded-md px-3 py-1.5 text-xs ${
              isGithub ? "bg-indigo-600 text-white" : "text-slate-400 hover:bg-slate-800"
            }`}
          >
            GitHub Models (free)
          </button>
          <button
            onClick={() => p.setProvider("ollama")}
            className={`rounded-md px-3 py-1.5 text-xs ${
              !isGithub ? "bg-indigo-600 text-white" : "text-slate-400 hover:bg-slate-800"
            }`}
          >
            Ollama (local)
          </button>
        </div>
      </div>

      {isGithub ? (
        <>
          {/* Token */}
          <div>
            <label className="mb-1.5 block text-xs font-medium text-slate-400">
              GitHub token{" "}
              <span className={p.hasToken ? "text-emerald-400" : "text-amber-400"}>
                {p.hasToken ? "· saved" : "· not set"}
              </span>
            </label>
            <button
              onClick={p.onUseCliLogin}
              disabled={p.cliBusy}
              className="w-full rounded-lg bg-indigo-600 px-3 py-2 text-xs font-medium text-white hover:bg-indigo-500 disabled:opacity-40"
            >
              {p.cliBusy ? "Signing in…" : "Use my GitHub CLI login (no token needed)"}
            </button>
            <p className="mt-1 text-[11px] text-slate-500">
              Reuses your <span className="font-mono text-slate-400">gh auth login</span> session —
              the easiest option if you have the GitHub CLI installed. Nothing to paste.
            </p>

            <div className="my-3 flex items-center gap-2 text-[10px] uppercase tracking-wide text-slate-600">
              <span className="h-px flex-1 bg-slate-700" />
              or paste a token
              <span className="h-px flex-1 bg-slate-700" />
            </div>

            <div className="flex gap-2">
              <input
                type="password"
                value={p.tokenInput}
                onChange={(e) => p.setTokenInput(e.target.value)}
                placeholder={p.hasToken ? "Replace token…" : "ghp_… or github_pat_…"}
                className={inputClass}
              />
              <button
                onClick={p.onSaveToken}
                disabled={p.savingToken || !p.tokenInput.trim()}
                className="shrink-0 rounded-lg bg-indigo-600 px-3 py-2 text-xs font-medium text-white hover:bg-indigo-500 disabled:opacity-40"
              >
                {p.savingToken ? "Saving…" : "Save"}
              </button>
            </div>
            <p className="mt-1 text-[11px] text-slate-500">
              Needs a token with GitHub Models access — for a fine-grained token, add the{" "}
              <span className="font-mono text-slate-400">Models</span> permission (read-only) at{" "}
              <span className="font-mono text-slate-400">{GITHUB_TOKEN_SETTINGS}</span>.
            </p>
          </div>
        </>
      ) : (
        <div>
          <label className="mb-1.5 block text-xs font-medium text-slate-400">Ollama URL</label>
          <input
            value={p.ollamaUrl}
            onChange={(e) => p.setOllamaUrl(e.target.value)}
            placeholder="http://localhost:11434/v1"
            className={inputClass}
          />
          <p className="mt-1 text-[11px] text-slate-500">
            Runs models locally. Install from ollama.com, then{" "}
            <span className="font-mono text-slate-400">ollama pull llama3.1</span>.
          </p>
        </div>
      )}

      {/* Model */}
      <div>
        <div className="mb-1.5 flex items-center justify-between">
          <label className="text-xs font-medium text-slate-400">Model</label>
          <div className="flex items-center gap-3">
            {isGithub && p.githubModel !== RECOMMENDED_GITHUB_MODEL && (
              <button
                onClick={() => p.setGithubModel(RECOMMENDED_GITHUB_MODEL)}
                className="text-[11px] text-indigo-400 hover:text-indigo-300"
              >
                Use recommended
              </button>
            )}
            <button
              onClick={p.onLoadModels}
              disabled={p.loadingModels}
              className="text-[11px] text-indigo-400 hover:text-indigo-300 disabled:opacity-50"
            >
              {p.loadingModels ? "Loading…" : "Load available models"}
            </button>
          </div>
        </div>
        <input
          value={isGithub ? p.githubModel : p.ollamaModel}
          onChange={(e) => (isGithub ? p.setGithubModel(e.target.value) : p.setOllamaModel(e.target.value))}
          placeholder={isGithub ? "openai/gpt-4o-mini" : "llama3.1"}
          className={inputClass}
        />
        {p.models.length > 0 && (
          <select
            onChange={(e) => {
              if (!e.target.value) return;
              isGithub ? p.setGithubModel(e.target.value) : p.setOllamaModel(e.target.value);
            }}
            value=""
            className={`${inputClass} mt-2`}
          >
            <option value="">Pick from {p.models.length} available…</option>
            {p.models.map((m) => (
              <option key={m.id} value={m.id}>
                {m.name} ({m.id})
              </option>
            ))}
          </select>
        )}
      </div>

      {/* Privacy */}
      <div>
        <label className="mb-1.5 block text-xs font-medium text-slate-400">Data sharing</label>
        <label className="flex cursor-pointer items-start gap-2 rounded-lg border border-slate-700 bg-slate-800/40 px-3 py-2.5">
          <input
            type="checkbox"
            checked={p.includeRealData}
            onChange={(e) => p.setIncludeRealData(e.target.checked)}
            className="mt-0.5 h-4 w-4 accent-indigo-500"
          />
          <span className="text-xs text-slate-300">
            Send my real balances & transactions for the best answers.
            <span className="mt-0.5 block text-[11px] text-slate-500">
              Off = privacy mode: only rounded aggregates (net worth to the nearest $1,000, savings
              rate, goal progress) are shared. With Ollama everything stays on your device either
              way.
            </span>
          </span>
        </label>
      </div>

      <div className="flex justify-end gap-2 pt-1">
        <button
          onClick={p.onSave}
          disabled={p.saving}
          className="rounded-lg bg-indigo-600 px-4 py-2 text-sm font-medium text-white hover:bg-indigo-500 disabled:opacity-50"
        >
          {p.saving ? "Saving…" : "Save settings"}
        </button>
      </div>
    </div>
  );
}
