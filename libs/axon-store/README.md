# libs/axon-store

One home for **when a capability's schema migration runs**.

A shared library, not a capability: no domain of its own, no upstream verdict, no CLI
(README.md#three-architectural-nouns). Consumers declare an `axon-store` path
dependency in the workspace.

## Why it exists

Seven capabilities own a Postgres schema, and all seven wrote the same `Store::open`: connect,
then run the entire migration. `CREATE TABLE IF NOT EXISTS` and `ALTER TABLE ... ADD COLUMN IF
NOT EXISTS` are cheap when they are no-ops — about 0.02 ms each, measured — so this looked free
and stayed for a year.

It is not free, because of what the no-op still takes:

```
NOTICE:  column "next_attempt" of relation "content_digests" already exists, skipping
        mode         |          tbl
 AccessExclusiveLock | comms.content_digests
```

One `batch_execute` is one transaction, so a single opener holds the strongest lock Postgres has
on roughly fifteen tables at once, to commit. Two openers each hold what the other needs.
Postgres calls that a deadlock and kills a session.

`comms` opens a store in 43 HTTP handlers and five timers. The collision was structural, not
unlucky: it showed up as `digest drain: db error: ERROR: deadlock detected` the day a third
fifteen-minute timer joined two that were already colliding rarely enough to look like weather.

## What changed

The DDL did not move. What moved is how often it runs.

`migrate_once` runs a capability's migration the first time a process sees a given (database,
schema) and never again for that pair. The second and every later `Store::open` do no DDL at
all, which is why this removes the deadlock rather than scheduling it: there is no longer a
second session taking those locks to collide with.

The cross-process advisory lock stays. Once-per-*process* says nothing about two processes of
the same capability starting together — a CLI run while the server boots, a restart overlapping
the process it replaces — and that window is real, if short.

## What it deliberately did not change

The old shape was self-healing by accident. Any code path could open a store against an empty
database and it worked, and every test helper leans on exactly that: build a per-pid schema,
call `open`, get a migrated schema back. Taking migration out of `open` entirely would have made
every one of those helpers, across seven capabilities, inherit an assumption that someone else
migrated first.

So the first `open` still migrates. A failed migration is not recorded as done, so the next
`open` retries instead of handing out a half-built schema.

## The other half: one connection per open

The migration was never the expensive part. Every `Store::open` also opened a fresh Postgres
session, and that is where the time went. Measured against the live database, not estimated:

```
Client::connect      32-39 ms   (five consecutive runs)
pooled Store::open   0.20-0.30 ms
```

`pool_for` gives a process one pool per database URL, so opening a store is a checkout. No call
site changed: `open` still takes a URL and returns a `Store`, because a pool keyed by URL is the
shape 43 handlers were already asking for.

Two settings are deliberate rather than inherited. `min_idle(0)`, because r2d2 otherwise keeps
`max_size` connections warm — right for a server, wrong for the CLI half of these crates, where
`comms sweep` would open ten sessions to run one query. And a five-second checkout timeout
instead of r2d2's thirty, because with Postgres down, thirty seconds reads as a hang rather than
a failure and lets requests pile up behind it.

## The estimate that was wrong

The originating issue read `/feed` at ~76 ms against `/health` at ~2 ms and concluded that
roughly 50 ms per request was connection setup. The connect is ~32 ms, and the remaining gap is
not a fixed cost — it scales with the response:

| Endpoint | Payload | After pooling |
|---|---|---|
| `/feed?days=1` | 57 KB | ~29-56 ms |
| `/feed?days=7` | 107 KB | ~57 ms (median of 15; ~78 ms before) |
| `/feed?days=30` | 194 KB | ~97 ms |

So that endpoint improved by about 21 ms and is still dominated by per-item work and
serialization. The pool was the right fix for the connect and was never going to be the fix for
the other two thirds. Whatever is worth doing about those is a different piece of work, against
a measurement rather than an inference.
