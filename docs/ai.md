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
| **Ollama** | Free | Fully local on your machine | Nothing — never leaves your device |
| **GitHub Models** | ⚠️ **Retired by GitHub** | GitHub's API | Your question + a snapshot of your finances (or only rounded aggregates in privacy mode) |

> **⚠️ GitHub Models has been retired.** GitHub has shut down the free GitHub Models inference
> service, so it now returns an error (HTTP 410) instead of answers — no token change can bring it
> back. **Use Ollama (below) to keep the advisor working for free and fully offline.** If GitHub
> Models is your selected provider, TrueNorth will tell you it's retired and to switch. You can also
> point the app at any other OpenAI-compatible API.

## Option A — Ollama (fully local, free, recommended)

[Ollama](https://ollama.com) runs open models entirely on your machine — nothing is ever sent off
your device, regardless of the privacy setting. It's now the recommended provider.

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

## Option B — GitHub Models (retired)

> **This provider no longer works** — GitHub retired the GitHub Models inference API, so requests
> fail with HTTP 410. The steps below are kept for reference only; use **Ollama** instead.

[GitHub Models](https://github.com/marketplace/models) previously gave free access to frontier models
(OpenAI GPT-4o, etc.) using a **GitHub personal access token** as the API key.

1. Create a token at [**github.com/settings/tokens**](https://github.com/settings/tokens).
   - A **fine-grained** token works; the only permission it needs is the **`models:read`** scope
     (under *Account permissions → Models*). A classic token with no extra scopes also works.
   - You don't need to grant it any repository access.
2. In TrueNorth, open **🧠 Ask AI → ⚙️ Settings**, make sure **GitHub Models** is selected, paste the
   token into **GitHub token**, and click **Save**. The token is stored locally (never shown again,
   never sent anywhere except GitHub's API as the bearer token).
3. Optionally click **Load available models** and pick one. The default is `openai/gpt-4o-mini`.
4. Ask away.

Model ids are `publisher/model` (for example `openai/gpt-4o-mini`, `openai/gpt-4o`,
`meta/llama-3.1-8b-instruct`).

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
  always, and fine for GitHub Models if you're comfortable sending the data to GitHub's API.
- **Privacy mode (toggle off).** Tools are disabled. Instead, only a **rounded-aggregate snapshot**
  is sent: net worth to the nearest $1,000, account count, savings rate, and goal progress — no
  exact balances, holdings, or individual transactions. Useful if you want GitHub Models' quality
  without sharing line-item detail.

With **Ollama**, the data never leaves your machine either way, so privacy mode mainly just shortens
the prompt.

The advisor is grounded: it's instructed to answer **only** from your data and to say so when the
information it needs isn't there, rather than inventing numbers. It's an **educational tool, not
licensed financial or tax advice**.

## Saved chats (threads)

Each conversation is a **thread** saved in the local **encrypted** database, so your chats survive
restarts and keep their full context:

- The **first message** auto-titles the thread; reopening the panel resumes your most recent one.
- The **☰** button opens the thread history — switch between chats, start a **＋ New chat**, or
  **🗑 delete** one (which removes all of its messages).
- Assistant turns store their tool-call trace alongside the text, so a reopened chat still shows
  what the model looked at.

## Where settings and the token live

- Provider, model, URL, and the data-sharing toggle are stored in the app's local `app_settings`
  table.
- Saved chats live in the encrypted database too: `chat_threads` (one row per conversation) and
  `chat_messages` (its turns, including the tool-call trace). Deleting a thread cascade-deletes its
  messages.
- The GitHub token is stored in the local secret store (`secrets.json` in the app data folder, the
  same place the database key lives in [open mode](../README.md#privacy)). It is never written to
  the repo and never returned to the UI after you save it.

To remove the token, clear the **GitHub token** field and save, or just switch to Ollama.
