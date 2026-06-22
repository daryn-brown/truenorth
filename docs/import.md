# Importing data & net-worth history

Phase 1 is manual-first: you enter or **import** accounts and **balance snapshots**, and the app
derives multi-currency net worth and a net-worth-over-time chart from them. This page documents the
import formats and how the history series is computed.

## Importing accounts + balance history

Open the app and click **⬆️ Import** in the header. You can either paste JSON or choose a
`.json` / `.csv` file. Imports are **idempotent**: accounts are matched case-insensitively on
`(institution, name)` and reused if they already exist, and snapshots upsert on
`(account, date)` — so re-importing the same data never creates duplicates.

### JSON

```json
{
  "accounts": [
    {
      "name": "Chequing",
      "institution": "Scotiabank",
      "account_type": "chequing",
      "currency": "CAD",
      "jurisdiction": "CA",
      "notes": "main spending account",
      "snapshots": [
        { "snapshot_date": "2025-01-01", "balance": 4200.50 },
        { "snapshot_date": "2025-02-01", "balance": 5100.00 }
      ]
    }
  ]
}
```

A bare array of account objects (without the `accounts` wrapper) is also accepted.

| Field | Required | Notes |
| --- | --- | --- |
| `name` | yes | Account name; rows with an empty name/institution are skipped. |
| `institution` | yes | Used with `name` to match/reuse existing accounts. |
| `account_type` | yes | e.g. `chequing`, `savings`, `brokerage`, `tfsa`, `rrsp`, `fhsa`, `401k`, `ira`, `roth_ira`, `credit`, `crypto`, `other`. |
| `currency` | yes | Any ISO code (e.g. `USD`, `CAD`, `JMD`). Converted into the USD + CAD totals via the latest USD-pivot FX rate — run **🔄 Refresh FX** so a newly added currency is fetched. |
| `jurisdiction` | yes | `US` or `CA`. |
| `notes` | no | Free text. |
| `snapshots` | no | Array of `{ snapshot_date: "YYYY-MM-DD", balance: number }`. |

### CSV

The first row must be a header. Rows are grouped into accounts by `(institution, name)`; each row
with a `snapshot_date` + `balance` adds one snapshot.

```csv
institution,name,account_type,currency,jurisdiction,snapshot_date,balance
Scotiabank,Chequing,chequing,CAD,CA,2025-01-01,4200.50
Scotiabank,Chequing,chequing,CAD,CA,2025-02-01,5100.00
Chase,Checking,chequing,USD,US,2025-02-01,1800.00
```

Columns `institution` and `name` are required; the rest are optional (`account_type` defaults to
`other`, `currency` to `USD`, `jurisdiction` to `US`). Quoted fields with embedded commas are
supported, and `$`/`,` are stripped from balances.

## How net-worth history is computed

The **Net Worth Over Time** chart comes from the `get_net_worth_history` command. For every date on
which any account has a snapshot, each account's **most recent balance as of that date is carried
forward** (an account contributes 0 before its first snapshot). Each balance is converted to USD and
CAD and summed.

**FX simplification:** history uses the **latest stored FX rates** (USD as the pivot currency) for
every point, so the trend reflects your balance changes rather than day-to-day currency noise.
Per-date historical FX rates are a future (Phase 2+) enhancement. Refresh current rates any time
with **🔄 Refresh FX** — it fetches a rate for every currency your accounts use.

Only **active** accounts are included, consistent with the net-worth summary card.
