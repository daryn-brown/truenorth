# Plan — Cross-border finance "second brain": Transparency & Decision-Making MVP

> Companion to the research report:
> `files/research/i-am-in-a-unique-situation-where-i-inter.md` (deep detail, citations, connector matrix).
> This plan is the **actionable build plan** for a **new app, separate from TrendWave**.

## Problem & goal
A cross-border life (US accounts from a 2024 internship; now in Canada on a temp visa with
CA accounts) makes net worth and day-to-day money decisions hard. Today that lives in a
Gemini chat fed by screenshots. Goal: a **local-first desktop app** that gives **one
trustworthy, multi-currency view** of everything (banks + brokerages), lets you **review
transactions**, and offers a **model-agnostic AI advisor** for "ask my finances" decisions.

**Focus right now: financial transparency + easy decision-making.**
**Explicitly deferred: automated trading** (future, separate, guarded module — see report §6).

## Scope
**In**
- Read-only account aggregation: brokerages via **SnapTrade** (free single-user), banks via
  **SimpleFIN Bridge** (one $15/yr connector covers Chase + Bask + Scotiabank), **manual/CSV** fallback.
- Normalized **SQLite** model: accounts, balances, holdings, transactions, prices, fx_rates, goals.
- **Multi-currency net worth** (USD + CAD), history chart, dashboard.
- **Transaction review**: search, filter, categorize, subscriptions/spending summaries.
- **Goals** tracking vs net worth.
- **Model-agnostic AI** advisor (GitHub Models / Ollama / Azure) + **local-only privacy mode**, RAG over the DB.
- **Security**: encrypted DB, secrets in OS keychain, no money-moving scopes anywhere.

**Out (deferred / non-goals)**
- Automated trading / order execution (separate module later).
- Tax filing, multi-user, real-time bank push, mobile.

## Approach
Mirror the **TrendWave stack** (Tauri v2 + Rust + React/TS + Tailwind + SQLite via rusqlite) —
the user already knows it and it's local-first/privacy-friendly. Ship value **incrementally**:
a **manual MVP** that already beats the Gemini ritual, then wire connectors, then the AI layer.
Everything stays **read-only**.

## Tech stack
- **Shell:** Tauri v2, Rust core, React + TypeScript + Tailwind.
- **Storage:** SQLite (rusqlite); encryption at rest (SQLCipher or app-level); `keyring` crate for tokens.
- **Connectors:** SnapTrade (HTTP), SimpleFIN Bridge (HTTP basic), manual/CSV importer.
- **Prices/FX:** Yahoo Finance lookup (reuse TrendWave pattern) + an FX-rate source.
- **AI:** one OpenAI-compatible client + provider registry (GitHub Models `https://models.github.ai/inference`,
  Ollama `http://localhost:11434/v1`, optional Azure/OpenAI).

## Key decisions & defaults (assumed; easy to change)
- **Base/home currency:** default **CAD** (current residence), with a **USD toggle**; always show **both**. *(Assumption — flag in open questions.)*
- **Connector order:** manual first → SnapTrade → SimpleFIN. Get transparency before perfect automation.
- **AI providers first:** **GitHub Models** (free frontier) + **Ollama** (private); Azure optional. Prefer local/Azure for real balances; redact/aggregate before the free GitHub tier.
- **Privacy:** read-only scopes only; encrypted DB; secrets never committed.

## Phased roadmap (todos tracked in SQL)
- **Phase 0 — Scaffold & foundations:** init Tauri+React/TS/Tailwind; SQLite + migrations + encryption + keyring; app shell (nav, dashboard skeleton, settings).
- **Phase 1 — Manual MVP (transparency core):** accounts + manual balances/holdings; FX + multi-currency net worth (USD+CAD); dashboard cards; net-worth history chart; **Gemini import** to seed accounts/history. *Retires the screenshot ritual.*
- **Phase 2 — Brokerage sync (SnapTrade):** single-user registration + connection portal; pull accounts/holdings/balances; price refresh + reconcile (Robinhood/Questrade/Wealthsimple).
- **Phase 3 — Bank sync (SimpleFIN):** setup-token → access; one connector for Chase + Bask + Scotiabank; pull balances + transactions; daily refresh.
- **Phase 4 — Transactions & decision aids:** transaction list (search/filter/categorize, rules + manual); spending/income summaries; subscription detection; goals vs net worth.
- **Phase 5 — Model-agnostic AI "second brain":** OpenAI-compatible client + provider registry; per-question model picker + local-only privacy mode; RAG/tools over SQLite (net worth, txns, holdings) with answers grounded + cited to your own data; redaction before free-tier.
- **Phase 6 — Hardening & polish:** encryption review, backup/export, onboarding/empty states, error handling.

## Open questions (non-blocking — sensible defaults assumed)
1. **Base currency** default — CAD assumed (you live in Canada); confirm vs USD.
2. **Accounts to include** beyond Robinhood/Questrade/Chase/Bask/Scotiabank — any **crypto / Wealthsimple**?
3. **AI provider ordering** — GitHub Models + Ollama first (Azure later)?
4. **SimpleFIN vs Plaid** for banks — cheap single connector (assumed) vs nicer DX/real-time.

## Risks
- Connector **coverage/pricing drift** — re-verify each provider before committing (esp. Canadian banks via MX).
- **Cross-border FX & data privacy** — keep real balances on local/Azure models; aggregate/redact for free GitHub tier.
- **Scope creep toward trading** — keep it out; the read-only dashboard must stay simple and trustworthy.

## Getting started — create the new repo, then a new project (do this first)
You're currently inside the **TrendWave** worktree, so you can't build here. Create a
**separate repo** in your normal Projects folder (a sibling of TrendWave), then add it to
Copilot as its **own project**.

**1) Create the folder + git repo** (run in your own terminal, outside this app's worktree):
```bash
cd ~/Projects                      # sibling of TrendWave, NOT the worktrees folder
mkdir finance-second-brain         # pick your name (e.g. networth-os, ledgerlens)
cd finance-second-brain
git init -b main
printf "# Finance Second Brain\n\nCross-border personal finance: multi-currency net worth, transactions, AI advisor.\n" > README.md
printf "/target\nsrc-tauri/target\nnode_modules\ndist\n.env\n*.db\n.DS_Store\n" > .gitignore
git add -A
git commit -m "chore: initialize repo"
```

**2) (Optional but recommended) create the GitHub repo + push:**
```bash
gh repo create finance-second-brain --private --source=. --remote=origin --push
```

**3) Add it to Copilot as a new project, then start a session:**
- In the Copilot app's project picker, **Add project** → point at `~/Projects/finance-second-brain`
  (or clone the new GitHub repo).
- **Create a new session** in that project (it'll get its own worktree/branch).

**4) Kick off the build:** in that new session, paste the **kickoff prompt** from the
research report (§11). That drives **Phase 0 → Phase 1** (Tauri scaffold, then the manual
multi-currency net-worth MVP). Reference this plan + the report for the rest.

> Note: this current session stays focused on TrendWave + the blueprint. All build work
> happens in the new project so the two codebases never mix.
