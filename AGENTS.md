# AGENTS.md — instructions for AI agents working in this repo

This repository has a strict **commit attribution policy**. Read this before making commits.

## Human-only attribution (required)

- **Do NOT add `Co-authored-by:` trailers for any AI or bot** — including
  `Co-authored-by: Copilot <...>`, `*[bot]` accounts, or AI-vendor noreply addresses
  (e.g. `@anthropic.com`, `@openai.com`).
- **Every commit must be authored and committed by the human repository owner.**
  Do not change `user.name` / `user.email` to a bot or generic identity.
- The owner does not want any non-human on the GitHub **contributors list**.

This is enforced by a `commit-msg` hook in [`.githooks/`](.githooks/) that strips AI/bot
co-author trailers, but **you must not add them in the first place**.

## Run builds locally, not via the cloud agent

- Do all work in **local** Copilot CLI sessions so commits are authored by the owner.
- The **cloud** Copilot coding agent commits as the `Copilot` bot account, which **would**
  appear as a contributor. Do not use it for this repo.

## When you commit

- Write a normal message (subject + optional body). **No AI/bot co-author trailers.**
- After committing, you may verify with:
  `git log --format='%an <%ae>%n%b' -n 5 | grep -i 'co-authored-by' || echo clean`
