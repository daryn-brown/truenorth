# AI advisor ("second brain")

TrueNorth has a built-in **AI advisor** that answers questions about **your own financial data** —
net worth, accounts, cashflow, your goal, holdings, and recent transactions. It's the "ask the LLM
about my finances" workflow, but the model reads a live snapshot straight from your local database
instead of you pasting screenshots into a chatbot.

Open it from the **🧠 Ask AI** button in the dashboard header.

Everything is **read-only** and **opt-in per provider**. You choose where the model runs:

| Provider | Cost | Where it runs | What's sent off-device |
| --- | --- | --- | --- |
| **GitHub Models** | Free with your GitHub account | GitHub's API | Your question + a snapshot of your finances (or only rounded aggregates in privacy mode) |
| **Ollama** | Free | Fully local on your machine | Nothing — never leaves your device |

## Option A — GitHub Models (free, recommended)

[GitHub Models](https://github.com/marketplace/models) gives you free access to frontier models
(OpenAI GPT-4o, etc.) using a **GitHub personal access token** as the API key. If you have a GitHub
account, you already have access.

1. Create a token at [**github.com/settings/tokens**](https://github.com/settings/tokens).
   - A **fine-grained** token works; the only permission it needs is the **`models:read`** scope
     (under *Account permissions → Models*). A classic token with no extra scopes also works.
   - You don't need to grant it any repository access.
2. In TrueNorth, open **🧠 Ask AI → ⚙️ Settings**, make sure **GitHub Models** is selected, paste the
   token into **GitHub token**, and click **Save**. The token is stored locally (never shown again,
   never sent anywhere except GitHub's API as the bearer token).
3. Optionally click **Load available models** and pick one. The default is `openai/gpt-4o-mini`,
   which is fast and well within the free tier.
4. Ask away.

> **Free-tier limits.** GitHub Models has per-minute and per-day request limits. If you hit them
> you'll see a clear "rate limited" message — wait a moment, or switch to a smaller model or to
> Ollama.

Model ids are `publisher/model` (for example `openai/gpt-4o-mini`, `openai/gpt-4o`,
`meta/llama-3.1-8b-instruct`). The full list is in the
[model catalog](https://github.com/marketplace/models).

## Option B — Ollama (fully local, fully private)

[Ollama](https://ollama.com) runs open models entirely on your machine — nothing is ever sent off
your device, regardless of the privacy setting.

1. Install Ollama and pull a model:
   ```sh
   ollama pull llama3.1
   ```
   (Ollama serves an OpenAI-compatible API on `http://localhost:11434` while running.)
2. In **🧠 Ask AI → ⚙️ Settings**, select **Ollama (local)**. The default URL
   (`http://localhost:11434/v1`) and model (`llama3.1`) work out of the box; click **Load available
   models** to pick another that you've pulled.
3. Ask away. If Ollama isn't running you'll get a "couldn't reach the AI provider" hint — start it
   with `ollama serve` (or just launch the app).

## What data the model sees

Before each answer, TrueNorth assembles a **snapshot** from your local database and sends it to the
selected model as context. The financial figures come from the same calculations the dashboard
shows — net worth (USD + CAD), per-account balances, cashflow (income / fixed / variable / savings
rate), goal progress, holdings, and your most recent transactions.

Two settings control this, in **🧠 Ask AI → ⚙️ Settings → Data sharing**:

- **Send my real balances & transactions (default).** The snapshot includes exact figures — the
  most accurate answers. Recommended for Ollama always, and fine for GitHub Models if you're
  comfortable sending the data to GitHub's API.
- **Privacy mode (toggle off).** Only **rounded aggregates** are sent: net worth to the nearest
  $1,000, account count, savings rate, and goal progress — no exact balances, holdings, or
  individual transactions. Useful if you want GitHub Models' quality without sharing line-item
  detail.

With **Ollama**, the data never leaves your machine either way, so privacy mode mainly just shortens
the prompt.

The advisor is grounded: it's instructed to answer **only** from the snapshot and to say so when the
data needed isn't there, rather than inventing numbers. It's an **educational tool, not licensed
financial or tax advice**.

## Where settings and the token live

- Provider, model, URL, and the data-sharing toggle are stored in the app's local `app_settings`
  table.
- The GitHub token is stored in the local secret store (`secrets.json` in the app data folder, the
  same place the database key lives in [open mode](../README.md#privacy)). It is never written to
  the repo and never returned to the UI after you save it.

To remove the token, clear the **GitHub token** field and save, or just switch to Ollama.
