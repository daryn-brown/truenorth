# Contributing

## Commit attribution policy

Commits in this repository are **attributed to the human owner only**. No AI/bot (Copilot,
`*[bot]` accounts, AI-vendor identities) may appear in the history or on the GitHub
**contributors list**.

### One-time setup (every fresh clone)

Enable the repo's git hooks so AI/bot co-author trailers are stripped automatically:

```bash
git config core.hooksPath .githooks
```

> `core.hooksPath` is local to each clone and is **not** copied by `git clone`, so run this once
> per machine/clone. (Existing worktrees of the same repo already inherit it.)

The hook lives at [`.githooks/commit-msg`](.githooks/commit-msg). It removes any
`Co-authored-by:` line referencing Copilot, `*[bot]`, or AI-vendor noreply addresses, while
leaving human co-authors intact.

### Use local builds

Run Copilot in **local** CLI sessions so commits are authored by you. The **cloud** Copilot
coding agent commits as the `Copilot` bot account and **would** appear as a contributor — avoid
it for this repo.

### Verify before pushing

```bash
git log --format='A:%an <%ae> | C:%cn <%ce>' -n 10           # authors should all be you
git log --format='%B' | grep -i 'co-authored-by' || echo 'clean: no co-author trailers'
```
