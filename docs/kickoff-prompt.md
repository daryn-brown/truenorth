# Kickoff prompt — first build session

Open this folder as a Copilot **project**, start a **new session**, and paste the prompt
below to begin **Phase 0 → Phase 1**. (Source: `docs/blueprint.md` §11.)

```
I'm building a LOCAL-FIRST, privacy-first desktop app called "Finance Second Brain"
— a personal-finance "second brain" for my cross-border (US + Canada) finances.
I currently track net worth by pasting screenshots into a chatbot; I want to
replace that with a real app that connects my accounts, reviews transactions,
tracks multi-currency net worth over time, and supports goals.

Stack (match my other app, TrendWave): Tauri v2 (Rust core + React/TypeScript +
Tailwind), local SQLite encrypted with SQLCipher, secrets in the OS keychain via
the `keyring` crate. Reuse a Yahoo Finance price lookup. For the "ask my finances"
feature, build a MODEL-AGNOSTIC LLM layer (one OpenAI-compatible client + a provider
registry) so I can switch models per question: GitHub Models (free frontier via my
GitHub PAT with models:read scope, base url https://models.github.ai/inference),
local Ollama (http://localhost:11434/v1) for a private mode, and optionally Azure/OpenAI.

Accounts I need to connect (design a pluggable connector abstraction; not all use
the same provider):
- Robinhood (US brokerage)  -> SnapTrade (read-only; free single-user tier)
- Questrade (CA brokerage)  -> SnapTrade OR official Questrade REST API
- Chase (US bank)           -> SimpleFIN Bridge ($15/yr) or Plaid
- Bask Bank (US savings)    -> SimpleFIN/Plaid, manual fallback
- Scotiabank (CA bank)      -> SimpleFIN Bridge (covers it via MX) or Plaid Canada
- Everything else           -> manual entry + CSV import (build this FIRST)

Hard requirements:
- Multi-currency: every value carries a currency; store fx_rates (USD<->CAD);
  show net worth in BOTH USD and CAD with a home-currency toggle.
- Net-worth history via per-account balance_snapshots time series + a chart.
- Tag accounts by type (chequing/savings/brokerage/TFSA/RRSP/FHSA/401k/IRA/Roth/
  credit/crypto) and jurisdiction (US/CA).
- Aggregation is READ-ONLY (no money movement). Any trade execution lives in a
  SEPARATE, opt-in, guarded module (off by default) — never mixed into the read-only
  dashboard. Encrypt the DB; never commit secrets.
- An importer to seed data from a JSON export (schema I'll provide).

Plan in phases:
1) Encrypted SQLite schema + manual/CSV accounts + multi-currency net worth + dashboard.
2) SnapTrade connector (Robinhood, Questrade, Wealthsimple holdings).
3) Bank sync (SimpleFIN for US; Plaid/Flinks for CA) + transaction review/categorization.
4) Goals, budgets, recurring detection, net-worth-over-time insights.
5) Model-agnostic "second brain": RAG over my SQLite DB with a model picker (GitHub
   Models / Ollama / Azure) + a local-only privacy mode + optional screenshot/OCR ingestion.
6) (Optional, SEPARATE guarded module) automated trading via Alpaca PAPER first —
   human-approve orders, kill switch, hard caps, idempotency, PDT/wash-sale guards,
   audit log. NOT Robinhood (read-only/ToS); Questrade live-trade needs partner approval.

Start with Phase 1: propose the SQLite schema, the Rust AccountConnector trait, and a
minimal Tauri+React dashboard showing total net worth in USD and CAD from manually
entered accounts. Then wait for my review before Phase 2.
```

## Before Phase 2 — extract your Gemini context
Use the export prompt in `docs/blueprint.md` §10 to pull your existing balances/history
out of your Gemini chat as JSON, then import it to seed the app in Phase 1.
