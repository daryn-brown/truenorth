# Connecting banks with SimpleFIN

TrueNorth can pull **real, read-only balances** (and investment holdings, where the institution
reports them) from banks and other institutions through [**SimpleFIN**](https://www.simplefin.org).
Synced balances flow straight into the existing multi-currency net worth and history chart: each
valid dated balances update their source-date snapshots, preserving the existing net-worth pipeline.

Everything is **read-only**. SimpleFIN only ever exposes balances and transactions — there is no
way to move money — and TrueNorth requests up to 90 calendar days of transaction history.

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

Open **Connections** from the sidebar or **Connect account** in the header, and choose **Banks**:

1. **SimpleFIN setup token** — paste your setup token and click **Connect**. TrueNorth claims the
   token and saves the access URL in the app's existing local credential store before fetching
   any bank data. A temporary outage therefore cannot lose an already-claimed setup token.
2. **Sync balances** — click **Sync now**. TrueNorth pulls your accounts, balances, and up to 90
   calendar days of transactions, then updates your net worth.

To connect more institutions, add them in your SimpleFIN bridge — they appear automatically on the
next sync. To rotate credentials, click **Use a new token** and claim a fresh setup token.

## Connection health and reauthentication

The floating **Sync accounts** button at the dashboard's bottom-right synchronizes all configured
providers (SimpleFIN, SnapTrade, Teller, and direct Questrade), independently of **Refresh FX**.
One provider failing does not stop the others. The button stays visible when scrolling and shows
progress; the page reserves space below its final account row.

The **Banks** tab shows each institution's authentication status and each account's **balance as
of** date. **Reconnect** opens SimpleFIN's portal and identifies the affected bank; select it there
and complete its password, code, or banking-app approval. Return to TrueNorth for one automatic
check, or use **I've finished - check balances**. Keep the existing bank connection and app token.
SimpleFIN does not document a per-bank reconnection URL, so the portal is opened rather than an
invented deep link.

Bank MFA is separate from app access. A bank-level `con.auth` message affects only that connection;
an app-level authentication failure prompts for a replacement setup token. Unknown and legacy
unscoped errors remain visible without guessing which institution needs authentication.

**Last SimpleFIN response** is not a bank refresh time. Balances at least **48 hours old**, missing
accounts, and failed or unverified connections trigger account and total-wealth warnings.
Last-known balances remain included in totals; accounts with no usable balance do not contribute.
Old installations show unverified status until their first health-aware sync.

SimpleFIN normally updates daily. Automatic SimpleFIN checks run at most every **six hours while
the desktop app is open**. Manual and return-from-browser checks have a **one-minute cooldown**;
a persisted rolling budget limits the app to **24 requests per 24 hours**, including failed attempts.
Repeated requests cannot bypass a bank's MFA or force a bank refresh.

## What a sync does

For each account SimpleFIN reports, TrueNorth (in a single transaction):

- **Upserts the account**, keyed by its connection and account ID in protocol v2, so re-syncing
  updates the existing row instead of creating duplicates. The account type is inferred from the
  account name (e.g. chequing, savings, credit, TFSA, RRSP, brokerage) and the jurisdiction from the
  account currency (CAD → CA, otherwise US). Questrade accounts are always classified as CA,
  including USD-denominated accounts.
- Existing unscoped account IDs are migrated without replacing the local account or losing history,
  transaction classifications, or user-corrected currencies.
- **Writes the bank's dated balance snapshot** (`source = 'simplefin'`), never a made-up "today"
  date. Missing, invalid, or future dates and authentication failures keep the previous balance
  with a warning. Older source timestamps cannot overwrite a newer saved balance.
- **Replaces the account's holdings** with any positions the institution reports (symbol, shares,
  per-share price + average cost derived from SimpleFIN's market-value and cost-basis totals), so
  closed positions disappear. An omitted holdings field or failed bank connection preserves the
  previous positions rather than erasing them. A successfully reported empty list clears positions.
- **Imports transactions from up to 90 calendar days**, keyed by SimpleFIN transaction id so later
  syncs update existing records instead of duplicating them.

If SimpleFIN reports a per-connection problem (for example, an institution needs to be
re-authenticated at the bridge), the sync still succeeds for everything else and surfaces the
message as a **warning** under the sync summary.

> **Brokerages may report cash only.** For some investment accounts, the SimpleFIN bridge returns
> just the **uninvested cash** balance and not the stock equity (market value) — Questrade is a known
> case. If a brokerage balance looks too low, connect that broker directly instead: see
> [**Questrade (direct API)**](questrade.md), under the **Direct** tab. The direct connector pulls
> the full account value (cash **and** equity) and automatically hides the redundant cash-only
> SimpleFIN duplicate so net worth isn't double-counted. Future SimpleFIN syncs keep that duplicate
> hidden while the direct Questrade account remains active.

## Disconnecting

**Disconnect SimpleFIN** (in Connections) removes the stored access URL from the credential store and
hides the connected accounts. Historical snapshots already written are left untouched. To fully
revoke access, also disable or delete the token in your SimpleFIN bridge.

## Privacy & security

- **Existing secret-store policy is preserved.** The access URL embeds HTTP Basic credentials and
  is stored through TrueNorth's local credential store. Open mode stores secrets in a local file
  with restrictive permissions; see [Privacy](../README.md#privacy). The encrypted finance database
  holds financial records and non-secret connection-health metadata, not the access URL.
- **Read-only by design.** TrueNorth requests account data and a transaction window of up to 90
  calendar days from the SimpleFIN protocol.
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
- **A balance is missing from net worth.** Check the source balance/date and any connection warning.
  Accounts in other currencies are supported; use **Refresh FX** if their conversion rate is missing.

## Cross-references

- [`README.md`](../README.md) — architecture diagram and roadmap.
- [`docs/snaptrade.md`](snaptrade.md) — connecting brokerages via SnapTrade (read-only).
- [`docs/questrade.md`](questrade.md) — connecting Questrade directly (full cash + equity).
- [`docs/import.md`](import.md) — manual / CSV import and how net-worth history is computed.
- [`docs/blueprint.md`](blueprint.md) — connector research, including SimpleFIN vs. alternatives.
