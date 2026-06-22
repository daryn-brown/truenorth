# Connecting Questrade directly

TrueNorth can pull **real, read-only balances and holdings** from [Questrade](https://www.questrade.com)
through Questrade's own free **personal API** — no aggregator in between. Unlike SimpleFIN, which (via
its data provider) often reports only the **uninvested cash** in a Questrade account, the direct
connector reads the account's **full value: cash *and* stock equity (market value)**. That total
flows straight into your multi-currency net worth and history chart: each sync writes one balance
snapshot per account, so no part of the net-worth pipeline changes.

Everything is **read-only**. The Questrade personal API only ever exposes account data — there is no
way to move money.

> **Why a direct connector?** [SimpleFIN](simplefin.md) is great for banks, but for some brokerage
> accounts its data provider returns cash only, not equity — Questrade is a known case. [SnapTrade](snaptrade.md)
> can read Questrade's full balance, but its Questrade integration is intermittently under
> maintenance. Connecting Questrade directly is the most reliable way to get true equity into net
> worth, and it's free.

## What you need

A Questrade account with **API access** enabled. There is no cost and no client secret for personal
use.

1. Sign in at [**login.questrade.com**](https://login.questrade.com) and open the **API Centre**
   (Account → *API access*, or [API Centre → Register a personal app](https://login.questrade.com/APIAccess/UserApps.aspx)).
2. **Register a personal app** (any name; you only need read access).
3. Click **Generate new token** to run a **manual authorization**. Questrade shows a **refresh
   token** — a short string. You'll paste it into TrueNorth once.

> **Refresh tokens are single-use and rotate.** Every time TrueNorth contacts Questrade it receives a
> *new* refresh token and immediately stores it, replacing the old one. An **unused** refresh token
> expires after about **7 days** — if that happens, just generate a new one in the API Centre and
> reconnect.

## Connecting

Open the app, click **🔗 Connect** in the header, and choose the **Direct** tab → **Questrade**:

1. **Questrade refresh token** — paste the token from the API Centre and click **Connect**. TrueNorth
   exchanges it for a short-lived access token, verifies it can list your accounts, and stores the
   rotated refresh token in your **OS keychain** (macOS Keychain / Windows Credential Manager) —
   never on disk or in the database. The access token is kept only in memory.
2. **Sync balances** — click **Sync now**. TrueNorth pulls your accounts, balances, and positions and
   updates your net worth.

To rotate the connection, click **Use a new token** and paste a fresh refresh token.

## What a sync does

For each **active** account Questrade reports, TrueNorth (in a single transaction):

- **Upserts the account**, keyed by its Questrade account number (`connector_ref`), so re-syncing
  updates the existing row instead of creating duplicates. The type is mapped from Questrade's
  account type (TFSA → tfsa, FHSA → fhsa, RRSP/LIRA/LIF/RRIF → rrsp, RESP → other, Cash/Margin →
  brokerage) and the jurisdiction is always **CA**.
- **Writes today's balance snapshot** (`source = 'questrade'`) using **`totalEquity`** — cash **plus**
  market value — in the account's home currency (CAD preferred). Because net worth and the history
  chart read the latest snapshot per account, your real balance appears immediately.
- **Replaces the account's holdings** with the positions Questrade reports (symbol, open quantity,
  current price, average entry price), so closed positions disappear.

### Coexistence with SimpleFIN ("complement, not overwrite")

The Questrade connector writes **only its own** account rows (`connector_kind = 'questrade'`) and
never modifies your SimpleFIN or manual accounts. To avoid double-counting, each Questrade sync also
**hides any aggregator-managed account that points at Questrade** — i.e. an active SimpleFIN or
SnapTrade account whose institution name contains "Questrade". These are the redundant, often
cash-only duplicates of the accounts you now sync directly. They're **soft-deleted** (marked
inactive — your history is preserved), and the count is shown in the sync summary.

> **If a duplicate remains:** the automatic cleanup matches on the institution name reported by the
> aggregator. If your SimpleFIN bridge labels the institution as something other than "Questrade",
> the cash-only duplicate won't be detected. In that case, just hide that account from the **Accounts**
> list so it stops contributing to net worth.

Sync is **manual** ("Sync now"). Automatic/background sync is deferred to a later phase.

## Disconnecting

**Disconnect Questrade** (in the Connect dialog) removes the stored refresh token from the keychain
and hides the connected accounts. Historical snapshots already written are left untouched. To fully
revoke access, also delete the personal app in your Questrade API Centre.

## Privacy & security

- **Secrets never touch disk.** Only the Questrade refresh token is persisted, and only in the OS
  keychain. The access token lives in memory for the duration of a sync. Nothing about Questrade is
  written to the database except the non-secret accounts, balances, and holdings you sync.
- **Read-only by design.** The personal API exposes account data only — TrueNorth never sends a trade
  or transfer request.
- **Direct HTTPS.** Requests go only to Questrade (`login.questrade.com` and your account-specific
  `api*.iq.questrade.com` host) over HTTPS (rustls). No third party sees your data.
- Nothing related to Questrade is committed to the repo; `.env`, `*.db`, and `*.sqlite` are
  gitignored.

## Troubleshooting

- **"Questrade rejected the connection…"** The refresh token is invalid, already used, or expired
  (after ~7 days unused). Generate a new token in the [API Centre](https://login.questrade.com/APIAccess/UserApps.aspx)
  and reconnect with **Use a new token**.
- **"Paste the refresh token… first."** The token box was empty. Copy the full token from the API
  Centre.
- **"Connect Questrade before syncing."** No refresh token is stored — connect first.
- **A balance still looks low / a cash-only Questrade account is still showing.** A separate
  connection (e.g. SimpleFIN) is importing the same account but the auto-cleanup didn't match its
  institution name. Hide that duplicate account from the **Accounts** list.
- **A balance is missing from net worth.** Net-worth conversion currently supports **USD and CAD**.
  Accounts in other currencies sync, but contribute 0 until multi-currency conversion is expanded.

## Adding other institutions' APIs

The **Direct** tab is built to grow: any institution that offers a personal API can get its own card
beside Questrade. The backend pattern is a small connector module plus a `commands::*` file with
`*_get_status` / `*_connect` / `*_sync` / `*_disconnect`; the frontend adds one card component under
`DirectConnectionsPanel`. See `src-tauri/src/connector/questrade/` and
`src-tauri/src/commands/questrade.rs` as the reference implementation.

## Cross-references

- [`README.md`](../README.md) — architecture diagram and roadmap.
- [`docs/simplefin.md`](simplefin.md) — connecting banks via SimpleFIN (and the equity limitation).
- [`docs/snaptrade.md`](snaptrade.md) — connecting brokerages via SnapTrade (read-only).
- [`docs/import.md`](import.md) — manual / CSV import and how net-worth history is computed.
- [`docs/blueprint.md`](blueprint.md) — connector research, including Questrade vs. alternatives.
