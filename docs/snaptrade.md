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
  discover what is connected, then sync only the accounts you select.
- **Commercial keys** — anything else. TrueNorth registers and manages the SnapTrade user for you;
  there's nothing extra to paste.

## Connecting an account

Open the app and click **🔗 Connect** in the header. The steps mirror the SnapTrade flow:

1. **SnapTrade API key** — paste your Client ID and Consumer Key, then **Save & verify**.
   TrueNorth validates the pair against SnapTrade before saving anything. The Consumer Key is
   stored in the app's **local, owner-only secret store**, not the finance database or repository.
2. **SnapTrade user** —
   - **Personal key (`PERS-…`):** paste your **User ID** and **User Secret** from the dashboard and
     click **Link user**. The **Find mine** button looks up the User ID registered to your key so
     you only have to paste the secret. TrueNorth validates the pair before saving the secret to the
     local secret store.
   - **Commercial key:** nothing to do here — a user is created automatically in the next step.
3. **Authorize your brokerage** — click **Connect a brokerage** to open SnapTrade's secure
   **connection portal** in your browser, where you log in to your institution. When you're done,
   return to the app. *If you already linked a brokerage in the SnapTrade dashboard (common with
   personal keys), you can skip straight to choosing accounts.*
4. **Choose accounts to sync** — review each provider account's institution, name, inferred type,
    currency, and masked account number (when supplied). Choose **Create new account**,
    **Link to existing account**, or **Ignore**, then **Save choices**. New accounts are **not
    selected by default**. Existing imported accounts remain selected unless previously hidden.
5. **Sync balances** — click **Sync now** to update only your selected accounts.

You can connect more than one brokerage — repeat step 3 (**Connect another brokerage**) and review
the new accounts before syncing. **Choose accounts to sync** stays available next to **Sync now**
for existing installations too. Selecting Questrade does not also import Robinhood just because
both appear under the same SnapTrade login. Provider badges distinguish SnapTrade, SimpleFIN,
Teller, and direct Questrade accounts; no badge means manual.

## Linking, ignoring, and existing duplicates

- **Create new** creates one local account for that exact provider ID when choices are saved.
   Saving/discovering does not fetch SnapTrade positions or write balance snapshots. Subsequent
   syncs reuse the saved account ID, not a name/balance/account-number match.
- **Link to existing** is an explicit source switch, confirmed in an in-app dialog. The target
   must be an active, currency- and type-compatible manual or SimpleFIN account. Its local ID,
   user labels, jurisdiction, notes, historical snapshots, transaction classifications, and goal
   references stay intact. A prior SimpleFIN binding is persistently excluded, so later SimpleFIN
   syncs cannot recreate the old source. Linking Teller/direct accounts or combining different
   account IDs from the same provider is not supported.
- **Ignore** is remembered across syncs and restarts. For an already imported duplicate, it hides
   that row from accounts, net worth, cashflow, and history charts, but **retains its stored data**.
   The original account is untouched. An old SimpleFIN exclusion after a handoff does not hide
   the account now supplied by SnapTrade.
- **Already-imported accounts cannot be reassigned or merged.** To clean up an existing duplicate,
   Ignore its SnapTrade entry and keep the original account. This release does not combine its
   historical records with another account. **Resume syncing to the same account** deliberately
   restores its existing row, after confirmation, rather than creating another copy.
- **Delete account** uses an in-app confirmation identifying the row and provider. It performs the
   same durable exclusion for SnapTrade/SimpleFIN, with history retained. Cancel makes no changes;
   errors are visible. A successful deletion followed by a refresh failure offers a refresh retry,
   not another deletion.

Never link two distinct Individual accounts merely because their names or balances match. The
last four digits are displayed only to help you recognize an account, not as identity evidence.
No account is automatically merged by name, balance, institution, type, or partial number.

## What a sync does

TrueNorth first lists provider metadata and reads saved selections. Unreviewed and ignored
accounts do not trigger position requests. A sync reports how many new accounts still need review,
or a clear **No accounts are selected** message when there is nothing eligible to sync. Discovery
and save errors are not treated as permission to import everything.

For each selected account, TrueNorth (in a single transaction):

- **Rechecks the saved account ownership** after the network requests. A concurrent Ignore,
   Delete, source switch, disconnect, or other stale selection aborts the import instead of
   reactivating an account. Save requests also reject stale provider/local details and duplicate
   target assignments atomically.
- **Preserves local account metadata**, including user-corrected currency. A newly created account
   gets its type from the brokerage's label and jurisdiction from its currency (CAD → CA, otherwise
   US). Questrade is always CA, including USD accounts. Existing direct-Questrade suppression
   still takes precedence while that connector is active.
- **Writes today's balance snapshot** (`source = 'snaptrade'`). Because net worth and the history
  chart read the latest snapshot per account, your real balance appears immediately.
- **Replaces the account's holdings** with the current positions (symbol, units, price, average
  cost, currency), so closed positions disappear.

Sync is **manual** ("Sync now"). Automatic/background sync is deferred to a later phase.

## Disconnecting

**Disconnect brokerage** (in the Connect dialog) removes the stored user secret from the secret store
and hides the connected accounts. Your **API key stays saved** so you can reconnect later without
re-entering it. Historical snapshots already written are left untouched. Selections remain stored
as exclusions; reconnecting does not silently reactivate them. Open **Choose accounts to sync**
and explicitly resume the accounts you want. New provider IDs require a new review.

- **Commercial keys:** the SnapTrade user is also deleted remotely.
- **Personal keys (`PERS-…`):** the user *isn't* deleted remotely — it's provisioned at signup and
  owns the brokerage connections you manage in the SnapTrade dashboard, so TrueNorth only clears its
  local copy of the secret. Re-link any time by pasting the secret again.

## Privacy & security

- **Local-only storage.** In the current open mode, the Consumer Key and SnapTrade user secret live
  in the owner-only secret file in the app data folder. The non-secret Client ID and user ID live
  in `app_settings`; account choices and financial data live in the encrypted local database.
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
- **No accounts selected / new accounts need review.** Finish the brokerage login, open **Choose
  accounts to sync**, and save your choices. Merely connecting a brokerage no longer imports it.
- **No compatible existing account.** Linking requires the same currency and inferred type.
  Hidden/deleted targets and Teller/direct accounts are not offered. Existing imports with a
  user-corrected currency can continue syncing; a new link cannot silently relabel currency.
- **Choices changed while saving or syncing.** Reload the review and check it again. No partial
  batch was saved or imported.
- **A balance is missing from net worth.** Confirm that the account is selected, a balance was
  reported, and an exchange rate is available for its currency.
- **Lost secret / "Connect a brokerage before syncing."** Re-open **🔗 Connect** and
  reconnect. For commercial keys TrueNorth automatically re-registers the user; for personal keys,
  paste your User Secret again in step 2.

## Cross-references

- [`README.md`](../README.md) — architecture diagram and roadmap.
- [`docs/import.md`](import.md) — manual / CSV import and how net-worth history is computed.
- [`docs/blueprint.md`](blueprint.md) — connector research, including SnapTrade vs. alternatives.

## Isolated regression checks

`npm run test:account-sync` exercises account review, explicit linking confirmation, persistent
choices, and Delete account with fictional data and mocked Tauri IPC in a local browser. External
requests are blocked. Rust selection/connector tests use in-memory databases and loopback HTTP
fixtures, never the installed app's database or credentials.
