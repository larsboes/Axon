# finance

Journal-backed cash flow, budgets, reviewed bank imports and subscriptions with
append-only price and state history.

## Why a capability

hledger is the first accounting engine. The private plaintext journal is canonical,
Postgres holds a disposable index, and this capability owns the review path and the
product above both.

The split is not a compromise between build and adopt. A plaintext journal under
git satisfies the knowledge-boundary's V1 and an index rebuilt from it satisfies
V2, so choosing the storage format did the boundary work rather than a rule someone
has to remember. What gets built here is the layer nothing off the shelf does well.
Double-entry semantics, lot accounting, price feeds and return math are not built
here: `hledger roi` already computes IRR and TWR, and `pricehist` already emits `P`
directives. Source parsing is different. Axon owns the typed CSV adapter, duplicate
detection, explicit review and journal write because those are product policy, not
accounting-engine policy.

## What exists today

- `AccountingEngine` defines check, transaction, register, balance, budget, cash-flow
  and ROI reports without exposing hledger syntax. `HledgerEngine` invokes hledger
  with `--no-conf` and normalizes its output.
- A configurable CSV adapter handles column names, delimiters, decimal marks,
  currencies and symbolic source accounts. It emits SHA-256-addressed
  `TransactionCandidate` values and discards the raw rows.
- A separate investment activity preview maps signed quantities, source references
  and optional exact-decimal unit prices. Preview is read-only; explicit confirmation
  re-runs the adapter and atomically stores only aggregate holdings in a private
  snapshot. Instrument aliases remain private mapping data.
- Candidates stay pending until the local UI confirms or rejects them. Confirmation
  validates the prospective journal, appends once, and atomically rebuilds the
  Postgres transaction projection. A retry cannot duplicate the posting.
- `/finance` has Overview, Budget, Transactions and Subscriptions. KPI cards, monthly
  cash flow, budget variance, the table and the interactive Sankey all use the same
  Rust projection. Internal transfers are excluded by default.
- Subscription prices and states remain append-only, with conflict-safe Obsidian
  writeback through `libs/markdown-root`.

## A subscription is not a row with a price

Every tool in this space stores the price as one mutable number. That number
answers "what am I paying now" and nothing else. It cannot say what this has cost
since it started, and it cannot notice a provider raising the price, because the
moment the new figure is written the old one is gone.

So a subscription carries two append-only series:

- **Price points.** A change appends a row with its date and reason. Two rows give
  true cost since inception and a drift signal; one mutable number gives neither.
- **State changes.** `considering → trial → active ⇄ paused → cancelled`, each with
  a date and a note, so "when did I pause this and why" survives the year.

The schema makes that structural. There is no `price` column on `subscriptions` and
no `status` column, because a column is a thing that can be updated. What the
current price *is* comes from `price_at()` over the series. There is no cached
total either: a stored figure is a second source of truth that goes stale silently.

`trial` counts as billing even at a price of zero. The price series says what it
costs and the state says whether you are on the hook, which is the case that makes
collapsing the two into one field wrong.

## Ownership across the vault boundary

| Statement | Owner | Written by |
|---|---|---|
| Why I pay for this, value check, alternatives | the vault note's prose | the human |
| Price history, state history, computed burn | Postgres | this capability |
| Current price, monthly equivalent, drift | the vault note, marked region | this capability, regenerable |
| Confirmed postings | private plaintext journal | this capability, after explicit review |
| Import candidates and transaction projection | Postgres | this capability, rebuildable |
| Budget targets | private `config/finance.json` | the human |
| Reviewed aggregate holdings | private configured snapshot | this capability, after explicit review |
| Holdings dashboard projection | Postgres | this capability, rebuildable from the private snapshot |

Writeback goes through `libs/markdown-root`'s region writer, which preserves every
byte outside the markers and refuses to overwrite a region a human edited. Nothing
here opens a file for writing by any other path. A conflict is reported and
counted, never resolved: the response names each conflicting note so the operator
can look at it.

Frontmatter seeds a subscription and is then not re-read for those fields. A
re-import would otherwise throw away every price change recorded since, because the
single cost figure in the note was only ever a starting point.

## HTTP surface

On the manifest-declared port. `GET /routes` serves the full manifest.

- `GET /health` · `GET /ready` (liveness and a reachable database, judged separately)
- `GET /api/subscriptions`
- `GET /api/subscriptions/burn?at=YYYY-MM-DD`
- `POST /api/subscriptions/:id/price` · `POST /api/subscriptions/:id/state`; both append
  idempotently, and the response says whether a new history point was created
- `GET /api/import/obsidian/scan` · `POST /api/import/obsidian`
- `POST /api/writeback`
- `GET /api/import/csv/mappings`
- `POST /api/import/csv` · `GET /api/import/candidates`
- `GET /api/import/investments/mappings` · `POST /api/import/investments/preview`
- `POST /api/import/investments/confirm`
- `POST /api/import/candidates/:id/review`
- `GET /api/ledger/check` · `POST /api/ledger/rebuild`
- `GET /api/dashboard?start=&end=&account=&category=&currency=`

## Configuration

Database from `$AXON_FINANCE_DATABASE_URL`, else the overlay's
`config/postgres.env`, else a localhost development fallback. Vault location from
the overlay's `config/finance.json`, or `AXON_FINANCE_OBSIDIAN_ROOT` for
development. `journal` and `budgets` live in that same private file; the journal can
also be set with `AXON_FINANCE_JOURNAL`. `schemas/finance.json.example` documents the
shape without carrying private deployment values. Named `csv_mappings` also live in
that private file; the loopback API supplies them to the local review UI, where the
operator can still edit every field before staging.
Named `investment_csv_mappings` supply the corresponding preview-only adapter. The
source identifier to symbolic commodity mapping belongs there rather than in Axon.
`investment_snapshot` names the private canonical aggregate written after review;
raw CSV rows and mapping values are never written to it. Prices shown in Overview are
latest activity prices, not live quotes or a claim about current market value.

Budget entries have `account`, `monthly_cents` and an optional `currency`. Accounts
stay symbolic. Real account numbers and the mapping from a symbol to an institution
belong in Vaultwarden, never in the journal or public config.

## Two representation choices

**Money is integer cents.** A monthly burn is a sum of divisions, since a yearly
plan contributes a twelfth, and floating point produces figures that disagree with
the bank by a cent for reasons nobody can reconstruct later. Conversion rounds half
away from zero rather than truncating, because truncation loses a cent per
subscription per month and the total drifts below reality.

**Dates are ISO-8601 strings.** They sort lexicographically, which is the only
operation performed on them. `trips` set the precedent, and a date library would be
a dependency bought for `<=`.

Weekly converts at 52/12, not four weeks a month. Four-week months are eleven
months of the year, and the error runs toward under-reporting what you spend.

## Ledger and engine boundary

The account tree, trip tag and public conventions are fixed and validated as
[`schemas/finance-journal.example`](../../schemas/finance-journal.example), because
the shape of the tree is a foreclosing call and writing it down after the importer
exists means writing it around the importer's accidents.

Two decisions in there worth naming:

**Accounts are symbolic.** `assets:bank:checking`, never an IBAN. Git history is
permanent and the knowledge-boundary requires that forgetting stay possible, so the
mapping from a symbolic name to a real account lives in Vaultwarden. This holds for
comments and commit messages too.

**A trip is a posting tag, not a branch.** A trip expense is also a food expense.
Duplicating the category tree under a trip prefix would make "what did I spend on
food this year" answerable only by remembering to union two subtrees. `hledger
balance tag:trip=<slug>` is the join, and the slug matches the trips capability's
plan, which is how the two agree without either reading the other's store.

`service.toml` declares `ledger = "hledger"`. That makes the existing scoped
toolchain check require hledger only on machines that enable Finance. Version 1.52.1
is pinned in `upstreams.toml` with its GPL-3.0-or-later licence. It remains a
separately installed executable. Axon invokes it but does not ship it with the MIT source.

Replacement is a measured decision. A native Rust engine can implement the same
contract later, but it must beat the existing adapter on a golden synthetic journal
before the system pays to recreate double-entry and return semantics.

## Related tools and why this is not them

| Tool | Good at | Relationship |
|---|---|---|
| [Actual Budget](https://actualbudget.org) | Envelope budgeting, fast local-first UI | Rejected as core. Its automatic German bank sync ran through GoCardless Bank Account Data, which stopped accepting new accounts in July 2025 |
| [Firefly III](https://firefly-iii.org) | A serious rule engine and a real REST API | PHP with its own database. A second store inside Axon, and a ledger that is not git-diffable, so agent writes stop being reviewable |
| [Ghostfolio](https://ghostfol.io) | Portfolio math, price feeds, allocation | A candidate for the investment half later. It owns valuation well and models a subscription's history not at all |
| [hledger](https://hledger.org) | Double-entry, commodities, reports and `roi` | Adopted as the first external engine behind `AccountingEngine`; not the importer and not bundled |
