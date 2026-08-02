# calendar

Axon's personal calendar layer: the source of truth for what time means on any
given day — availability windows for travel, on‑site work in Bonn, remote‑work
blocks, busy periods, events you're attending, rhythms that materialize those
blocks, and (later) day‑planning detail. Open by default: an empty day carries
no meaning, and absence is not a block.

## Status

**Phase B complete (core schema + usable month dashboard); Phase F's week and
day views built; Phase C's Feed verdict surface is live; Phase E's Google
account connection, reviewed draft import, and explicit export review are live.**
The Postgres tables, HTTP CRUD, rhythm materialization, and the dashboard
month/week/day workspace are built and tested. Feasibility verdicts and feasible
travel windows are computed and served over HTTP, with unit coverage in
`src/correlate.rs`. Feed's Entdecken view batches dated opportunities through
the verdict endpoint and renders the resulting evidence as a soft explanation;
it never hides a candidate for a calendar conflict. Handing a window to a fare
search is still a command the operator runs, not something the system does on
its own. Phase A's Luma import is live: `scout --promote-calendar` upserts saved
Luma events as `source = luma` entries, verified end to end against this service.

The Google sync (Phase E) is code‑complete, fixture‑tested and connected to the
configured account. The dashboard starts with a 30-day, read-only review: it
shows exact-title/time duplicate candidates, requires explicit selection, then
re-fetches each selection and rejects a changed Google revision before writing a
non-blocking Axon draft. The Calendar workspace also has a 90-day draft inbox:
adopting an entry picks its kind and raises it to `planned`, while removing one
deletes only Axon's copy. Every path that needs a token still fails loudly and
by name (which key, which file, which setup step) instead of returning an empty
success; the setup steps are in § Phases > E. No event clustering, no travel-day
view. This README carries the full build plan — it's the working document until
the capability is fully built out. Each phase below has a checklist; items
without a check are not started.

### Built

- [x] Postgres schema + store (entries, rhythms, bounded planning contexts, indexes, dedupe)
- [x] HTTP CRUD for entries, rhythms, and bounded planning contexts
- [x] Rhythm materialization (forward‑only, idempotent)
- [x] Dashboard month‑grid view — inspect/edit entries, paint timeframes, create rhythms
- [x] Dashboard week and day views — hour columns, all‑day band, create by clicking an hour
- [x] Feasibility verdicts for dated candidates, with the entries that drove them
- [x] Feasible travel windows, in the day shape `transit plan --dates` consumes
- [x] Google import as drafts, deduped on the Google event id, Axon‑wins on collision
- [x] Dashboard Google import review — bounded preview, duplicate warnings, explicit selection, revision-checked draft import
- [x] Google export gated on a per‑entry opt‑in ledger
- [x] Real offset handling both ways, DST boundaries included (`src/zone.rs`)
- [x] Commitment axis — `possible`/`planned`/`committed`, orthogonal to kind, with the three tiers rendered and clickable in the workspace
- [x] Trip drafts — events grouped by place and time proximity (`GET /api/trip-drafts`)
- [x] Time-bounded planning context — editable notices that affect Home ranking without blocking calendar time
- [x] Home composition — upcoming calendar commitments, open calendar choices, planning context, location clusters, and source overview
- [ ] A dashboard surface that shows a verdict (Entdecken badge — not started)
- [x] Materialising a reviewed trip draft into a `trips.plan` through the
      public Trips API, with an idempotence ledger and dashboard confirmation
- [ ] Dashboard travel‑day view (Phase F — not started)

### Dashboard surface

The calendar workspace lives at `/calendar` and is a first‑class primary nav entry
("Kalender", between Home and Feed). It provides:

- **Three views — Monat, Woche, Tag** — behind one segmented switcher. All three
  share a single anchor date, so switching keeps the same date in view; only the
  ‹ Heute › navigation moves it, by month, by week or by day. The service window
  is exactly what is on screen (`from` inclusive, `to` exclusive), so switching
  view reloads the range rather than over‑fetching a month.
- A **month grid**: compact, colored entry chips per day by kind (busy → red,
  work_onsite → blue, work_remote → teal, away → amber, event → purple,
  travel_ok → green); out‑of‑month days dimmed; today highlighted
- **Week and day views** sharing one time grid: 24 hour rows, one column per day,
  timed entries positioned and sized by wall‑clock time and split side by side
  where they overlap. All‑day entries pin to a band above the hours and span the
  days they cover. Rhythm‑derived blocks are hatched with a dashed edge —
  "what my rhythm says" reads differently from "a one‑off I painted". A line
  marks the current time on today's column; days outside the anchor month are
  dimmed. The view opens on the first entry of the range, or on the working
  morning when it is empty. An empty day stays empty — absence is not a block.
- **Entry creation/edit** — clicking an entry chip opens it in any view; clicking
  an empty hour opens the same form prefilled as a timed entry in that hour
  (ends stay exclusive: 09:00 opens 09:00–10:00), clicking the all‑day band
  prefills an all‑day entry for that day, and in the month grid dragging across
  days prefills the inclusive range. Kind picker, title, all‑day/timed toggle,
  inclusive calendar dates, location and notes are editable; deleting asks for
  confirmation, and rhythm-derived overrides explain that they detach from the
  rhythm. Clicking a date in the week header jumps to that day's view.
- **Planning context** — the info panel above the calendar shows only notices
  whose validity overlaps the visible range. Contexts have a type, title,
  explanation, and inclusive validity window; they can be created, edited, and
  deleted in place. They inform Home and ranking but never occupy time.
- **Rhythm creation** — kind select, title, weekday picker, start/end times,
  validity window; on save the server materializes future instances and the
  grid refreshes
- **Active rhythms list** — compact accordion below the grid showing each
  rhythm's kind dot, title, weekday schedule and time

Built in `dashboard/src/routes/calendar/+page.svelte` and
`dashboard/src/lib/calendar/` (MonthGrid, TimeGrid, EntryForm, RhythmForm,
types/constants). The date math and placement live in `types.ts` — `timedSpan`
and `allDaySpan` turn an entry into a half‑open interval against one day or one
window, and a single `packLanes` handles both the horizontal splitting of
overlapping timed blocks and the vertical stacking of the all‑day band.
The proxy auto‑discovers calendar's service via `tools/capability.sh registry`.
The manifest's API-only proxy mode keeps `/calendar` owned by this workspace and
forwards only `/calendar/api` to the service (restart the dev server after
enabling the capability).

## Data model

### `calendar.entries`

| Column | Type | Notes |
|---|---|---|
| `id` | TEXT PK | `cal:entry:…` |
| `kind` | TEXT | See `KNOWN_KINDS` in model.rs |
| `title` | TEXT | |
| `starts_at` | TEXT | Naive local instant: `YYYY-MM-DD` (all‑day) or `YYYY-MM-DDTHH:MM:SS` |
| `ends_at` | TEXT | Exclusive (all‑day: next day; timed: after start) |
| `all_day` | INTEGER | 0 or 1 |
| `location` | TEXT? | |
| `notes` | TEXT? | |
| `source` | TEXT | `manual` / `rhythm` / `feed` / `scouting` / `luma` / `google` |
| `external_id` | TEXT? | Dedupe key per source |
| `rhythm_id` | TEXT? → rhythms(id) ON DELETE SET NULL | |
| `payload` | TEXT (JSON) | Inert provider evidence |
| `created_at`, `updated_at` | TEXT | Epoch seconds |

Unique indexes: `(source, external_id)` partial where non‑null, `(rhythm_id,
starts_at)` partial where non‑null (materialization idempotency).

### `calendar.contexts`

| Column | Type | Notes |
|---|---|---|
| `id` | TEXT PK | `cal:context:…` |
| `kind` | TEXT | `uncertainty` / `transition` / `preference` / `planning_gap` / `note` |
| `title` | TEXT | Reader-facing summary |
| `details` | TEXT? | Why the context matters |
| `valid_from`, `valid_until` | TEXT | Inclusive `YYYY-MM-DD` window |
| `created_at`, `updated_at` | TEXT | Epoch seconds |

Contexts are queried by overlap with the visible range. They stay separate from
entries deliberately: an uncertain colloquium window or a move period should
shape recommendations without pretending every day is busy.

### `calendar.rhythms`

| Column | Type | Notes |
|---|---|---|
| `id` | TEXT PK | `cal:rhythm:…` |
| `kind` | TEXT | |
| `title` | TEXT | |
| `location` | TEXT? | |
| `byweekday` | TEXT | CSV: `"tu,we,th"` |
| `start_time`, `end_time` | TEXT? | `HH:MM`; both or neither (all‑day) |
| `valid_from`, `valid_until` | TEXT | `YYYY-MM-DD` |
| `active` | INTEGER | |
| `created_at`, `updated_at` | TEXT | Epoch seconds |

### `calendar.google_exports`

The per‑entry export opt‑in (Phase E). A row **is** the opt‑in — the table
starts empty, so nothing exports by default, and opting out deletes the row.

| Column | Type | Notes |
|---|---|---|
| `entry_id` | TEXT PK → entries(id) ON DELETE CASCADE | Deleting the entry withdraws the opt‑in |
| `google_calendar_id` | TEXT | Recorded per row, so a later config change cannot relocate an event already pushed |
| `google_event_id` | TEXT? | `NULL` until the first push; afterwards what makes the next push an update, not a duplicate |
| `pushed_at`, `created_at` | TEXT | Epoch seconds |

### Kinds (extensible, not a CHECK constraint)

`busy`, `work_onsite`, `work_remote`, `away`, `event`, `nightlife`, `travel_ok`,
`draft` are the well‑known kinds. Kinds are stored and validated as machine‑safe tokens
(`[a-z0-9_]+`, 1‑40 chars) — new kinds for day‑planning blocks (Phase F) land
without a migration. The correlation layer (Phase C) maps known kinds onto
feasibility verdicts; unknown kinds are treated as neutral.

`draft` is the one kind that carries state, not meaning: it is what an
imported Google event lands as until the operator adopts it. See § Drafts.

### Sources

`manual`, `rhythm`, `feed`, `scouting`, `luma`, `web`, `google`. Same free‑text approach.

### Commitment: how binding an entry is

> **Why a second axis and not a `draft` kind**

`kind` says *what* an entry is. `commitment` says *whether it is happening*:
`possible`, `planned`, `committed`. The two are orthogonal, because a holiday
can be an idea or a booked flight and an event can be a bookmark or a paid
ticket.

This used to be spelled `kind = "draft"`, and this section used to argue for
that. The argument had a stated cost — "a draft cannot simultaneously carry the
kind Google suggests", so an out‑of‑office could not be an `away` — and it only
ever covered the Google case. Everything scouting promoted landed as a plain
`event`, which `correlate` read as a hard block, so an Impact Lab in Atlanta
nobody was going to attend deleted two days from August.

What survives from the old argument is its third point, and it survives
unchanged: **the dedupe key outlives adoption.** `(source, external_id)` is
what the partial unique index enforces, and raising the commitment changes
neither field, so the next import still recognises the entry.

Three consequences worth knowing:

1. **Adoption is raising the commitment**, not re‑kinding. Correcting the kind
   Google guessed is a correction, not a takeover, and `google::decide` treats
   it as such.
2. **`possible` can never block a day.** `correlate::impact` is
   `min(kind_ceiling, commitment_ceiling)`, so the commitment caps what an
   entry is allowed to cost whatever its kind.
3. **The column is a closed set with a CHECK constraint**, unlike kinds. An
   unknown kind has a safe reading; an unknown commitment has none, because
   deciding how hard a day is blocked is the entire job of the field.

The migration defaults to `possible`, not `committed`: a bug in that direction
hands out a free day you can check, rather than a silently blocked one you
never see.

### What Phase E settled

Phase E was named as the forcing function for real offsets, and it forced this:
**the offset becomes real at the boundary, and the store stays naive.**

Google emits genuine RFC 3339 (`2026-08-14T18:00:00+02:00`). `src/zone.rs`
converts it, in both directions, applying the offset *in effect at that
instant* rather than a fixed one:

- **Import** — Google's offset‑bearing instant → naive wall time in the
  operator's `home_timezone`, which is what lands in `starts_at`/`ends_at`.
- **Export** — the stored naive wall time → RFC 3339 with the offset that wall
  time actually had, plus the zone name, which is what Google's `dateTime`
  wants.
- **Nothing is lost.** The original offset‑bearing strings and Google's own
  `timeZone` sit byte‑for‑byte in `payload`, so a read path that later needs
  true instants can recover them without a re‑import.

That is exactly the plan this section carried before Phase E existed — store the
original in `payload`, layer normalization where a read path needs it — and it
survived contact. The alternative, offsets in the columns, was not close: the
window query compares `starts_at`/`ends_at` lexicographically in SQL,
`correlate::instant_minutes` parses them as naive, and the dashboard's
`types.ts` does its own date math on them. An offset suffix breaks all three.
Making instants first‑class is a v2 with a `timezone` column and a migration,
not a sync feature.

**Home timezone is configuration with no default.** `home_timezone` in the
overlay's `calendar.json`; both sync runs refuse until it is set. A wrong zone
writes every imported event an hour or more off, silently and plausibly, which
is worse than a refusal — the same call `scouting` made for the Luma promotion.

**The two DST edges, named.** These are pinned by tests in `src/zone.rs` and
`src/google.rs`, not left to be discovered:

| | Spring forward (29 Mar 2026, 01:00 UTC) | Autumn back (25 Oct 2026, 01:00 UTC) |
|---|---|---|
| What the wall clock does | skips 02:00–03:00 | replays 02:00–03:00 |
| Importing an *instant* inside it | fine — 00:59Z → 01:59, 01:00Z → 03:00 | fine, but two instants an hour apart both read 02:30 |
| Importing an event *spanning* it | fine — one real hour reads as two | **refused, by name**: one real hour reads as zero, and a zero‑length entry is not storable |
| Exporting a wall time inside it | **refused, by name**: no clock in this zone ever showed it, so there is no offset to stamp | resolved to the *first* occurrence (summer offset) — a stated convention, because there is no fact of the matter |

Both refusals are loud and specific, never bottoming out in the generic
"ends_at must be after starts_at". They are the honest edge of a naive model,
and naming them is the price of keeping it.

**Rhythms** store start/end as wall `HH:MM` and materialize into naive local
times. A rhythm timed at `09:00` produces `2026-08-14T09:00:00` for every
instance — no DST skip, no ambiguity. V2 may introduce an explicit `timezone`
column.

**Ends are exclusive.** An all‑day entry spanning only 2026-08-14 stores
`starts_at = "2026-08-14"`, `ends_at = "2026-08-15"`. This makes window
overlap queries simple: `substr(starts_at,1,10) < $to AND substr(ends_at,1,10) >
$from`.

## Why this shape

### A calendar capability (not trips, not scouting)

Trip plans (`trips`) own itinerary state — transport, stays, activities —
after you decide to go somewhere. Availability is the *before* state: what
your schedule allows. Putting availability into `trips` would make trips own
the user's calendar, which is exactly the domain‑ownership argument the trips
README makes for why plan items don't live in `transit` or `scouting`.
`scouting` finds and scores opportunities; it doesn't own your time. A
calendar domain is a third axis, not a second store in an existing one.

### Kinds are data, not a constraint

The transit README documents a real bug where a CHECK constraint
(`trigger_reason`) needed a migration when a new valid value appeared. Kinds
are the same shape — extensible — and a CHECK constraint would create the same
migration tax on every new kind. Instead, the database stores TEXT, the Rust
type validates shape (token/empty/len), and a `KNOWN_KINDS` constant documents
what the hardcoded correlation knows about. A kind unknown to the correlation
layer is simply treated as neutral — it still stores, queries, and renders
fine. The constraint lives at the thin edge where a verdict is produced, not
at every insert.

### Materialized rhythms, forward‑only

Rhythms are materialized on create and update (and re‑materialized on
demand via `POST /api/rhythms/:id/materialize`). Materialized instances store
`rhythm_id`, so a reader can distinguish "this is what my rhythm says" from
"this is a one‑off I painted." Re‑materialization is **forward‑only**: it
affects dates ≥ today. Past entries stay as historical record — editing a
rhythm doesn't rewrite history. User overrides are respected because any
patch to a rhythm‑linked entry detaches it (`rhythm_id → NULL`), so the next
re‑materialize won't touch it.

### No hard filter on conflicts (soft verdicts)

Correlation annotates an opportunity with `free` / `needs-travel-day` /
`conflicts` — it never hides it. A good enough event is worth moving a remote
day for, and that decision belongs to the operator, not to an opaque filter.
The same rule shapes the windows endpoint: a window that costs a travel day is
still returned, with the days it would cost named on it.

### Calendar hands the days over; transit never asks for them

Constraining a fare search to feasible days could run either direction. It runs
this one: calendar computes the windows and publishes them, transit grew a
`plan --dates` flag that searches exactly the days it is given, and the caller
moves the list between them.

The other direction — transit calling calendar when it plans — would put a
second capability's availability rules inside a fare searcher, add a
`requires` edge to a capability transit works fine without, and make a fare
search fail or silently widen whenever the calendar service is down. Transit
already took its dates as flags; it now takes a better-chosen set of them and
still knows nothing about why those days and not others. Calendar owns
availability, transit owns fares, and the only thing crossing between them is a
list of dates.

Concretely, the hand‑off is a shell one‑liner rather than a code path:

```bash
transit plan --from <EVA> --destinations "München" --dates "$(
  curl -s '127.0.0.1:8087/api/windows?from=2026-08-01&to=2026-09-01&min_days=2' |
    jq -r '[.windows[].days[]] | join(",")'
)"
```

`--dates` is mutually exclusive with `--date-from`/`--date-to` (a silent
precedence rule between "all of September" and "these four days" would hide
which one ran), and `--max-queries` still caps the fan‑out — a month of feasible
days is thinned to the query budget the same way a window is sampled. The flag
lives in `capabilities/transit/src/main.rs`; nothing there names this
capability.

## HTTP surface

| Method | Path | Notes |
|---|---|---|
| GET | `/health` | |
| GET | `/api/entries?from=&to=&kind=` | Window query, optional CSV kind filter |
| GET | `/api/proposals?from=&to=` | External, non-Google `possible` entries waiting for a Calendar decision; manual soft blocks are excluded |
| POST | `/api/entries` | Create entry |
| GET | `/api/entries/:id` | |
| PUT | `/api/entries/external` | Idempotent provider contribution; requires `source` + `external_id` |
| PATCH | `/api/entries/:id` | Any patch to a rhythm‑linked entry detaches it |
| DELETE | `/api/entries/:id` | |
| GET | `/api/rhythms` | |
| POST | `/api/rhythms` | Creates + materializes future instances |
| GET | `/api/rhythms/:id` | |
| PATCH | `/api/rhythms/:id` | Re‑materializes future instances |
| DELETE | `/api/rhythms/:id`?delete_instances=true | |
| POST | `/api/rhythms/:id/materialize` | Idempotent re‑materialize |
| POST | `/api/verdicts` | Feasibility verdicts for a batch of dated candidates (see Correlation contract) |
| GET | `/api/windows?from=&to=&min_days=` | Runs of days travel is possible in; `from` inclusive, `to` exclusive |
| GET | `/api/trip-drafts?from=&to=&max_gap_days=` | Recomputed, place/time-clustered journey candidates; no write |
| POST | `/api/trip-drafts/materialize` | Explicitly create one Trips plan from named `entry_ids`; Calendar records an idempotence ledger only after Trips accepts it |
| POST | `/api/google/import` | Pull the configured Google calendar in as drafts. Body `{"dry_run": false}` |
| GET | `/api/google/drafts?from=&to=` | Google-source entries still at `possible`, the draft inbox's ownership contract |
| POST | `/api/google/import-preview` | Read-only, date-bounded candidate review. Body `{"from":"YYYY-MM-DD","to":"YYYY-MM-DD"}`; maximum 90 days |
| POST | `/api/google/import-selected` | Re-fetch and import only reviewed selections as drafts. Every selection carries its Google `updated` revision; a changed event returns 409 for re-review |
| POST | `/api/google/export` | Push the opted-in entries, and only those. Body `{"dry_run": false}` |
| GET | `/api/google/exports` | The export opt‑in ledger |
| PUT | `/api/entries/:id/google-export` | Opt one entry in. Body `{"google_calendar_id": …}` optional |
| DELETE | `/api/entries/:id/google-export` | Opt out. The Google event it already created is left alone |

Confirming a draft is `PATCH /api/entries/:id {"kind": "event"}` and dismissing
one is `DELETE /api/entries/:id` — no new verbs, because re‑kinding already was
one. Listing what is waiting is `GET /api/entries?from=&to=&kind=draft`.

Both sync routes answer **400 with a named error** when the home timezone, the
calendar id or the credential is missing — never 200 with an empty report. A
selected import answers **409** when Google changed an event after the review;
the user must inspect the current version before Axon writes it.

## Correlation contract

The correlation layer (effectively the first half of Phase 4 as `capabilities/postgres/README.md` defines it, which
originally described people/friends windows) is Phase C. Built in
`src/correlate.rs`: pure functions over a slice of entries, so the verdict rules
are unit‑testable without a database and the store stays the only thing that
queries.

**Verdicts.** A candidate is a date range plus the caller's own id — a
`scouting.opportunity` id from Feed's Entdecken view. Every entry overlapping
that range contributes an impact, and the verdict is the worst of them:

| Entry kind | Impact | Why |
|---|---|---|
| `away`, `event` | `conflicts` | You are elsewhere, or already committed to something concrete |
| `work_onsite`, `busy` | `needs-travel-day` | Movable — go remote, or drop the block. Still offered, with the cost named |
| `work_remote`, `travel_ok` | `free` | Location‑flexible by definition, or the explicit yes |
| Google-source `possible` entry | `free` | An unadopted import. Neutral as policy, not by accident — see § Drafts |
| anything else | `free` | A kind this layer never heard of is neutral, never a block |

No overlapping entry at all is `free`. The neutral‑unknown rule is the other
half of "kinds are data, not a constraint" below: a Phase F day‑planning kind
lands without a migration *and* without silently blocking travel on every day it
covers.

**The candidate's own entry never conflicts with it.** Once promotion has run,
the calendar holds an `event` entry for that opportunity. Correlating the
opportunity would otherwise make every saved opportunity conflict with itself,
so an entry whose `external_id` equals the candidate id is excluded and reported
as `already_in_calendar` instead. That is the free clause's "or only
`travel_ok`/`event`"; a *different* confirmed event on the same day is still a
conflict, because you cannot attend both.

The match is on `external_id` alone, so **the candidate id must be the key
promotion wrote**, not a differently-shaped id for the same thing. A promoter
that stores a bare provider event id has to ask about that same bare id; passing
the wrapped opportunity id instead reads as a different event and self-conflicts.

**A verdict carries its evidence.** Every overlapping entry comes back with its
id, kind, title, range and impact, strongest first — including the neutral ones,
which explain a day without driving it. That is what lets a UI say "you have
on‑site work that day" instead of showing a badge with no argument.

**Foreign timestamps are read as local wall time.** Providers emit
`2026-07-10T18:00:00.000Z` (Luma), `2026-10-23T00:00:00+00:00`
(euro‑hackathons) and naive `2026-08-01T08:00:00` (transit) for the same kind of
fact. The zone designator and any fractional seconds are dropped and the clock
face is taken as written — the single‑home‑timezone call the time model above
already makes. Nothing is converted *here*, deliberately: the callers of this
endpoint (Feed's Entdecken, `transit`) already hand in instants that were
produced in the home zone, so converting them again would move them. The path
that does convert is the Google import, which receives foreign offsets and runs
them through `src/zone.rs` — see § Time model > What Phase E settled. A
candidate with no end, or one whose provider repeated the same instant twice,
covers the whole start day: an event without an end is a day, not an instant.

**Feasible windows.** `GET /api/windows` groups a span into maximal runs of days
where travel is possible at all — every day whose verdict is not `conflicts`.
Each window carries its days, the ones inside it that would cost a travel day,
and the worst verdict in the run, so the soft cost stays visible instead of
being filtered out. `min_days` drops runs too short to be worth searching.

Phase C's third piece — clustering two or more events by place and time proximity — is
`correlate::cluster_trips`, served at `GET /api/trip-drafts?from&to`
(`max_gap_days` defaults to 5, `home` falls back to `home_city` in the config).
A draft is recomputed per request rather than stored, because it is a function
of entries that can all move. Turning one into a real `trips.plan` remains a
deliberate act: Calendar calls Trips' public HTTP API and records the result in
its own idempotence ledger; it never writes into Trips' store directly.

Verdicts are soft — see "No hard filter on conflicts" above.

## Google sync contract

Phase E, built in `src/google.rs` (every rule, pure and fixture‑tested),
`src/google_sync.rs` (credential, transport, the two runs) and `src/zone.rs`
(offsets). Google is a **contributing** calendar, never a second source of
truth.

### Import

`POST /api/google/import` pulls `events.list` for the configured calendar over
the window `[today − import_days_back, today + import_days_ahead)` and lands
each event as a draft.

- **`singleEvents=true`.** A recurring series is expanded into instances, each
  with its own stable id (`base…_20260819T063000Z`). That id is the
  `external_id`, so twelve instances are twelve entries and a re‑import updates
  those twelve rather than adding more. The series id is kept in `payload`.
- **Dedupe is the existing upsert.** `(source = "google", external_id)` through
  `PUT /api/entries/external`'s semantics and the partial unique index behind
  them. Running an import twice produces one entry per event; running it twice
  over an *unchanged* calendar writes nothing at all, because the mapping is
  byte‑stable and a no‑diff refresh is skipped.
- **All‑day events need no arithmetic.** Google's `end.date` is exclusive, which
  is this capability's convention too.
- **`payload` is the evidence, and it is inert.** Google's event verbatim: the
  original offset‑bearing instants, its `timeZone`, `iCalUID`, `htmlLink`,
  `transparency`, `eventType`, `recurringEventId`. Deliberately no "imported at"
  stamp — a timestamp would make every run a write.
- **No date is guessed.** An event this layer cannot read exactly — no end, both
  a `date` and a `dateTime`, an instant with no offset, a span across the autumn
  transition — is reported as skipped with its reason and left alone. The rest
  of the run still imports.

### Conflict policy: Axon wins

One rule decides it, and adoption is the signal (§ Drafts):

| Google says | Axon holds | What happens |
|---|---|---|
| an event | nothing | `create` — a new draft |
| an event | a `draft` | `refresh-draft`, or nothing if it did not move |
| an event | anything re‑kinded | **`keep-axon-version`** — no write; what Google now says is reported on the outcome |
| `cancelled` | a `draft` | `drop-draft` — the source withdrew something never adopted |
| `cancelled` | anything re‑kinded | **`keep-cancelled-axon-version`** — the entry stays, the operator is told |
| `cancelled` | nothing | `skip` |

A divergence is *reported*, never silently applied and never silently dropped.

### Export

Opt‑in per entry, and only per entry. `PUT /api/entries/:id/google-export`
writes the ledger row; `POST /api/google/export` pushes exactly the rows that
exist. There is no "export everything" path and no default‑on flag, because the
empty table is the default.

- **First push inserts, later pushes patch** — the ledger remembers the Google
  event id, so a second run updates instead of duplicating.
- **The ledger's calendar id wins over the current config**, so re‑pointing
  `google.calendar_id` cannot relocate an event that already lives somewhere.
- **Two entries may never be opted in**, refused at the boundary with a reason:
  one whose `source` is `google` (it would duplicate the event Google already
  owns, and the next import would pull the copy back as a third thing), and a
  materialized rhythm instance (push the rhythm's meaning as its own recurring
  event instead of shipping generated instances).
- **Opting out does not delete the Google event.** Deleting something off
  someone's calendar as the side effect of a toggle is not a call this
  capability makes.

### The credential

Three keys — `GOOGLE_CLIENT_ID`, `GOOGLE_CLIENT_SECRET`, `GOOGLE_REFRESH_TOKEN`
— in a plain `KEY=value` file in the private overlay, the same shape
`capabilities/comms` uses. Default `$AXON_PERSONAL_ROOT/config/calendar.env`,
overridable with `google.env_path`. Nothing is read from this repo, and no token
value is ever logged — not in an error, not in a report, not in a failed‑refresh
body (Google puts token material in some of those).

`calendar.events` is the scope; it covers reading and writing events, so one
grant serves both runs. `capabilities/comms/auth/get-refresh-token.ts` already
requests it, so pointing `google.env_path` at `comms.env` reuses that grant
instead of minting a second one.

**Absent or broken credentials fail loudly and by name**, never as a no‑op:
which key, which file, and what to do about it. An HTTP failure from Google
names the likely cause (401 → revoked, 403 → missing scope or unshared
calendar, 404 → wrong `calendar_id`, 429 → rate limit).

## Phases

### A — Luma live‑verify
- [x] Add an explicit “in Kalender übernehmen” action for dated Scouting
      opportunities in Feed > Entdecken; confirm in the calendar entry form and
      promote through an idempotent source/external-id upsert
- [x] Run the existing `capabilities/scouting` luma adapter against the calendars
      you actually track. Done 2026-07-30 — **the adapter did not work.** What
      broke and what fixed it is written up in `capabilities/scouting/README.md`
      § Verdict. One calendar is tracked so far (Claude Community Events);
      which others to track is still open, see below.
- [x] Decide per calendar: ICS feed (Luma offers per‑calendar ICS subscribe)
      vs. fixing the scrape adapter. **Not a mix, as it turned out — fix the
      scrape, for both surfaces.** Evidence below.
- [x] When a Luma event is saved in scouting, promote into a `calendar.entry`
      with `source = luma` and `event` kind — that completes the pipeline from
      discovery to availability annotation. `scout --promote-calendar`; see
      `capabilities/scouting/README.md` § Calendar promotion for the contract.

#### ICS vs. scrape: why scrape won

Luma offers both for a calendar and both work. They were compared on the one
axis this capability cares about — can an entry be created without guessing?

| | `GET /calendar/get-items` (JSON) | `GET /ics/get?entity=calendar&id=…` (ICS) |
|---|---|---|
| Dedupe key | `api_id` (`evt-…`), the id the adapter already mints | `UID` (`evt-…@events.lu.ma`) — recoverable, one parse further away |
| Schedule | UTC instant **plus** the event's IANA `timezone` | `DTSTART`/`DTEND` in UTC, no `TZID`, no `VTIMEZONE` |
| Location | structured `city`/`country`/`address` | one free‑text `LOCATION` line, `GEO` on most |
| Scope | future events only | the whole history — 290 VEVENTs for 62 future ones |
| Parser cost | the shape the adapter already deserializes | a new ICS parser (line unfolding, escaping, property params) |

The JSON endpoint returns the **same `entries[].event` object** as the discover
feed the adapter already read, so supporting it cost one URL and no new parsing.
ICS would have cost a parser and still delivered less: with no per‑event zone,
turning `DTSTART` into local wall time needs a geo→timezone lookup Axon does not
have — precisely the guessing this capability refuses. ICS stays the fallback
for a calendar `get-items` does not serve; none encountered yet.

#### Still open: which calendars to track

One is tracked (`claude-community` / `cal-TOpA5LAFfuDeFpu`, the public Claude
Community Events calendar — 62 future events, every one with usable schedule
metadata). Declare more as `luma-calendar` entries in `scouting.json`'s
`sources[]`; each needs the calendar's `cal-…` api id, since Luma publishes no
public slug lookup.

### B — Core capability + dashboard month-grid
- [x] Postgres schema + store (entries, rhythms, indexes, dedupe)
- [x] HTTP CRUD for entries and rhythms
- [x] Rhythm materialization (forward‑only, idempotent)
- [x] Dashboard month‑grid view — paint timeframes, create rhythms (see Dashboard surface above)
- [x] Entry edit/delete from the month grid (entry chip → edit dialog)
- [x] Drag‑to‑paint multi‑day blocks across the grid

### C — Correlation
- [x] Feasibility verdicts on dated candidates — kind→impact mapping, evidence,
      unknown kinds neutral, the candidate's own promoted entry excluded
- [x] Constrain transit fare search to feasible windows — calendar publishes the
      days, `transit plan --dates` searches exactly those (see the why‑block)
- [x] Verdict endpoint in the correlation layer — `POST /api/verdicts` (batch)
      and `GET /api/windows`
- [x] Entdecken renders the batch verdict as a soft badge with evidence; a
      conflict never hides the opportunity or blocks calendar promotion

### D — Combining + trip assembly
- [x] Geo/time clustering of saved events
- [x] Reviewed draft trip → `trips.plans` through Calendar's HTTP boundary
- [x] Materialised plan opens directly in the existing Travel workspace
- [ ] Dashboard Trips workspace renders combined trip

### E — Google Calendar sync (two‑way)
- [x] Import: new events tagged `source = google`, dedupe by external_id;
      imported events arrive as drafts you confirm/re‑kind/dismiss. See
      § Google sync contract and § Drafts.
- [x] Export: per‑event opt‑in → push to Google Calendar. The
      `calendar.google_exports` ledger is the opt‑in; nothing exports by default.
- [x] Conflict policy: Axon is the source of truth; sync resolves toward Axon.
      Implemented in `google::decide`, table in § Conflict policy.
- [x] Real offset handling, both directions, both DST edges — § Time model >
      What Phase E settled.
- [x] **Connect a real account.** OAuth refresh and a live, read-only preview
      against the configured account were verified on 2026-08-01. Import remains
      explicit; no bulk entries were written during that check.
- [x] Dashboard surface for a date-bounded Google import review: it explains
      existing entries and likely exact-title/time duplicates, then imports only
      explicit, revision-checked selections as drafts.
- [x] Dashboard draft inbox: 90-day Google-source/possible queue, with
      re-kind + planned adoption or Axon-only removal.
- [x] Per-entry Google export toggle plus a dry-run review and explicit push.
      The toggle writes only the opt-in ledger; the separate sync review is
      the only dashboard action that contacts Google.
- [ ] Watch/incremental sync (`syncToken`, push notifications). This build
      re‑lists the window every run, which is fine at one operator's volume and
      wrong at scale.

#### Connecting an account

The credential is a configured input, never something this capability acquires
on its own. In order:

1. **Reuse the grant you already have, if you have one.** `bw unlock`, then
   `bun capabilities/comms/auth/get-refresh-token.ts` already writes
   `GOOGLE_CLIENT_ID` / `GOOGLE_CLIENT_SECRET` / `GOOGLE_REFRESH_TOKEN` into
   `axon-overlay/config/comms.env` **with the `calendar.events` scope
   included**. If that file exists and its token was minted after that scope was
   added, skip to step 4 and point `google.env_path` at it. If the token
   predates the scope, re-run the script once (`prompt=consent` is set, so it
   returns a fresh refresh token) — that also keeps Gmail working.
2. Otherwise, in Google Cloud Console: create (or reuse) a project, enable the
   **Google Calendar API**, and create an **OAuth 2.0 Client ID** of type
   *Desktop app*. Add yourself as a test user on the consent screen so the grant
   does not expire in seven days.
3. Mint a refresh token with `scope=https://www.googleapis.com/auth/calendar.events`
   and `access_type=offline&prompt=consent`, and write the three keys into
   `axon-overlay/config/calendar.env` as `KEY=value` lines, mode `600`. The
   comms bootstrap is the reference implementation.
4. Create `axon-overlay/config/calendar.json` from
   `capabilities/calendar/calendar.config.example.json`, setting at minimum
   `home_timezone` and `google.calendar_id` (`primary`, or a secondary
   calendar's `…@group.calendar.google.com` id), plus `google.env_path` if you
   reused `comms.env`.
5. Restart `calendar-server` and dry-run the import:
   `curl -s -X POST 127.0.0.1:8087/api/google/import -H 'content-type: application/json' -d '{"dry_run":true}'`.
   It reports what it *would* do and writes nothing. A misconfiguration comes
   back as a 400 naming the key and the file.
6. Run it for real (`"dry_run": false`), then look at what landed:
   `curl -s '127.0.0.1:8087/api/entries?from=…&to=…&kind=draft'`. Confirm one
   with `PATCH /api/entries/:id {"kind":"event"}`, dismiss one with `DELETE`.
7. Only if you want the other direction: opt a single entry in with
   `PUT /api/entries/:id/google-export`, dry-run `POST /api/google/export`, then
   run it. Nothing else is ever pushed.

### F — Week and day views
- [x] Week view: seven day columns over 24 hour rows, timed entries at their
      wall‑clock position, all‑day entries pinned to a band at the top
- [x] Day view: the same time grid at one column, taller hours, entry location
      on the chip; rhythm‑derived blocks distinguishable from one‑offs
- [x] View switcher sharing one anchor date across month/week/day
- [x] Create by clicking an empty hour or the all‑day band; edit by clicking a chip
- [ ] Travel‑day view: transit legs + event + buffer times
- [ ] General day‑planning blocks (the stuff that would overload Google Calendar)
- [ ] Not synced to Google — these are Axon‑only detail

## Done looks like

1. ✅ The operator can open a month‑grid view, see which days are open (empty),
   with painted availability windows, rhythm‑generated blocks, and imported
   events.
2. Scouting opportunities are annotated with a feasibility verdict that
   explains (not hides) conflicts: "This event is on 14 Aug; you have on‑site
   work that day — you'd need to take it remote or take a day off."
3. Saved events cluster into trip suggestions: "Two events in Munich in one
   week — want a combined trip 12–16 Aug?"
4. ✅ Google Calendar imports start as a bounded, reviewed draft selection;
   exports remain opt-in per event. The real account connection was verified
   read-only on 2026-08-01.
5. The day view on travel days shows the full picture: departure, connections,
   arrival, event window, buffer, return.

## Config

`AXON_CALENDAR_DATABASE_URL` (env) → `axon-overlay/config/postgres.env` →
`host=127.0.0.1 port=5432 user=axon password=axon dbname=axon` (fallback)

`AXON_CALENDAR_PORT` or `AXON_PORT` → 8087 (default)

Phases A–D need nothing else: all personalization is the data in the calendar
tables themselves. Phase E added the first config file, because a Google sync
cannot work without two personal values and refuses to guess either.

`$AXON_CALENDAR_CONFIG` → `$AXON_PERSONAL_ROOT/config/calendar.json` →
`capabilities/calendar/calendar.config.json` (local, gitignored). Template:
`calendar.config.example.json`.

| Key | Default | Notes |
|---|---|---|
| `home_timezone` | **none** | The zone every stored wall time is in. Both sync runs refuse until it is set |
| `google.calendar_id` | **none** | `primary`, or a secondary calendar's `…@group.calendar.google.com` id |
| `google.env_path` | `<overlay>/config/calendar.env` | `KEY=value` file with the three `GOOGLE_*` keys. May point at `comms.env` |
| `google.import_days_back` / `_ahead` | 7 / 120 | Import window, relative to today |
| `google.max_events` | 1000 | Hard bound on one import's paging |

No secret value belongs in `calendar.json` — only the path to the file that
holds them.
