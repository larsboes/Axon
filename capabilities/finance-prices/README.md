# finance-prices

The nightly market-price fetch. One job, no port, no HTTP surface: it runs
`finance-cli prices fetch`, writes what each provider returned into `finance`'s
own tables, and exits.

## What it writes

- `finance_prices` — one row per instrument per observed day per source. An
  observation, never a correction: a re-fetch of a day already stored is refused
  by the UNIQUE tuple, so running this twice costs nothing.
- `finance_fx_rates` — one row per currency pair per published day, stored as the
  ECB publishes it (quote units per one base unit) and never inverted at write
  time.
- `finance_price_fetches` — one row per attempt, successful or not, with a status
  and a bounded reason. This is what `GET /finance/api/prices/status` reads, and
  it is why "why is this instrument stale" has an answer that is not somebody's
  memory.

## The providers

| name | what it gives | needs the network |
| --- | --- | --- |
| `broker` | replays the reviewed activity price already in the holdings projection | no |
| `yahoo` | daily adjusted closes per instrument, keyed on the `ticker` in the overlay | yes |
| `ecb` | daily euro reference rates, with Frankfurter as the documented fallback | yes |

`broker` is the floor: it always works, so every holding stays priced on a
machine with no internet. It writes one observation per run, which is also its
limit — a history built only from `broker` grows one point per night, and the
risk model needs 120 before it will say anything.

## Why this shape: a job manifest rather than a field on finance

`tools/check-service-tomls.sh` refuses `autostart` and `schedule` in one
manifest — a service and a periodic job are opposite claims about one process —
and `capabilities/finance/service.toml` declares `autostart`. So the schedule
cannot live there. `capabilities/feed-sweep/service.toml` is the precedent and
states the same argument for the same reason.

## Why no freshness contract yet

`freshness_advise_hours` and `freshness_stale_hours` are deliberately not
declared on `capabilities/finance/service.toml`. `tools/doctor.ts` answers a
contract whose producer has never run with a fault on the first run, which is
correct behaviour and is why nothing is declared until something has fed it.
`GET /finance/api/prices/status` reports the gap meanwhile, per instrument and
per provider, which is what a human actually reads.

## Failure is a row, never a stopped run

A per-instrument refusal — no configured ticker, a gate page, a body that is not
the contract — writes a `finance_price_fetches` row with `status = 'refused'` and
a named reason, and the run continues. The exit status is non-zero only when
every attempt refused or errored.

It is not keyed on rows written, and that is the difference between a job that
reports its own health and one that cries every night: every write here is
idempotent (`UNIQUE (instrument, observed_on, source)`), so a run that finds
nothing new is the normal outcome — `broker` re-reads the same reviewed date, a
market provider re-reads a weekend, an offline host writes nothing at all. A run
with no targets at all is a success too: nothing was asked of it, and the
staleness it leaves is visible on `GET /finance/api/prices/status`.

## Running it by hand

```
finance-cli prices fetch                    # every registered provider
finance-cli prices fetch --provider broker  # one
finance-cli prices fetch --dry-run          # print, write nothing
finance-cli prices status                   # what is fresh and what is not
```

## Verifying it without writing into anything real

`AXON_DB_PATH` isolates the database and **nothing else**. Two of this
capability's outputs are files whose location comes from configuration, so they
land in the real overlay and the real vault whatever the database path says:

| what | where it goes | how to redirect it |
| --- | --- | --- |
| the decision ledger's month files | `<overlay>/data/finance/decisions/` | `AXON_FINANCE_DECISIONS_ROOT` |
| the subscriptions projection | the configured vault | `AXON_FINANCE_OBSIDIAN_ROOT` |

`AXON_FINANCE_DECISIONS_ROOT` exists so the write can be redirected without
redirecting the config read: pointing `AXON_PERSONAL_ROOT` at a scratch directory
does both, so a verification run either writes into the owner's overlay or runs
against a configuration that is not theirs.

```
export AXON_DB_PATH=/tmp/scratch.db
export AXON_FINANCE_DECISIONS_ROOT=/tmp/scratch-exports
export AXON_FINANCE_OBSIDIAN_ROOT=/tmp/scratch-vault
```
