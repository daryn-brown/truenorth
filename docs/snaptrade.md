# Connecting brokerages with SnapTrade

Phase 2 lets TrueNorth pull **real, read-only balances and holdings** from brokerages —
Robinhood, Questrade, Wealthsimple, and [many others](https://snaptrade.com/global-brokerages) —
through [**SnapTrade**](https://snaptrade.com). Synced balances flow straight into the existing
multi-currency net worth and history chart: each sync writes one balance snapshot per account, so
no part of the net-worth pipeline changes.

Everything is **read-only**. TrueNorth requests a read-only connection and never asks for trading
scopes, so it cannot place orders or move money.

## What you need

A free SnapTrade developer account. The free tier covers a **single end user**, which is exactly
this app's model (one person, on one machine).

1. Sign up at the [SnapTrade dashboard](https://dashboard.snaptrade.com).
2. Copy your **Client ID** and **Consumer Key** from the dashboard. The Client ID identifies your
   app; the Consumer Key is the secret used to sign requests.

### Personal vs. commercial keys

SnapTrade issues two kinds of keys, and TrueNorth supports both:

- **Personal keys** — the Client ID starts with **`PERS-`**. These are the free, individual keys.
  SnapTrade **creates your user automatically at signup**, so the usual "register a user" call is
  unavailable. Instead you copy your **User ID** and **User Secret** from the dashboard once (see
  step 2 below). You can also link brokerages directly in the SnapTrade dashboard — TrueNorth will
  read whatever is connected.
- **Commercial keys** — anything else. TrueNorth registers and manages the SnapTrade user for you;
  there's nothing extra to paste.

## Connecting an account

Open the app and click **🔗 Connect** in the header. The steps mirror the SnapTrade flow:

1. **SnapTrade API key** — paste your Client ID and Consumer Key, then **Save & verify**.
   TrueNorth validates the pair against SnapTrade before saving anything. The Consumer Key is
   stored in your **OS keychain** (macOS Keychain / Windows Credential Manager) — never on disk or
   in the database.
2. **SnapTrade user** —
   - **Personal key (`PERS-…`):** paste your **User ID** and **User Secret** from the dashboard and
     click **Link user**. The **Find mine** button looks up the User ID registered to your key so
     you only have to paste the secret. TrueNorth validates the pair before saving the secret to the
     keychain.
   - **Commercial key:** nothing to do here — a user is created automatically in the next step.
3. **Authorize your brokerage** — click **Connect a brokerage** to open SnapTrade's secure
   **connection portal** in your browser, where you log in to your institution. When you're done,
   return to the app. *If you already linked a brokerage in the SnapTrade dashboard (common with
   personal keys), you can skip straight to Sync.*
4. **Sync balances** — click **Sync now**. TrueNorth pulls your accounts, balances, and positions
   and updates your net worth.

You can connect more than one brokerage — repeat step 3 (**Connect another brokerage**) and sync
again. Connected accounts are tagged **via SnapTrade** in the account list.

## What a sync does

For each account SnapTrade reports, TrueNorth (in a single transaction):

- **Upserts the account**, keyed by its SnapTrade account id (`connector_ref`), so re-syncing
  updates the existing row instead of creating duplicates. The account type is inferred from the
  brokerage's label (e.g. TFSA, RRSP, Roth IRA, 401(k)), and jurisdiction from the account currency
  (CAD → CA, otherwise US).
- **Writes today's balance snapshot** (`source = 'snaptrade'`). Because net worth and the history
  chart read the latest snapshot per account, your real balance appears immediately.
- **Replaces the account's holdings** with the current positions (symbol, units, price, average
  cost, currency), so closed positions disappear.

Sync is **manual** ("Sync now"). Automatic/background sync is deferred to a later phase.

## Disconnecting

**Disconnect brokerage** (in the Connect dialog) removes the stored user secret from the keychain
and hides the connected accounts. Your **API key stays saved** so you can reconnect later without
re-entering it. Historical snapshots already written are left untouched.

- **Commercial keys:** the SnapTrade user is also deleted remotely.
- **Personal keys (`PERS-…`):** the user *isn't* deleted remotely — it's provisioned at signup and
  owns the brokerage connections you manage in the SnapTrade dashboard, so TrueNorth only clears its
  local copy of the secret. Re-link any time by pasting the secret again.

## Privacy & security

- **Secrets never touch disk.** The Consumer Key and the SnapTrade user secret live only in the OS
  keychain. The non-secret Client ID and user id live in the local `app_settings` table.
- **Read-only by design.** The connection portal is opened with a read-only connection type.
- **Direct, signed HTTPS.** Requests go only to `api.snaptrade.com` over HTTPS (rustls) and are
  signed with HMAC-SHA256 per SnapTrade's request-signature scheme. No third party sees your data.
- Nothing related to SnapTrade is committed to the repo; `.env`, `*.db`, and `*.sqlite` are
  gitignored.

## Troubleshooting

- **"SnapTrade rejected the credentials."** Double-check the Client ID and Consumer Key, and that
  your SnapTrade account is active.
- **"This is a personal SnapTrade key…"** Your key is a `PERS-` key, so you must link your User ID
  and User Secret in step 2 before authorizing or syncing. Copy them from the SnapTrade dashboard
  (use **Find mine** to fill the User ID automatically).
- **"SnapTrade rejected those credentials. Double-check the User ID and User Secret…"** The User ID
  and User Secret pasted in step 2 don't match. Re-copy them from the dashboard; the User Secret can
  be rotated there if needed.
- **No accounts after syncing.** Make sure you finished the brokerage login in the browser portal
  (step 3) — or linked the brokerage in the SnapTrade dashboard — before clicking **Sync now**, then
  sync again.
- **A balance is missing from net worth.** Net-worth conversion currently supports **USD and CAD**.
  Accounts in other currencies sync, but contribute 0 until multi-currency conversion is expanded.
- **Lost keychain entry / "Connect a brokerage before syncing."** Re-open **🔗 Connect** and
  reconnect. For commercial keys TrueNorth automatically re-registers the user; for personal keys,
  paste your User Secret again in step 2.

## Cross-references

- [`README.md`](../README.md) — architecture diagram and roadmap.
- [`docs/import.md`](import.md) — manual / CSV import and how net-worth history is computed.
- [`docs/blueprint.md`](blueprint.md) — connector research, including SnapTrade vs. alternatives.
