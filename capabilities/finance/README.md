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
  and optional exact-decimal unit prices. Private profiles can explicitly classify
  position-changing and non-position activity values; an unclassified nonzero
  quantity fails closed rather than inflating holdings. Preview is read-only;
  explicit confirmation re-runs the adapter and atomically stores only aggregate
  holdings in a private snapshot. Instrument aliases remain private mapping data.
- Candidates stay pending until the local UI confirms or rejects them. Confirmation
  validates the prospective journal, appends once, and atomically rebuilds the
  Postgres transaction projection. A retry cannot duplicate the posting. Confirmed
  uncategorized expenses can later be grouped locally by description and explicitly
  batch-reclassified; the selected journal postings are validated and replaced once.
- A confirmed expense can then receive a reviewed purpose and personal/shared split.
  The personal share remains on the expense account; money fronted for others posts
  to `assets:receivable:shared`. A linked repayment settles that receivable and is
  never projected as income or negative spending.
- `/finance` has Overview, Planning, Budget, Transactions and Subscriptions. Personal result,
  external cash movement, category composition, purpose/trip summaries, budget
  variance, the table and the constrained flow explorer all use the same Rust
  projection. Transactions starts with largest-first categorization review and a
  trip-first allocation workspace: Trips owns the plan and dates, while Finance loads
  that window and reviews each transaction's personal share. Internal transfers are
  excluded by default.
- Planning uses medians from complete months, private behavior rules and dated
  commitments to project monthly spending and savings. Exceptional trip spending is
  excluded from the recurring forecast. Reviewed balances and holdings add liquidity,
  runway and concentration; partial snapshots remain visibly partial.
- Subscription anomalies, card break-even calculations and loyalty values share that
  planning response. Provider terms need dated source links. Eligible spend can come
  from reviewed journal postings, but benefit use and point value remain explicit
  private assumptions, so an incomplete comparison stays provisional.
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
| Spending behavior, forecast adjustments and personal card/loyalty values | private `config/finance.json` | the human |
| Reviewed aggregate holdings | private configured snapshot | this capability, after explicit review |
| Holdings dashboard projection | Postgres | this capability, rebuildable from the private snapshot |
| Baseline, forecast, runway and decision results | API response only | this capability, rebuilt from reviewed private state |

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
- `POST /api/import/csv/preview` · `POST /api/import/csv` · `GET /api/import/candidates`
- `GET /api/import/investments/mappings` · `POST /api/import/investments/preview`
- `POST /api/import/investments/confirm`
- `POST /api/import/candidates/:id/review`
- `POST /api/import/candidates/reclassify-batch`
- `POST /api/import/candidates/:id/allocation`
- `POST /api/import/candidates/:id/reimbursement`
- `GET /api/ledger/check` · `POST /api/ledger/rebuild`
- `GET /api/dashboard?start=&end=&account=&category=&currency=`

The dashboard response includes source freshness and the planning report. Neither is
stored as a second source of truth.

## Configuration

Database from `$AXON_FINANCE_DATABASE_URL`, else the overlay's
`config/postgres.env`, else a localhost development fallback. Vault location from
the overlay's `config/finance.json`, or `AXON_FINANCE_OBSIDIAN_ROOT` for
development. `journal` and `budgets` live in that same private file; the journal can
also be set with `AXON_FINANCE_JOURNAL`. `schemas/finance.json.example` documents the
shape without carrying private deployment values. Named `csv_mappings` also live in
that private file; the loopback API supplies them to the local review UI, where the
operator can still edit every field before staging. A mapping explicitly declares
amount direction, accepted date formats, and whether every row must match the header
or rows without transaction fields may be counted and ignored. Preview returns only
quality counts and an identity token; staging recomputes the CSV and requires that
unchanged token. Stable-reference duplicates are counted within one export. When
the source has no reference, repeated normalized rows are preserved with
deterministic occurrence identities, so legitimate repetition and overlapping-export
idempotency both survive in the candidate store.
Named `investment_csv_mappings` supply the corresponding preview-only adapter. The
stable source key and source identifier to symbolic commodity mapping belong there
rather than in Axon. `investment_snapshot` names the private canonical collection
written after review. Reconfirming one source replaces only that source; Overview
derives its aggregate and review coverage from every confirmed source. A provider
without an export can use a privately authored current-position CSV with one dated
row per open position. Raw CSV rows and mapping values are never written to the
canonical collection. When one instrument spans sources, its activity price is
suppressed rather than selecting an arbitrary source. Other prices shown in Overview
are latest activity prices, not live quotes or a claim about current market value.

Optional `planning` configuration controls baseline length, cash-buffer target,
source-freshness thresholds, category behavior rules and dated adjustments. Private
source expectations can track each symbolic transaction account or reviewed holdings
source independently, so one recent import cannot hide another stale source. A rule
classifies what normally recurs; a trip purpose remains exceptional even when its
category is otherwise recurring. Historical categories replaced by commitment or
subscription series are removed once before those dated series are added, so they do
not count twice. Fixed costs without a replacement series stay in the forecast.

Card options carry fee, reward-rate, FX-cost and dated source evidence. When private
account prefixes are configured, eligible spend is derived from reviewed expense
postings for the trailing twelve months. Personal benefit values, FX spend, point
valuations and loyalty balances are never inferred. They remain private inputs, and
the response marks a decision provisional until usage and sources have been reviewed.

Transaction source accounts are balance accounts. A reviewed card purchase therefore
posts from a liability to an expense, while a settlement posts between balance accounts.
The latter projects as a transfer and is excluded from spending unless transfers are
explicitly requested.

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

**A trip is context, not a category branch.** A trip expense is also a food expense.
Duplicating the category tree under a trip prefix would make "what did I spend on
food this year" answerable only by remembering to union two subtrees. `hledger
balance tag:axon-trip-id=<plan-id>` is the join. Finance stores only the opaque Trips
plan identifier; the dashboard asks Trips for the current title. Purpose is a
separate `axon-purpose` tag, and ownership is represented by postings, so category,
reason and whose money was spent remain independently queryable.

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
