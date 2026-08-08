# finance

Subscriptions as contracts with append-only price and state history, and the
decision layer above the ledger.

## Why a capability

hledger owns the ledger and Postgres owns the index over it. This owns everything
above both: what a subscription has cost over its life, what a trip actually cost,
whether a position crossed its exit rule.

The split is not a compromise between build and adopt. A plaintext journal under
git satisfies the knowledge-boundary's V1 and an index rebuilt from it satisfies
V2, so choosing the storage format did the boundary work rather than a rule someone
has to remember. What gets built here is the layer nothing off the shelf does well.
Double-entry posting, lot accounting, CSV rule engines, price feeds and return math
are not built here at all: `hledger roi` already computes IRR and TWR, and
`pricehist` already emits `P` directives.

## What exists today

Phase 2 of the spec: subscriptions, and no ledger. That ordering is deliberate. The
vault notes already carry the frontmatter, so the whole loop can be proven end to
end before the first bank export is parsed.

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
- `POST /api/subscriptions/:id/price` · `POST /api/subscriptions/:id/state`
- `GET /api/import/obsidian/scan` · `POST /api/import/obsidian`
- `POST /api/writeback`

## Configuration

Database from `$AXON_FINANCE_DATABASE_URL`, else the overlay's
`config/postgres.env`, else a localhost development fallback. Vault location from
the overlay's `config/finance.json`, or `AXON_FINANCE_OBSIDIAN_ROOT` for
development. No path, institution or figure appears in this repository.

The journal root is deliberately not a config key yet. Phase 3 adds it, and
declaring a key before anything reads it is how a manifest starts lying.

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

## The ledger, when it lands

Phase 3 adds the journal underneath all of this. The account tree, the trip tag and
the conventions are already fixed and validated as
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

`hledger` is declared in `toolchain.toml` as optional, scoped to a capability
declaring `ledger` in its `service.toml`. Nothing declares it yet, so the checker
correctly reports it out of scope rather than missing.

## Related tools and why this is not them

| Tool | Good at | Relationship |
|---|---|---|
| [Actual Budget](https://actualbudget.org) | Envelope budgeting, fast local-first UI | Rejected as core. Its automatic German bank sync ran through GoCardless Bank Account Data, which stopped accepting new accounts in July 2025 |
| [Firefly III](https://firefly-iii.org) | A serious rule engine and a real REST API | PHP with its own database. A second store inside Axon, and a ledger that is not git-diffable, so agent writes stop being reviewable |
| [Ghostfolio](https://ghostfol.io) | Portfolio math, price feeds, allocation | A candidate for the investment half later. It owns valuation well and models a subscription's history not at all |
| [hledger](https://hledger.org) | Double-entry, commodities, `roi`, CSV rules | Adopted, in phase 3. This capability sits above it and reimplements none of it |
