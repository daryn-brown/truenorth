# GitHub Copilot — repository instructions

## Commit attribution: human-only (required)

When committing in this repository:

- **Never add a `Co-authored-by:` trailer for Copilot or any AI/bot.** Specifically do not add
  `Co-authored-by: Copilot <223556219+Copilot@users.noreply.github.com>` or trailers referencing
  `*[bot]` accounts or AI-vendor noreply addresses (`@anthropic.com`, `@openai.com`).
- **All commits must be authored by the human repository owner only.** The owner must not appear
  alongside any non-human on the GitHub contributors list.

A `commit-msg` hook in `.githooks/` strips such trailers as a safety net, but do not rely on it —
do not add the trailers at all.

## Run builds locally

Use **local** Copilot CLI sessions (commits authored by the owner). Do **not** use the cloud
Copilot coding agent for this repo — it commits as the `Copilot` bot and would show up as a
contributor.

See [`AGENTS.md`](../AGENTS.md) and [`CONTRIBUTING.md`](../CONTRIBUTING.md) for details.
