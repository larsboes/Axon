<!-- human-voice: ignore em_dash -->
<!-- The em dashes here separate a term from its definition, the idiom every other
     capability README in this repo uses. -->

# punctuality

How reliable a German train actually is, measured rather than predicted.

This capability folds Deutsche Bahn's own published stop history into per-station
statistics and answers questions about them. It is deliberately the bottom rung of the
intelligence ladder (README.md#implementation-languages-and-intelligence): a lookup table over millions of observed stops
is not a placeholder for a model, it is the number a model has to beat. Without it,
"our prediction is good" is not a claim anyone can check.

## Use

```bash
punctuality ingest                          # 2025-12..latest, download + aggregate
punctuality ingest --from 2026-01 --to 2026-03
punctuality stations bonn                   # eva lookup by name
punctuality stats "Bonn Hbf" --type ICE     # per hour, weekday and weekend
punctuality stats 8000044 --min-n 100       # unpadded eva from HAFAS works too
```

`ingest` replaces the whole aggregate rather than merging into it. The rows are a
function of the ingested window, so merging a narrower run into a wider one would leave
rows from months no longer covered with nothing in the table to show it. Every run
records its window in `punctuality.ingest_runs`.

## Where the numbers come from

[piebro/deutsche-bahn-data](https://github.com/piebro/deutsche-bahn-data) —
`upstreams.toml` [deutsche-bahn-data], CC-BY-4.0, the data licensed by Deutsche Bahn.
A job calls DB's Station Data and Timetables APIs four times a day and republishes the
result as one parquet file per month. So this is DB's own data under DB's own licence,
not a scrape.

The live Timetables API was the obvious alternative and cannot do this job: it answers
"what is delayed right now" and has no memory. History is the entire point here.

Upstream states three things about the data that the ingest is built around.

Coverage widened on 2025-11-02: before it only the largest ~100 stations, after it all
~5300. `dataset::FIRST_FULL_COVERAGE_MONTH` therefore starts at 2025-12, since 2025-11
is a mixed month. Reaching further back would buy history at the price of making a
station's statistics depend on when it entered the dataset.

Collection is 98.92% complete, with 196 named missing hours. So an absent train never
means "did not run". It may mean nobody was collecting that hour.

Timestamps are already Europe/Berlin local time, with no conversion applied. The ingest
treats them as wall-clock and carries no timezone library; converting them again would
move the rush hour by an hour or two.

## How the aggregate is built

One histogram per cell, where a cell is (station, train type, hour of day, weekend).
Delays are small integers, so counting them per minute is a complete description of the
distribution: quantiles read off the cumulative counts, memory is bounded, and
histograms add, which is what lets ingest stream file by file and still produce one
exact result. Buckets span [-5, 120] minutes with an open-ended bucket at each end;
99.906% of observed delays land inside that range (measured on 2026-06 before the range
was chosen, not assumed). `Cell::quantile_is_saturated` reports when a quantile fell
into an edge bucket, so a bound never gets printed as if it were a value.

Why not an analytics engine: see § Why this shape: Rust over a second engine, below.

### The statistics, and why these

| Column | What |
|---|---|
| `p50`, `p90` | median and 90th percentile delay in minutes |
| `share_late_6` | share of stops at least 6 minutes late |
| `cancel_rate` | share of scheduled stops cancelled |
| `mean_delay` | kept, but read it next to `p50` |
| `n`, `canceled` | sample size, because a rate without one is decoration |

Six minutes is DB's own punctuality threshold, which makes `share_late_6` comparable to
the figures they publish about themselves.

`mean_delay` is reported but is not the headline, and the reason is visible in the data:
ICE stops at Bonn Hbf in 2026-06 have a mean of 17.6 minutes and a median of 7. Delay
distributions are heavily right-skewed, so a handful of hour-late trains drag the mean
somewhere no actual journey lives. An earlier prototype of this idea aggregated on
`avg_delay` alone; that is the failure mode this table exists to avoid.

**A cancelled stop is never counted as a punctual one.** It has no delay reading, so it
stays out of `n`, the mean and the late share, and lands in `canceled`/`cancel_rate`
instead. Folding it in as "0 minutes late" would let the worst possible outcome improve
every other number.

## Gotchas

EVA numbers are zero-padded to eight digits here (`08000044`). HAFAS, and therefore
`capabilities/transit`, returns them unpadded (`8000044`). Joining the two without
`store::normalize_eva` returns zero rows and looks exactly like "no data for that
station". It presented that way the first time.

The raw cache really is a cache. `<overlay>/data/punctuality/raw` holds the downloaded
parquet at roughly 600 MB per month. Every byte is re-downloadable, it sits outside the
backup set on purpose, and deleting it costs only the next ingest's download time.

A download failure costs the whole run, which is why `ensure_local` retries four times
with backoff. Bodies that size drop occasionally: the first full run died on month six
of seven after five were already parsed. Cached months are skipped, so a re-run after a
failure only re-fetches what is actually missing. Partial downloads land in `.part` and
are only renamed on success, so an interrupted transfer is never parsed as a whole file.

`ingest` holds the whole aggregate in memory before writing, around 400k cells at 512
bytes of histogram each. That is the design rather than an oversight: the alternative is
partial writes, and partial writes cannot produce exact quantiles.

## The contract

`punctuality-server` on `:8085` is how other capabilities read this. `capabilities/transit`
is the first consumer and reaches it over HTTP, never by linking this crate (README.md#schemas-and-dependency-direction).

| Endpoint | What |
|---|---|
| `GET /health` | up, plus the window the aggregate currently covers |
| `POST /lookup` | up to 200 stops in one call, `{eva, train_type, hour, weekend}` each |
| `GET /stations?q=` | eva lookup by name fragment |
| `GET /stations?eva=&train_type=` | every hour cell for one station |

`/health` reports coverage on purpose. A server that answers but has never ingested is
up and useless, and without the window "no data for that train" reads the same as "that
train is never late".

`/lookup` answers `null` for a stop it knows nothing about, and for any cell thinner
than 30 observations. It never falls back to a neighbouring hour or a station average:
both would answer a question nobody asked, and downstream a substituted number is
indistinguishable from a measured one.

### What transit does with it

`Journey.delay_risk_score` is filled with `share_late_6` at the journey's destination,
for the arriving train's type and arrival hour.

**That is not the probability the trip works out.** It says nothing about catching a
transfer, and a journey whose first leg is late enough to miss a connection arrives on a
different train than the one this describes. Transfer risk is a different quantity and
this data cannot produce it — which is exactly the gap `bahnvorhersage` fills with a
separate `verbindungsscore`.

Absence degrades and never fails: if this server is down, or has no cell, transit
returns the same journeys with `delay_risk_score: null`. Verified by stopping the
service and searching — HTTP 200, five journeys, every score null.

The rung after that is comparison, not replacement. `bahnvorhersage`
(gitlab.com/bahnvorhersage/bahnvorhersage, GPL-3) publishes a self-hostable predictor
that outputs P(all transfers caught) and P(arrival within 10 minutes). Adopting it as a
capability and scoring it against this baseline is what would justify a model at all —
in either direction.

## Why this shape: Rust over a second engine

Migrated from its dissolved `decisions/` entry on 2026-07-28: this governs one
thing, so it lives with that thing (README.md#decisions-live-with-their-owner).

## Decision

`capabilities/punctuality` aggregates ~120M published stop records with the `parquet`
crate (`upstreams.toml` [arrow-rs]) in a plain Rust binary, and writes the ~400k-row
result into the shared `capabilities/postgres`. No embedded analytics engine is added to
Axon, at any layer.

DuckDB was the obvious candidate and was evaluated seriously, against the live dataset,
before this was written. It stays a fine tool to reach for interactively — it is how the
numbers in this entry were independently checked — it is simply not a dependency the
repo has to carry.

## Why

**Both DuckDB shapes cost more than they buy here.**

`duckdb-rs` bundles a C++ embedded database. That is a large non-Rust dependency in the
Bazel graph, with its own build, to run one `GROUP BY`. The DuckDB CLI avoids the build
but moves the aggregation into a SQL file outside the type system, where the histogram
rule that a cancellation is not a punctual train — the one decision in this capability
most likely to be got wrong — would live as an untested `WHERE` clause.

**The workload does not actually need a query engine.** Delays are small integers, so a
fixed 128-bucket histogram per cell is a complete description of the distribution, and
histograms add. That gives exact quantiles, bounded memory, and file-by-file streaming
from about eighty lines of arithmetic that unit tests can pin down. Measured first, not
assumed: 99.906% of 2026-06's delays fall in [-5, 120] minutes, which is what makes the
window honest rather than convenient.

**It keeps one storage story.** scouting, transit and comms already keep their tables in
the shared Postgres. Adding an engine would mean another place data lives, another
backup question, another thing to explain.

## What this forecloses

Ad-hoc SQL over the raw stop records is no longer one command away — a new question
about the raw data means either a DuckDB session against the parquet cache (which is
what the cache is for) or new Rust. Accepted: the aggregate answers the questions
`capabilities/transit` actually asks, and those files stay on disk precisely so
exploration never has to go through this crate.

The raw records are also deliberately NOT loaded into Postgres. `capabilities/postgres`
is backed up with `pg_dumpall`, so anything in that database is in every backup forever;
120M rows of re-downloadable public data would bloat every backup to store what
upstream will hand back on request. The parquet cache is therefore a cache — outside the
backup set, safe to delete, rebuilt by re-running ingest.

## Considered and declined

<!-- human-voice: ignore bold_bullets -->
<!-- README.md#decisions-live-with-their-owner defines this section: the rejected option in bold, then why it
     lost. The bold is the index into the list, not ornament. -->

- **DuckDB via `duckdb-rs`** ([duckdb/duckdb-rs](https://github.com/duckdb/duckdb-rs),
  MIT) — bundled C++ in the Bazel graph for one aggregation. Declined on cost, not
  quality.
- **DuckDB CLI driven from a shell script** — puts the statistical rules that decide the
  answer in untested SQL, and adds a binary dependency only this capability needs.
- **`pg_parquet` in the Postgres container** — requires a custom image, which trades the
  pinned official `postgres` image for one Axon has to build and audit itself.
- **Loading raw stops into Postgres** — see above; the backup consequence decides it.

## Considered and declined

<!-- human-voice: ignore bold_bullets -->
<!-- README.md#decisions-live-with-their-owner shape: the rejected option in bold, then why it lost. -->

- **Calling the DB Timetables API directly** — real-time only, no history, and it needs
  an API key. It is what the upstream dataset is built from, so consuming the dataset
  gets the same data plus two years of memory.
- **Grouping by line number as well as train type** — `line_number` is null for every
  long-distance train (ICE/IC/EC), so it would add cells that are empty exactly where
  the interesting delays are.
- **Keeping raw stop records in Postgres** — see the decision entry; it would put 120M
  rows of re-downloadable public data into every `pg_dumpall`.
