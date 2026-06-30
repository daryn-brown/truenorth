# Connecting US banks with Teller (free)

TrueNorth can pull **real, read-only balances** from US banks through
[**Teller**](https://teller.io), which is **free for personal use**. Synced balances flow straight
into the existing multi-currency net worth and history chart: each sync writes one balance snapshot
per account, so no part of the net-worth pipeline changes.

Everything is **read-only** — TrueNorth requests Teller's `balance` product only, and there is no
way to move money.

Teller complements the other connectors: use **SnapTrade** for brokerages, **SimpleFIN** or
**Teller** for banks and cards, and **Questrade** for full cash + equity. Teller is the only
**free** option for *real* bank balances (SimpleFIN's bridge is ~$15/yr; Plaid's free tier is
sandbox-only fake data). It is **US-only**.

## What you need

A free [Teller](https://teller.io) account. From the Teller dashboard you get:

1. An **application id** (it looks like `app_xxxxxxxxxxxxxxxxxx`).
2. A choice of **environment**:
   - **Sandbox** — fake test data, **no certificate required**. Good for trying the flow (log in
     with Teller's test credentials, e.g. username `username` / password `password`).
   - **Development** — your **real** bank data, **free**, with a generous cap (~100 linked
     enrollments). This is the "free + real" path.
   - **Production** — real data for shipping an app to other people (paid beyond Teller's limits).
3. A **client certificate** + **private key** (PEM). Teller issues these from the dashboard and they
   are **required** for the `development` and `production` environments (they're used for mutual-TLS).
   Sandbox doesn't need them.

> Keep your private key safe. Anyone with your certificate **and** an access token could read those
> balances. TrueNorth stores both only in your local, owner-only secret store (never in the
> database, never committed to the repo).

## Connecting

Open the app, click **🔗 Connect** in the header, and choose the **US banks** tab:

1. **Teller application** — paste your **application id**, pick your **environment**, and (for
   development/production) paste your **client certificate** and **private key**. Click **Save
   configuration**. TrueNorth validates that the certificate and key parse as a TLS identity before
   storing them.
2. **Link a bank** — click **Link a bank** to open **Teller Connect** and log in to your bank.
   When it finishes, TrueNorth verifies the returned access token (by listing accounts, which also
   captures the institution name) and stores it. Repeat **Link another** for more banks.
   - *Already have an access token?* Use the small link under the button to paste a `token_…`
     access token directly — handy if the embedded Teller Connect window doesn't fully load.
3. **Sync balances** — click **Sync now**. TrueNorth pulls your accounts and balances and updates
   your net worth.

## What a sync does

For each account Teller reports, TrueNorth (in a single transaction):

- **Upserts the account**, keyed by its Teller account id (`connector_ref`), so re-syncing updates
  the existing row instead of creating duplicates. The account type is inferred from Teller's
  `type`/`subtype` (checking → chequing, savings/money-market → savings, credit card → credit), with
  a name-based fallback. All Teller accounts are treated as **US**.
- **Writes today's balance snapshot** (`source = 'teller'`). Because net worth and the history chart
  read the latest snapshot per account, your real balance appears immediately — and if you also sync
  the same account through another connector, the **freshest snapshot wins**.
- **Prefers the ledger balance** (total funds) over the available balance for net worth.
- **Stores credit-card balances as negative.** Teller reports a card's balance as a positive *amount
  owed*; TrueNorth flips the sign so liabilities subtract from net worth.
- **Skips closed accounts.**

If one enrollment fails (for example, the bank needs to be re-authenticated), the sync still
succeeds for everything else and surfaces the message as a **warning** under the sync summary.

Sync is **manual** ("Sync now"). Automatic/background sync is deferred to a later phase.

## Disconnecting

**Disconnect Teller** (in the Connect dialog) removes the stored enrollments **and** your client
certificate from the secret store and hides the connected accounts. Your saved application id and
environment, and any historical snapshots already written, are left untouched. To fully revoke
access, also delete the application or certificate in your Teller dashboard.

## Privacy & security

- **Secrets stay in the local secret store.** Access tokens and the client certificate/private key
  are kept only in TrueNorth's owner-only local secret store — never in the database, never in the
  repo. Non-secret config (application id, environment, last-synced time) lives in `app_settings`.
- **Read-only by design.** TrueNorth requests only Teller's `balance` product.
- **Direct HTTPS + mTLS.** Requests go only to `api.teller.io` over HTTPS (rustls), presenting your
  client certificate. No third party sees your data.

## Troubleshooting

- **"The development and production environments need a Teller client certificate."** Paste your
  certificate **and** private key in step 1, or switch the environment to **Sandbox**.
- **"That certificate or private key couldn't be read as PEM."** Copy the full PEM blocks, including
  the `-----BEGIN …-----` / `-----END …-----` lines, for **both** the certificate and the key.
- **"Teller rejected the request…"** The access token or certificate is wrong, or they belong to
  different Teller applications. Re-link the bank with Teller Connect and confirm the certificate is
  from the same app.
- **Teller Connect didn't open.** Check your internet connection (the widget loads from Teller's
  CDN), or use *Already have an access token?* to paste a token instead.
- **A balance is missing from net worth.** Net-worth conversion currently supports **USD and CAD**.
  Teller is USD, so this is rarely an issue.

## Cross-references

- [`README.md`](../README.md) — architecture diagram and roadmap.
- [`docs/simplefin.md`](simplefin.md) — connecting banks via SimpleFIN (paid bridge, US + Canada).
- [`docs/snaptrade.md`](snaptrade.md) — connecting brokerages via SnapTrade (read-only).
- [`docs/questrade.md`](questrade.md) — connecting Questrade directly (full cash + equity).
- [`docs/import.md`](import.md) — manual / CSV import and how net-worth history is computed.
