# libs/axon-store

One home for **when a capability's schema migration runs**.

A shared library, not a capability: no domain of its own, no upstream verdict, no CLI
(README.md#three-architectural-nouns). Consumers compile it in with a `#[path]` include rather
than a cargo path dependency, for the same reason `libs/axon-config` does.

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

## What it does not fix

The other half of the same problem, and the larger half in wall-clock terms. A request that
opens a store costs about 55 ms; the migration was never more than 2-3 ms of it. The rest is a
fresh connection and its auth:

```
/feed?limit=1   0.053 - 0.089 s   <- opens a Store
/health         0.0005 s          <- does not
```

Connection reuse needs a pool, and a pool means adopting a crate no capability currently
resolves. That is an upstream verdict, not a refactor, and it is tracked separately.
