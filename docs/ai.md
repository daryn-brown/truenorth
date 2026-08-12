# AI advisor ("second brain")

TrueNorth has a built-in **AI advisor** that answers questions about **your own financial data** —
net worth, accounts, cashflow, your goal, holdings, and recent transactions. It's the "ask the LLM
about my finances" workflow, but instead of you pasting screenshots into a chatbot, the model
**calls read-only tools** that query your local database on demand and then writes the answer in
**rich markdown**.

It lives in a **collapsible side panel** docked to the right of the dashboard. Toggle it with the
**🧠 Ask AI** button in the header, or the slim **rail** on the right edge; the open/collapsed state
is remembered between launches. Conversations are **saved as threads** that retain their full
context and can be revisited, renamed, or deleted (see [Saved chats](#saved-chats-threads)).

Everything the advisor does is **read-only** and **opt-in per provider**. You choose where the model
runs:

| Provider | Cost | Where it runs | What's sent off-device |
| --- | --- | --- | --- |
| **GitHub Copilot** | Uses your Copilot plan | GitHub-hosted models through the official Copilot SDK | Conversation + minimal account context + requested tool results (or only the current question and rounded aggregates in privacy mode) |
| **Ollama** | Free | Fully local on your machine | Nothing — never leaves your device |

The old **GitHub Models** provider was retired. TrueNorth now uses the official **GitHub Copilot
SDK**, which can consume the Copilot access attached to your GitHub account instead of requiring a
separate model API key.

## Option A — GitHub Copilot (your existing subscription)

1. Install the [GitHub CLI](https://cli.github.com/) and sign in with the account that has Copilot:
   ```sh
   gh auth login
   ```
2. In **🧠 Ask AI → ⚙️ Settings**, select **GitHub Copilot** and click **Check my Copilot access**.
   TrueNorth reads the local `gh` login for each app session; it never displays or stores that token.
3. Leave the model set to `auto`, or click **Load available models** to choose one included in your
   plan.

TrueNorth bundles the version-matched Copilot runtime, so a separate `copilot` executable is not
required. The runtime is started in the SDK's isolated **empty mode**:

- only TrueNorth's eight read-only finance tools are available;
- shell, filesystem, coding, MCP, skills, and host-Git capabilities are disabled;
- each transient SDK session is disconnected and permanently deleted after the answer (and any
  leftover sessions from an interrupted prior run are deleted on startup);
- memory, telemetry, and remote export are disabled;
- durable chats remain only in TrueNorth's encrypted local database.

The model request and any finance-tool results still travel through GitHub. Individual Copilot
accounts should review GitHub's **model training and improvements** setting before sharing exact
financial records.

## Option B — Ollama (fully local, free)

[Ollama](https://ollama.com) runs open models entirely on your machine — nothing is ever sent off
your device, regardless of the privacy setting. Use it when keeping exact financial data local is
more important than using Copilot-hosted models. TrueNorth enforces this boundary by accepting only
loopback Ollama URLs (`localhost`, `127.0.0.1`, or `::1`) while disabling HTTP proxies and redirects,
not remote OpenAI-compatible endpoints.

1. Install Ollama and pull a model:
   ```sh
   ollama pull llama3.1
   ```
   (Ollama serves an OpenAI-compatible API on `http://localhost:11434` while running.)
2. In **🧠 Ask AI → ⚙️ Settings**, select **Ollama (local)**. The default URL
   (`http://localhost:11434/v1`) works out of the box. TrueNorth automatically uses a model you've
   actually pulled — if the stored default (`llama3.1`) isn't installed, it falls back to one that
   is — so you can also just click **Load available models** to pick a specific one.
3. Ask away. If Ollama isn't running you'll get a "couldn't reach the AI provider" hint — start it
   with `ollama serve` (or just launch the app).

## How answers are produced

When **Send my real balances & transactions** is on (the default), the advisor is **agentic**: rather
than reading one fixed snapshot, the model decides which data it needs and calls **read-only finance
tools** that run against your local database. It can chain several calls — e.g. pull cashflow, then
the recent transactions behind a category, then recurring charges — before writing its answer. Each
answer shows a collapsible **"Used N tools"** trace so you can see exactly what it looked at.

The tools available to the model:

| Tool | What it returns |
| --- | --- |
| `get_net_worth_summary` | Net worth in USD + CAD, FX date, account count, home currency |
| `list_accounts` | Each account's institution, type, jurisdiction, currency, and balance |
| `get_cashflow` | Income / fixed / variable / net, savings rate, and variable-by-category for a window |
| `list_transactions` | Recent transactions, filterable by search text and flow (income/fixed/variable/transfer) |
| `find_recurring_transactions` | Subscriptions / recurring charges, detected by grouping similar merchants |
| `get_liabilities` | Credit-card and loan accounts with balances owed |
| `get_holdings` | Investment holdings with estimated value |
| `get_goal` | Goal progress and projected completion date |

All tools are **read-only** — the advisor can never add, edit, or delete your data. The figures come
from the same calculations the dashboard shows.

## What data the model sees

The data-sharing toggle in **🧠 Ask AI → ⚙️ Settings → Data sharing** controls how much is sent:

- **Send my real balances & transactions (default).** Enables the agentic tools above, so the model
  can pull exact figures and line-item detail — the most accurate answers. Recommended for Ollama
  always, and available for GitHub Copilot only when you're comfortable sending the tool results
  through GitHub.
- **Privacy mode (toggle off).** Tools are disabled. Instead, only a **rounded-aggregate snapshot**
  is available: net worth to the nearest $1,000, account count, savings rate, and goal progress — no
  exact balances, holdings, or individual transactions. With Copilot, only that snapshot and the
  current question are sent; prior saved turns are omitted because they might contain exact figures
  from an earlier mode. Useful when you want Copilot's model quality without sharing line-item
  detail.

The separate **Refine categories with AI** action needs merchant-level transaction details. When
Copilot is selected, TrueNorth blocks that action in privacy mode; enable real-data sharing
explicitly or use local Ollama.

With **Ollama**, the data never leaves your machine either way, so privacy mode mainly just shortens
the prompt.

The advisor is grounded: it's instructed to answer **only** from your data and to say so when the
information it needs isn't there, rather than inventing numbers. For tax questions it must establish
the tax year, country, state/province, residency, and filing status; keep US and Canadian rules
separate; and avoid inventing rates, thresholds, deadlines, or treaty results. It is an
**educational tax-planning tool, not a licensed tax professional or a source of current tax law** —
verify material guidance against IRS/CRA sources or a qualified cross-border professional before
filing.

## Saved chats (threads)

Each conversation is a **thread** saved in the local **encrypted** database, so your chats survive
restarts and keep their full context:

- The **first message** auto-titles the thread; reopening the panel resumes your most recent one.
- The **☰** button opens the thread history — switch between chats, start a **＋ New chat**, or
  **🗑 delete** one (which removes all of its messages).
- Assistant turns store their tool-call trace alongside the text, so a reopened chat still shows
  what the model looked at.

## Where settings and authentication live

- Provider, model, URL, and the data-sharing toggle are stored in the app's local `app_settings`
  table.
- Saved chats live in the encrypted database too: `chat_threads` (one row per conversation) and
  `chat_messages` (its turns, including the tool-call trace). Deleting a thread cascade-deletes its
  messages.
- TrueNorth does **not** store a Copilot token. It asks the local GitHub CLI for the currently
  authenticated account when starting the bundled Copilot runtime.
