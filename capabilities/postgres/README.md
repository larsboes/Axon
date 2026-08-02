# postgres

Shared correlation data layer for the interlinked services — `scouting`, `transit`, `calendar`,
and whatever else needs cross-domain joins later. One Postgres instance, one database, each
capability owns its own schema (`scouting.*`, `transit.*`) inside it. Why that layer exists at
all is below; open work on it is in GitHub Issues.

## Verdict

**Adopt, official image, pinned.** Not built, not forked — Postgres is the whole point of this
capability, and the official `postgres` image (MIT-licensed Dockerfile generator, Apache-lineage
Postgres itself) is the standard, audited way to run it. Alpine base for the same reason
`vaultwarden` picked it: smaller image, smaller CVE surface than the Debian-based variant.

**Why one shared instance instead of a container per capability:** each ported service (`scouting`,
soon `transit`) individually needing a Postgres container is exactly the "way more machinery than
needed" pattern this repo avoids elsewhere — N containers, N watchdogs, N backup targets for what's
functionally one small personal database. One instance, schema-per-capability for ownership/migration
isolation, correlation queries live in a thin layer across schemas. The actual driving reason this
exists is cross-domain correlation, not "Postgres is nice to have" — see the next section.

**Why not one database per capability instead of one DB with multiple schemas:** cross-schema joins
within one database are a single connection, no `dblink`/foreign-data-wrapper machinery — schemas
get you migration/ownership isolation without giving up the actual correlation queries this whole
thing is for.

## Why this shape: cross-domain correlation

The differentiator over an existing travel-search engine, an existing event scout, or an ad-hoc
"ask an AI to search" is **persistent cross-domain correlation**: events, cheap travel, the
operator's own availability and (later) people all scored against one interest profile and
queryable *against each other*, with a memory of what has already been seen and judged. If this
system does not correlate across domains, there is no reason to have built it instead of a flight
search, an event site and a chat tab. That is what makes one shared schema-per-capability database
the right shape, and it is why `scouting` migrated back off SQLite onto this instance.

**What the phase labels in the source mean.** Several doc-comments across `scouting` and `transit`
name a phase; the numbering is scope, not status, and it is defined only here:

| Phase | Scope |
|---|---|
| 1 | Scouting memory: `status` (`new`/`dismissed`/`saved`) on `opportunities`, plus a `source_state` per-adapter cursor so re-runs skip already-judged items |
| 2 | This shared instance: scouting off SQLite, `transit` ported with its own `trips`/`trip_legs` tables, its fare search wired in as a scored scouting source |
| 3 | Fuzzy/triggered trip-search sessions (`transit plan`), built on Phase 2's schema |
| 4 | The `calendar` capability: availability windows, scoped rhythms, events, day views — and the feasibility verdicts that join it against `scouting.opportunities` |
| 5 | People as time-windowed entities, plus a suggestion engine over everything above |

**What the correlation layer has to answer.** These are the acceptance criteria the design is
judged against, not a roadmap:

1. A constant background scan of events and cheap travel, continuously scored against an interest
   profile, with dismissed items staying dismissed.
2. A triggered fuzzy trip search — "in September I feel like a trip" — as soft destination
   expansion plus date-window sampling into a persistent session.
3. Re-runs that skip already-judged items rather than rescoring the world.
4. Every scored opportunity carrying a feasibility verdict from the operator's calendar:
   `free` / `needs-travel-day` / `conflicts`.
5. Cost as a cross-cutting score dimension, so "cheap" means the same thing for a flight, an event
   and (later) a couch to sleep on.
6. People as time-windowed entities — a friend's *residence* window and *availability* window are
   structurally the same shape as a `calendar.entries` row, which is why Phase 4 came before
   Phase 5 rather than after it.
7. Trip overviews that assemble the above into what to do and who to invite.

**Where the join itself lives is still open**, and it is the first decision the correlation layer
forces: a SQL view across schemas, or a dedicated module. The calendar README's "Correlation
contract" section specifies the verdict protocol; the query shape and volume are now known, so the
call is no longer hypothetical. Whether a `correlations` table is worth having stays deferred until
Phase 4's query patterns have actually been exercised.

## Considered and declined

**Surfacing `transit`'s trip-search journeys back into `scouting.opportunities`.** It would make
trip results show up in the backlog alongside events, which sounds like a free win. It is not: it
needs a scouting-side adapter reading from transit sessions, which reverses the established
dependency direction (`scouting → transit`, never the reverse). The direction is a correlation-layer
design decision, and it belongs to whoever builds that layer — not a side effect of wanting one more
row type in the backlog view.

## Architecture

```
tools/service-runner.sh  ─┬─ reads capabilities/postgres/service.toml (image, tag, ports, volumes, env_file)
                          ├─ reads axon-overlay/config/machine.toml (os, container_runtime)
                          └─ dispatches to apple-container | docker | podman -- same shared
                             mechanism vaultwarden already runs through, zero new dispatch code
                             (except the managed-volume branch below, apple-container only)
```

**Data volume is `managed_volume = "true"` in `service.toml`, not a plain bind mount** — the
one real divergence from `vaultwarden`'s pattern, and apple-container-only. Its virtiofs bind
mounts don't support guest-side chown/chmod at all (confirmed empirically, not assumed — see
`schemas/service.toml.example`), and the official postgres image's entrypoint always chowns its data dir before
`initdb`. `tools/service-runner.sh` creates a real `container volume` (`axon-postgres-data`) and
mounts that instead, on apple-container only; Docker/Podman keep the plain bind mount, unaffected.
`PGDATA=/var/lib/postgresql/data/pgdata` in `postgres.env` is required alongside this — the
volume's filesystem root isn't empty (has a `lost+found`), and `initdb` refuses to use a
non-empty mount point directly. Same fix the pre-existing LifeOS `lifeos-postgres` container
already uses.

One database (`axon`), created via `POSTGRES_DB` at first boot. Each capability creates and uses
its own schema inside it (`CREATE SCHEMA IF NOT EXISTS scouting;` etc.) rather than a separate
database, so cross-schema correlation queries stay a single connection/transaction.

## Layout

| File | Purpose |
|---|---|
| `service.toml` | Declares the container: image, tag (pinned `17.10-alpine`), port (localhost-only — nothing but local capabilities should ever reach this), `managed_volume = "true"` |
| tracked config template | `capabilities/postgres/postgres.env.example` (`POSTGRES_DB`, `POSTGRES_USER`, `POSTGRES_PASSWORD`, `PGDATA`) |
| real config values | `axon-overlay/config/postgres.env` (`POSTGRES_DB`, `POSTGRES_USER`, `POSTGRES_PASSWORD`, `PGDATA`) |
| real data | apple-container-managed volume `axon-postgres-data` (not a host directory — see Architecture) |
| password pointer | `axon-overlay/secrets/postgres-password.md` (Vaultwarden reference, not the value) |

## Env contract

Keep reproducible defaults in `postgres.env.example` and set operational values (especially
`POSTGRES_PASSWORD`) in `<overlay>/config/postgres.env`. This lets the capability run from
template plus explicit private override without leaking secret material into the public repo.

## Commands

```bash
tools/service-runner.sh start postgres               # create-or-start
tools/service-runner.sh install-persistence postgres  # survive crash/reboot (skipped on docker/podman)
```

Consuming capabilities connect via a standard Postgres connection string built from
`axon-overlay/config/postgres.env`'s values, `postgresql://<user>:<password>@127.0.0.1:5432/axon`,
each setting its own schema (`search_path` or fully-qualified table names) — never the `public`
schema directly, so two capabilities never collide on table names.

## First-run setup

`POSTGRES_PASSWORD` doesn't exist yet — generate and store it yourself, interactively:

```bash
tools/setup-secret.sh postgres password POSTGRES_PASSWORD
```

Same pattern `vaultwarden`'s own `ADMIN_TOKEN` used by hand once, generalized into a reusable
tool once a second capability needed the same steps (see `tools/setup-secret.sh`'s header and
`README.md#secrets`). It prompts your vault master password directly (via `bw unlock` — never seen
by an agent), stores the value as a Vaultwarden item (`postgres-password`, folder `Axon`), syncs
the one required plaintext copy into `axon-overlay/config/postgres.env`, and writes the pointer
doc at `axon-overlay/secrets/postgres-password.md` — never printing the value or the vault
session key anywhere. Add `POSTGRES_DB=axon` and `POSTGRES_USER=axon` to that same env file if
they're not already there (the script only manages the one var you pass it). Then
`tools/service-runner.sh start postgres` and verify it's actually reachable (`pg_isready`, or a
real connection-string test) before running any migration against it.

## Gotchas

- **Port is `127.0.0.1`-only, not `0.0.0.0`** — unlike `vaultwarden`'s deliberate LAN exposure,
  nothing outside this machine has a reason to reach this database. Widen it only if a real
  cross-device need shows up, and treat that as a call worth a `## Why this shape:` block here,
  not a quiet port-mapping edit.
- **Backup is a logical dump, never a file copy.** `backup_pg_dumpall = "true"` makes
  `tools/backup.sh` run `pg_dumpall` *inside* the container and stage the result as
  `pg_dumpall.sql`. Raw-copy (`backup_paths`) is wrong here twice over: copying a live cluster
  mid-write risks a torn backup, and under apple-container the data dir is a managed volume with
  no host path to copy at all. Don't add `backup_paths` here as a workaround. `pg_dumpall` rather
  than `pg_dump` so roles and every database restore from one file. The tool fails loud on a
  stopped container or an empty dump instead of shipping a 0-byte tarball that reads as success.
- **`axon-overlay/data/postgres/data/` is dead — an artifact of the first (bind-mount) attempt,
  before the virtiofs chown problem was found.** The real data lives in the `axon-postgres-data`
  managed volume now. Safe to delete that directory; nothing reads it.
- **`POSTGRES_PASSWORD` generation/storage is a deliberately separate, user-run step** (not
  bundled into "just set this capability up," and not something an agent runs on general
  instruction) — see "First-run setup" above and `tools/setup-secret.sh`.
- Pin bumps: `17.10-alpine` was adopted on 2026-08-01 after the repository cooldown and an exact
  image scan. Check the official supported-tags manifest and image digest, wait out `axon.toml`'s
  cooldown unless fixing an active vulnerability, and rehearse backup/restore before the next
  supported 17.x patch.
