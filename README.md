# Finance Second Brain

A **local-first, privacy-first** desktop app for managing **cross-border (US + Canada)**
personal finances: connect bank + brokerage accounts, review transactions, track
**multi-currency net worth** over time, set goals, and ask a **model-agnostic AI advisor**
questions about your own data.

> Replaces the "paste screenshots into a chatbot" workflow with a real, queryable system.

## Status
🌱 **Bootstrapped — not yet scaffolded.** This repo currently contains the blueprint only.
The next step is to open it as a Copilot project and run the kickoff prompt (Phase 0).

## Scope (now)
**Financial transparency + easy decision-making.** Everything is **read-only**.
- Aggregation: brokerages via **SnapTrade** (free single-user), banks via **SimpleFIN Bridge**
  (one ~$15/yr connector covers Chase + Bask + Scotiabank), plus **manual/CSV** fallback.
- **Multi-currency net worth** (USD + CAD) with history chart + dashboard.
- Transaction review (search/filter/categorize) + goals.
- **Model-agnostic AI** (GitHub Models / Ollama / Azure) with a local-only privacy mode.

**Deferred (separate, guarded module later):** automated trading / order execution.

## Planned stack
Tauri v2 (Rust core) · React + TypeScript + Tailwind · SQLite (rusqlite, SQLCipher) ·
secrets in the OS keychain (`keyring`). Mirrors the TrendWave stack.

## How to start building
1. Open this folder as a **project** in Copilot.
2. Create a **new session**.
3. Paste the prompt in [`docs/kickoff-prompt.md`](docs/kickoff-prompt.md) to drive **Phase 0 → Phase 1**.

## Docs
- [`docs/blueprint.md`](docs/blueprint.md) — full research report (connectors, architecture, cross-border notes, citations).
- [`docs/plan.md`](docs/plan.md) — phased build plan.
- [`docs/kickoff-prompt.md`](docs/kickoff-prompt.md) — ready-to-paste prompt for the first build session.

## Phased roadmap
0. Scaffold (Tauri/React/SQLite shell, encryption, keychain)
1. Manual multi-currency net-worth MVP (+ Gemini/JSON import)
2. SnapTrade brokerage sync
3. SimpleFIN bank sync
4. Transactions & goals
5. Model-agnostic AI "second brain"
6. Hardening & polish

## Privacy
Financial data stays **local and encrypted**. Secrets live in the OS keychain, never in
the repo (`.env`, `*.db`, `*.sqlite` are gitignored). For AI, prefer local Ollama or Azure
for real balances; redact/aggregate before using the free GitHub Models tier.
