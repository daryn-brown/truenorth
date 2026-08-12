import { useCallback, useEffect, useRef, useState } from "react";
import type {
  AiProvider,
  AiSettings,
  ChatMessage,
  ChatThread,
  CopilotStatus,
  ModelInfo,
} from "../types/ai";
import {
  aiAppendMessage,
  aiChat,
  aiCopilotStatus,
  aiCreateThread,
  aiDeleteThread,
  aiGetSettings,
  aiGetThreadMessages,
  aiListModels,
  aiListThreads,
  aiSaveSettings,
} from "../hooks/useFinanceApi";
import MarkdownMessage from "./MarkdownMessage";
import ToolTrace from "./ToolTrace";

interface Props {
  /** Whether the panel is expanded (vs. collapsed to a slim rail). */
  open: boolean;
  onOpen: () => void;
  onClose: () => void;
}

const inputClass =
  "w-full rounded-lg border border-slate-600 bg-slate-800 px-3 py-2 text-sm text-slate-200 placeholder-slate-600 focus:outline-none focus:ring-2 focus:ring-indigo-500";

const SUGGESTIONS = [
  "Find my recurring subscriptions and tell me which ones to review.",
  "Analyze my spending habits and show the biggest opportunities to improve.",
  "How am I tracking toward my $100k goal?",
  "What tax-planning questions should I ask about my US and Canadian finances?",
];

const isCopilotAccessError = (msg: string) =>
  /copilot|authenticat|entitlement|gh auth|github cli|github account/i.test(msg);

export default function AdvisorPanel({ open, onOpen, onClose }: Props) {
  const [settings, setSettings] = useState<AiSettings | null>(null);
  const [showSettings, setShowSettings] = useState(false);
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [input, setInput] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Thread persistence: saved conversations the user can switch between or delete.
  const [threads, setThreads] = useState<ChatThread[]>([]);
  const [activeThreadId, setActiveThreadId] = useState<number | null>(null);
  const [threadsOpen, setThreadsOpen] = useState(false);

  // Editable settings draft (mirrors `settings` until saved).
  const [provider, setProvider] = useState<AiProvider>("copilot");
  const [copilotModel, setCopilotModel] = useState("");
  const [ollamaModel, setOllamaModel] = useState("");
  const [ollamaUrl, setOllamaUrl] = useState("");
  const [includeRealData, setIncludeRealData] = useState(true);
  const [savingSettings, setSavingSettings] = useState(false);
  const [copilotStatus, setCopilotStatus] = useState<CopilotStatus | null>(null);
  const [checkingCopilot, setCheckingCopilot] = useState(false);
  const [models, setModels] = useState<ModelInfo[]>([]);
  const [loadingModels, setLoadingModels] = useState(false);

  const scrollRef = useRef<HTMLDivElement>(null);
  const initedRef = useRef(false);

  const applySettings = useCallback((s: AiSettings) => {
    setSettings(s);
    setProvider(s.provider);
    setCopilotModel(s.copilot_model);
    setOllamaModel(s.ollama_model);
    setOllamaUrl(s.ollama_url);
    setIncludeRealData(s.include_real_data);
  }, []);

  const refreshThreads = useCallback(async () => {
    try {
      setThreads(await aiListThreads());
    } catch {
      // Non-fatal: the chat still works without an up-to-date list.
    }
  }, []);

  // Load a saved thread's messages into the view.
  const openThread = useCallback(async (id: number) => {
    setError(null);
    setShowSettings(false);
    setThreadsOpen(false);
    setActiveThreadId(id);
    try {
      const msgs = await aiGetThreadMessages(id);
      setMessages(
        msgs.map((m) => ({ role: m.role, content: m.content, steps: m.steps })),
      );
    } catch (e) {
      setError(String(e));
    }
  }, []);

  // Start a fresh conversation. The thread row is created lazily on the first send.
  const newChat = useCallback(() => {
    setActiveThreadId(null);
    setMessages([]);
    setError(null);
    setShowSettings(false);
    setThreadsOpen(false);
  }, []);

  // Load settings each time the panel opens; load threads once and resume the most recent.
  useEffect(() => {
    if (!open) return;
    setError(null);
    aiGetSettings()
      .then((s) => {
        applySettings(s);
      })
      .catch((e) => setError(String(e)));

    if (!initedRef.current) {
      initedRef.current = true;
      aiListThreads()
        .then((ts) => {
          setThreads(ts);
          if (ts.length > 0) void openThread(ts[0].id);
        })
        .catch((e) => setError(String(e)));
    }
  }, [open, applySettings, openThread]);

  // Keep the latest message in view.
  useEffect(() => {
    scrollRef.current?.scrollTo({ top: scrollRef.current.scrollHeight, behavior: "smooth" });
  }, [messages, busy]);

  useEffect(() => {
    setModels([]);
  }, [provider]);

  // Ensure a thread exists to attach messages to, creating one on demand.
  const ensureThread = useCallback(async (): Promise<number> => {
    if (activeThreadId != null) return activeThreadId;
    const t = await aiCreateThread();
    setActiveThreadId(t.id);
    setThreads((prev) => [t, ...prev]);
    return t.id;
  }, [activeThreadId]);

  // Call the model with the conversation, then append + persist the assistant reply.
  const generate = useCallback(
    async (convo: ChatMessage[], threadId: number) => {
      const reply = await aiChat(convo);
      setMessages((m) => [...m, reply]);
      await aiAppendMessage(threadId, "assistant", reply.content, reply.steps);
      await refreshThreads();
    },
    [refreshThreads],
  );

  const send = useCallback(
    async (text: string) => {
      const content = text.trim();
      if (!content || busy) return;
      const next: ChatMessage[] = [...messages, { role: "user", content }];
      setMessages(next);
      setInput("");
      setBusy(true);
      setError(null);
      try {
        const threadId = await ensureThread();
        await aiAppendMessage(threadId, "user", content);
        await generate(next, threadId);
      } catch (e) {
        setError(String(e));
      } finally {
        setBusy(false);
      }
    },
    [messages, busy, ensureThread, generate],
  );

  // After a failed send the conversation already ends with the user's turn (the assistant reply was
  // never appended) and that user turn is already persisted, so retry only re-generates the answer.
  const retry = useCallback(async () => {
    if (busy || messages.length === 0 || activeThreadId == null) return;
    setBusy(true);
    setError(null);
    try {
      await generate(messages, activeThreadId);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }, [busy, messages, activeThreadId, generate]);

  const handleDeleteThread = useCallback(
    async (id: number) => {
      try {
        await aiDeleteThread(id);
        setThreads((prev) => prev.filter((t) => t.id !== id));
        if (activeThreadId === id) newChat();
      } catch (e) {
        setError(String(e));
      }
    },
    [activeThreadId, newChat],
  );

  const checkCopilotAccess = useCallback(async () => {
    if (checkingCopilot) return false;
    setCheckingCopilot(true);
    setError(null);
    try {
      setCopilotStatus(await aiCopilotStatus());
      return true;
    } catch (e) {
      setCopilotStatus(null);
      setError(String(e));
      return false;
    } finally {
      setCheckingCopilot(false);
    }
  }, [checkingCopilot]);

  const checkCopilotAndRetry = useCallback(async () => {
    if (await checkCopilotAccess()) await retry();
  }, [checkCopilotAccess, retry]);

  const handleSaveSettings = async () => {
    setSavingSettings(true);
    setError(null);
    try {
      const saved = await aiSaveSettings({
        provider,
        copilot_model: copilotModel,
        ollama_model: ollamaModel,
        ollama_url: ollamaUrl,
        include_real_data: includeRealData,
      });
      applySettings(saved);
      setShowSettings(false);
    } catch (e) {
      setError(String(e));
    } finally {
      setSavingSettings(false);
    }
  };

  const handleLoadModels = async () => {
    setLoadingModels(true);
    setError(null);
    try {
      setModels(await aiListModels(provider, ollamaUrl));
    } catch (e) {
      setError(String(e));
    } finally {
      setLoadingModels(false);
    }
  };

  // Collapsed: a slim, always-present rail so the chat is one click away.
  if (!open) {
    return (
      <div className="flex h-full w-12 shrink-0 flex-col items-center border-l border-slate-800 bg-slate-900 py-3">
        <button
          onClick={onOpen}
          title="Open finance brain"
          className="flex flex-col items-center gap-2 rounded-lg px-2 py-2 text-slate-400 hover:bg-slate-800 hover:text-white"
        >
          <span className="text-lg">🧠</span>
          <span className="text-[10px] font-semibold uppercase tracking-wider [writing-mode:vertical-rl]">
            Ask AI
          </span>
        </button>
      </div>
    );
  }

  const canChat = settings != null;
  const activeProvider = settings?.provider ?? provider;
  const activeIncludeRealData = settings?.include_real_data ?? includeRealData;
  const activeThread = threads.find((t) => t.id === activeThreadId);
  const headerTitle = activeThread?.title ?? "Finance brain";

  return (
    <div className="relative flex h-full w-[380px] shrink-0 flex-col border-l border-slate-800 bg-slate-900 lg:w-[440px]">
      {/* Header */}
      <div className="flex items-center justify-between gap-2 border-b border-slate-800 px-3 py-2.5">
        <div className="flex min-w-0 items-center gap-1.5">
          <button
            onClick={() => setThreadsOpen((v) => !v)}
            title="Chat history"
            className="rounded-lg border border-slate-700 px-2 py-1 text-xs text-slate-400 hover:bg-slate-800"
          >
            ☰
          </button>
          <span className="truncate text-sm font-semibold text-white" title={headerTitle}>
            {headerTitle}
          </span>
        </div>
        <div className="flex shrink-0 items-center gap-1">
          <button
            onClick={newChat}
            title="New chat"
            className="rounded-lg border border-slate-700 px-2 py-1 text-xs text-slate-400 hover:bg-slate-800"
          >
            ＋
          </button>
          <button
            onClick={() => setShowSettings((v) => !v)}
            title="AI provider settings"
            className="rounded-lg border border-slate-700 px-2 py-1 text-xs text-slate-400 hover:bg-slate-800"
          >
            ⚙️
          </button>
          <button
            onClick={onClose}
            title="Collapse panel"
            className="rounded-lg border border-slate-700 px-2 py-1 text-xs text-slate-400 hover:bg-slate-800"
          >
            »
          </button>
        </div>
      </div>

      <div className="border-b border-slate-800 px-3 py-1.5">
        <span className="rounded-full border border-slate-700 bg-slate-800 px-2 py-0.5 text-[10px] uppercase tracking-wider text-slate-400">
          {activeProvider === "copilot" ? "GitHub Copilot" : "Ollama · local"}
        </span>
      </div>

      {error && (
        <div className="mx-3 mt-3 rounded-lg border border-red-700/50 bg-red-900/20 px-3 py-2 text-xs text-red-300">
          <p>{error}</p>
          {activeProvider === "copilot" && isCopilotAccessError(error) && messages.length > 0 && (
            <button
              onClick={() => void checkCopilotAndRetry()}
              disabled={busy || checkingCopilot}
              className="mt-2 rounded-md border border-red-600/60 bg-red-800/40 px-2.5 py-1 font-medium text-red-100 hover:bg-red-800/70 disabled:opacity-50"
            >
              {checkingCopilot ? "Checking…" : "Check Copilot access and retry"}
            </button>
          )}
        </div>
      )}

      {showSettings ? (
        <SettingsPanel
          provider={provider}
          setProvider={setProvider}
          copilotModel={copilotModel}
          setCopilotModel={setCopilotModel}
          ollamaModel={ollamaModel}
          setOllamaModel={setOllamaModel}
          ollamaUrl={ollamaUrl}
          setOllamaUrl={setOllamaUrl}
          includeRealData={includeRealData}
          setIncludeRealData={setIncludeRealData}
          copilotStatus={copilotStatus}
          onCheckCopilot={checkCopilotAccess}
          checkingCopilot={checkingCopilot}
          models={models}
          loadingModels={loadingModels}
          onLoadModels={handleLoadModels}
          onSave={handleSaveSettings}
          saving={savingSettings}
        />
      ) : (
        <>
          {/* Messages */}
          <div ref={scrollRef} className="flex-1 space-y-3 overflow-y-auto px-4 py-4">
            {messages.length === 0 ? (
              <div className="flex h-full flex-col items-center justify-center gap-4 text-center">
                <p className="max-w-sm text-sm text-slate-400">
                  Ask anything about your finances. I call tools on your accounts, net worth,
                  cashflow, goal, holdings, and transactions to answer.
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
                    Loading AI settings…
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
          <div className="border-t border-slate-800 px-4 py-3">
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
              {activeProvider === "copilot" ? (
                <>
                  {activeIncludeRealData
                    ? "Your real balances & transactions are sent through GitHub Copilot to answer."
                    : "Privacy mode: only this question and rounded aggregates are sent through GitHub Copilot."}
                </>
              ) : (
                "Running locally via Ollama — nothing leaves your device."
              )}{" "}
              Educational planning only; verify tax guidance before filing.
            </p>
          </div>
        </>
      )}

      {/* Thread history drawer */}
      {threadsOpen && (
        <div className="absolute inset-0 z-10 flex">
          <div className="flex w-64 flex-col border-r border-slate-800 bg-slate-900">
            <div className="flex items-center justify-between border-b border-slate-800 px-3 py-2.5">
              <span className="text-xs font-semibold text-slate-300">Chats</span>
              <button
                onClick={newChat}
                className="rounded-md border border-slate-700 px-2 py-1 text-[11px] text-slate-300 hover:bg-slate-800"
              >
                ＋ New
              </button>
            </div>
            <div className="flex-1 space-y-1 overflow-y-auto p-2">
              {threads.length === 0 ? (
                <p className="px-2 py-4 text-center text-xs text-slate-600">No saved chats yet.</p>
              ) : (
                threads.map((t) => (
                  <div
                    key={t.id}
                    className={`group flex items-center gap-1 rounded-lg px-2 py-1.5 ${
                      t.id === activeThreadId ? "bg-slate-800" : "hover:bg-slate-800/60"
                    }`}
                  >
                    <button
                      onClick={() => void openThread(t.id)}
                      className="flex-1 truncate text-left text-xs text-slate-300"
                      title={t.title}
                    >
                      {t.title}
                    </button>
                    <button
                      onClick={() => void handleDeleteThread(t.id)}
                      title="Delete chat"
                      className="text-xs text-slate-600 opacity-0 transition-opacity hover:text-red-400 group-hover:opacity-100"
                    >
                      🗑
                    </button>
                  </div>
                ))
              )}
            </div>
          </div>
          {/* Click-away closes the drawer. */}
          <div className="flex-1 bg-black/40" onClick={() => setThreadsOpen(false)} />
        </div>
      )}
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
      <div className="max-w-[90%] rounded-2xl border border-slate-700 bg-slate-800 px-3.5 py-2 text-slate-200">
        <MarkdownMessage content={message.content} />
        {message.steps && message.steps.length > 0 && <ToolTrace steps={message.steps} />}
      </div>
    </div>
  );
}

interface SettingsProps {
  provider: AiProvider;
  setProvider: (p: AiProvider) => void;
  copilotModel: string;
  setCopilotModel: (v: string) => void;
  ollamaModel: string;
  setOllamaModel: (v: string) => void;
  ollamaUrl: string;
  setOllamaUrl: (v: string) => void;
  includeRealData: boolean;
  setIncludeRealData: (v: boolean) => void;
  copilotStatus: CopilotStatus | null;
  onCheckCopilot: () => void;
  checkingCopilot: boolean;
  models: ModelInfo[];
  loadingModels: boolean;
  onLoadModels: () => void;
  onSave: () => void;
  saving: boolean;
}

function SettingsPanel(p: SettingsProps) {
  const isCopilot = p.provider === "copilot";
  return (
    <div className="flex-1 space-y-5 overflow-y-auto px-4 py-4">
      {/* Provider */}
      <div>
        <label className="mb-1.5 block text-xs font-medium text-slate-400">Provider</label>
        <div className="grid grid-cols-2 gap-1 rounded-lg border border-slate-700 bg-slate-800/60 p-1">
          <button
            onClick={() => p.setProvider("copilot")}
            className={`rounded-md px-3 py-1.5 text-xs ${
              isCopilot ? "bg-indigo-600 text-white" : "text-slate-400 hover:bg-slate-800"
            }`}
          >
            GitHub Copilot
          </button>
          <button
            onClick={() => p.setProvider("ollama")}
            className={`rounded-md px-3 py-1.5 text-xs ${
              !isCopilot ? "bg-indigo-600 text-white" : "text-slate-400 hover:bg-slate-800"
            }`}
          >
            Ollama (local, free)
          </button>
        </div>
      </div>

      {isCopilot ? (
        <div className="space-y-2">
          <div className="rounded-lg border border-indigo-500/40 bg-indigo-500/10 px-3 py-2 text-[11px] leading-relaxed text-indigo-100">
            Uses the official GitHub Copilot SDK and your existing Copilot subscription. The bundled
            runtime runs in isolated mode with only TrueNorth&apos;s read-only finance tools — no
            shell, files, coding tools, or cloud session history.
          </div>
          <button
            onClick={p.onCheckCopilot}
            disabled={p.checkingCopilot}
            className="w-full rounded-lg bg-indigo-600 px-3 py-2 text-xs font-medium text-white hover:bg-indigo-500 disabled:opacity-40"
          >
            {p.checkingCopilot ? "Checking…" : "Check my Copilot access"}
          </button>
          <p className="text-[11px] text-slate-500">
            Uses your local GitHub CLI session. If needed, run{" "}
            <span className="font-mono text-slate-400">gh auth login</span> with the GitHub account
            that has Copilot.
          </p>
          {p.copilotStatus && (
            <p className="rounded-lg border border-emerald-700/50 bg-emerald-900/20 px-3 py-2 text-[11px] text-emerald-300">
              {p.copilotStatus.message}
            </p>
          )}
        </div>
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
            <span className="font-mono text-slate-400">ollama pull llama3.1</span>. TrueNorth
            automatically uses a model you've already pulled and only accepts loopback URLs.
          </p>
        </div>
      )}

      {/* Model */}
      <div>
        <div className="mb-1.5 flex items-center justify-between">
          <label className="text-xs font-medium text-slate-400">Model</label>
          <div className="flex items-center gap-3">
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
          value={isCopilot ? p.copilotModel : p.ollamaModel}
          onChange={(e) =>
            isCopilot ? p.setCopilotModel(e.target.value) : p.setOllamaModel(e.target.value)
          }
          placeholder={isCopilot ? "auto" : "llama3.1"}
          className={inputClass}
        />
        {isCopilot && (
          <p className="mt-1 text-[11px] text-slate-500">
            Leave as <span className="font-mono text-slate-400">auto</span> to use Copilot&apos;s
            current default, or load the models available to your plan.
          </p>
        )}
        {p.models.length > 0 && (
          <select
            onChange={(e) => {
              if (!e.target.value) return;
              isCopilot ? p.setCopilotModel(e.target.value) : p.setOllamaModel(e.target.value);
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
              With GitHub Copilot, off means only the current question and rounded aggregates (net
              worth to the nearest $1,000, savings rate, goal progress) are shared; prior chat turns
              are not. Exact tool results leave your device only when this is on. With Ollama
              everything stays local either way.
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
