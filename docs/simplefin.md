# Connecting banks with SimpleFIN

TrueNorth can pull **real, read-only balances** (and investment holdings, where the institution
reports them) from banks and other institutions through [**SimpleFIN**](https://www.simplefin.org).
Synced balances flow straight into the existing multi-currency net worth and history chart: each
sync writes one balance snapshot per account, so no part of the net-worth pipeline changes.

Everything is **read-only**. SimpleFIN only ever exposes balances and transactions — there is no
way to move money — and TrueNorth requests `balances-only` data.

SimpleFIN complements [SnapTrade](snaptrade.md): use **SnapTrade** for brokerages (Robinhood,
Questrade, Wealthsimple) and **SimpleFIN** for banks and cards. You can run both at once.

## What you need

A SimpleFIN account with a **bridge** that has at least one institution connected. The
[SimpleFIN Bridge](https://bridge.simplefin.org) costs about **$15/year** and covers multiple
institutions.

1. Sign in to your [SimpleFIN bridge](https://bridge.simplefin.org) and connect your bank(s).
2. Create a **setup token**: click **Connect** (sometimes "Connect to an app") to generate a
   one-time token — a long Base64 string. You'll paste it into TrueNorth once.

> A setup token can only be claimed **once**. After you connect it in TrueNorth, it's exchanged for
> a durable *access URL* and can't be reused. If a claim fails, generate a fresh token.

## Connecting

Open the app, click **🔗 Connect** in the header, and choose the **Banks** tab:

1. **SimpleFIN setup token** — paste your setup token and click **Connect**. TrueNorth claims the
   token, exchanges it for an access URL, verifies the access URL works, and stores it in your
   **OS keychain** (macOS Keychain / Windows Credential Manager) — never on disk or in the database.
2. **Sync balances** — click **Sync now**. TrueNorth pulls your accounts and balances and updates
   your net worth.

To connect more institutions, add them in your SimpleFIN bridge — they appear automatically on the
next sync. To rotate credentials, click **Use a new token** and claim a fresh setup token.

## What a sync does

For each account SimpleFIN reports, TrueNorth (in a single transaction):

- **Upserts the account**, keyed by its SimpleFIN account id (`connector_ref`), so re-syncing
  updates the existing row instead of creating duplicates. The account type is inferred from the
  account name (e.g. chequing, savings, credit, TFSA, RRSP, brokerage) and the jurisdiction from the
  account currency (CAD → CA, otherwise US).
- **Writes today's balance snapshot** (`source = 'simplefin'`). Because net worth and the history
  chart read the latest snapshot per account, your real balance appears immediately.
- **Replaces the account's holdings** with any positions the institution reports (symbol, shares,
  per-share price + average cost derived from SimpleFIN's market-value and cost-basis totals), so
  closed positions disappear. Most banks report no holdings — that's expected.

If SimpleFIN reports a per-connection problem (for example, an institution needs to be
re-authenticated at the bridge), the sync still succeeds for everything else and surfaces the
message as a **warning** under the sync summary.

Sync is **manual** ("Sync now"). Automatic/background sync is deferred to a later phase.

## Disconnecting

**Disconnect SimpleFIN** (in the Connect dialog) removes the stored access URL from the keychain and
hides the connected accounts. Historical snapshots already written are left untouched. To fully
revoke access, also disable or delete the token in your SimpleFIN bridge.

## Privacy & security

- **Secrets never touch disk.** The SimpleFIN access URL — which embeds HTTP Basic credentials — is
  stored only in the OS keychain. Nothing about SimpleFIN is written to the database except the
  non-secret accounts and balances you sync.
- **Read-only by design.** TrueNorth requests `balances-only` data from the SimpleFIN protocol.
- **Direct HTTPS.** Requests go only to your SimpleFIN server (e.g. `bridge.simplefin.org`) over
  HTTPS (rustls). No third party sees your data.
- Nothing related to SimpleFIN is committed to the repo; `.env`, `*.db`, and `*.sqlite` are
  gitignored.

## Troubleshooting

- **"SimpleFIN rejected the access URL…"** Your stored credentials are no longer valid (or the token
  was already claimed). Click **Use a new token**, generate a fresh setup token in your bridge, and
  reconnect. If you think the old token leaked, disable it in the bridge.
- **"Paste the setup token… first."** The token box was empty. Copy the full Base64 token from your
  SimpleFIN bridge.
- **A connection warning after syncing.** SimpleFIN flagged one institution (often it needs to be
  re-authenticated at the bridge). Fix it in the bridge, then sync again — other accounts are
  unaffected.
- **No accounts after syncing.** Make sure at least one institution is connected in your SimpleFIN
  bridge before syncing.
- **A balance is missing from net worth.** Net-worth conversion currently supports **USD and CAD**.
  Accounts in other currencies sync, but contribute 0 until multi-currency conversion is expanded.

## Cross-references

- [`README.md`](../README.md) — architecture diagram and roadmap.
- [`docs/snaptrade.md`](snaptrade.md) — connecting brokerages via SnapTrade (read-only).
- [`docs/import.md`](import.md) — manual / CSV import and how net-worth history is computed.
- [`docs/blueprint.md`](blueprint.md) — connector research, including SimpleFIN vs. alternatives.
