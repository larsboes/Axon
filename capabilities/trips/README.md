# trips

Persistent user intent and itinerary state for Axon's cross-capability Trips workspace.
It owns neither transport search nor event discovery: the dashboard composes `transit`
and `scouting`, and reads bounded `calendar` entries as trip fixpoints. Trips stores only
the references the operator explicitly adds to the itinerary, with inert provider payloads.

## Contract

The public shape is [`schemas/trip-plan.schema.json`](../../schemas/trip-plan.schema.json).
`trips.plans` records general places rather than only stations, up to four destinations,
a date window, travelers and allowed transport modes. Its explicit stages can use different
dates, travelers, modes and booking states. `trips.plan_items` records selected transport
options, events, activities, places, stays, images, notes and option sets. The payload is JSON
text so provider-specific evidence can be preserved without making provider fields part of the
durable plan contract.

<!-- human-voice: ignore em_dash -->

That freedom cost a caller the ability to know what to send: any JSON was accepted, so a
guessed shape got a 201 and a row nobody could read back. Four item types now promise a shape,
are validated on write, and name the missing field on rejection:

- `transport` — `{mode, journey}`. One producer, one shape, and the item to write for "hold
  this connection in the plan".
- `option_set` — `{query, options, observed_at?}`. Every fare a search offered, including the
  ones not taken. It exists because an unchosen fare cannot be queried back later at
  yesterday's price, so an unrecorded option set is gone rather than merely unwritten.
- `booking` — `{provider, order_ref, …}`. What makes a stage's `booked` status mean something:
  the order reference, fare name, refundability and cancellation deadline of a purchase made
  elsewhere. (Declared 2026-08-11; this list previously stopped at two.)
- `stay` — `{check_in, check_out, latitude, longitude, …}`. Where you sleep, next to how you
  get there. Declared for its intended producer, accommodation search results entered through
  the agent surface (in-repo, the demo seeder is the one writer); coordinates are required
  because the place matching downstream runs on them, and the provider's URL, price and
  rating ride along unvalidated.

Every other type stays permissive on purpose. `event` alone is written by three producers with
three different shapes (a scouting opportunity, a whole search result, a calendar anchor), so
declaring one shape for it would reject two of them. A variant is declared where there is
exactly one shape to promise, and nowhere else.

The accommodation flow, for the agent surface: anchor the search on the stage's
**destination coordinate**, not the city name — the provider takes coordinates directly, and a
coordinate is what lets the result match places later. When the stage carries no coordinate
(imported and drafted plans often don't), resolve one from `GET /api/places` before falling
back to geocoding: a place already visited usually has it. Write the offers as one
`option_set` (query, anchor provenance, every offer with its coordinate and URL), then the
chosen candidate as a `stay`. First run 2026-08-12 against a real plan: the October Berlin
stage had no coordinate, the December Berlin place did, and the search rode that one.

The Travel workspace exposes plan editing for title, start, up to four destinations, dates,
interests, travelers and transport modes through the existing `PATCH /api/plans/:id` contract.
Deletion uses `DELETE /api/plans/:id` behind a two-step UI confirmation. Deleting an imported
Axon plan never deletes its source Obsidian note.

For an active plan, the dashboard queries Calendar with the plan's inclusive date window
converted to Calendar's exclusive `to` bound. Planned and committed events at a destination
are shown before fresh discovery results as fixpoints. Adding one to the itinerary remains
an explicit action and records `calendar:<entry-id>` as the external id; opening Travel does
not copy or mutate calendar data.

The Travel overview also consumes Scouting's `travel_candidate` route. A future event matches
an upcoming plan only when its start day is inside the plan window and its complete coordinate
is within 75 km of a complete destination coordinate (great-circle distance; never city-string
equality or a missing-coordinate `0,0` guess). Unmatched dated candidates are sent to Calendar's
batch verdict endpoint. `free` and `needs-travel-day` remain actionable with the cost named;
`conflicts` is displayed as a no and cannot seed a plan. A matched event is added through Trips'
idempotent plan-item API only after an explicit click. A viable unmatched event only pre-fills
the existing plan form; the operator still supplies/reviews the route before saving.

This join remains computed in the dashboard because that is already the documented composition
edge for Trips, Scouting, and Calendar. It persists no recommendation and reads no foreign store.

HTTP surface on the manifest-declared port:

- `GET /health`
- `GET|POST /api/plans`
- `GET /api/plans/:id`
- `PATCH /api/plans/:id`
- `DELETE /api/plans/:id`
- `POST /api/plans/:id/items`
- `DELETE /api/plans/:plan_id/items/:item_id`
- `GET /api/import/obsidian/scan`
- `POST /api/import/obsidian`
- `POST /api/import/obsidian/all`

Rows live in the shared SQLite file — `AXON_DB_PATH`, else
`$AXON_PERSONAL_ROOT/data/axon/axon.db` — under the table prefix `trips`, so the two tables
are `trips_plans` and `trips_plan_items` (libs/axon-store/README.md). No personal station,
destination or credential is tracked here.

Obsidian scanning is enabled by `$AXON_PERSONAL_ROOT/config/trips.json`, shaped like
[`schemas/trips.json.example`](../../schemas/trips.json.example). The scanner stays inside
that configured root and considers only Markdown notes with `category: trip`. Scanning is
read-only. Import is explicit, idempotent by vault-relative path and requires a chosen
origin instead of guessing one. This contract does not scan Comms keeper notes or Scouting
opportunities; those vault surfaces remain owned by their respective capabilities.

## A sentence to a draft

There is one way to start a trip today: a form needing an origin picked from
`transit.suggest`, destinations, dates and modes typed field by field. "Somewhere warm in
October, under 300 euro, by train" has no entry point at all.

```bash
trips draft-intent "Munich for a conference the 14th to the 16th of September 2026, by train"
```

It prints a `CreatePlan`-shaped body plus what it could not settle, and persists nothing.
Every destination comes back as a `place:<slug>` with null coordinates, bit-identical to
what the dashboard mints from typed text, so a station still has to be picked before
anything can be searched. The model emits no EVA, no price, no feasibility and no plan id;
if it returns them anyway they are dropped, because nothing reads them.

CLI only and no HTTP route, deliberately. The question is whether a small local model turns
a travel sentence into a valid form, and until that has started a real trip more than twice
it does not need a surface.

### What the model got wrong, and what catches it

Measured against the on-device rung on 2026-08-11, so these are observations rather than
worries:

- Asked for "somewhere warm in October" with no year, it answered **2023**, twice. A plan
  quietly created for a date three years past is worse than a blank field, so a date is
  kept only when it is well-formed and not in the past.
- It lists `dates` as unresolved almost every time, **including when it has just returned
  correct ones** — for the Munich sentence above it answered 2026-09-14/16 and called dates
  unresolved in the same object. Trusting that claim discarded dates the sentence plainly
  gave.

So neither the dates nor the self-report is trusted. The check decides and rewrites
`unresolved` to match, which is the whole shape of this: the model proposes words, a
deterministic path decides what survives. A prompt is not a validation layer.

## Why a capability

Trip planning is a bounded domain with its own persistent state. Putting plan items in
`transit` would make transport own events; putting them in `scouting` would make
discovery own journeys. The dashboard remains a shell: it coordinates calls and renders
the workspace, while this capability owns the data.

## Related tools and why Axon is not all of them

Axon is useful when a plan should remain local-first and connect personal context across
`trips`, `transit`, `scouting` and the vault. It should still point to a narrower or more
mature tool when that tool already owns the immediate job:

| Tool | Individually good at | Relationship to Axon |
|---|---|---|
| [TREK](https://github.com/liketrek/TREK) | Self-hosted, real-time group planning with invitations, reservations, shared costs, packing lists, documents and a PWA | A serious whole-product alternative for collaborative trips. Axon should exchange neutral exports later rather than recreate TREK's group surface |
| [TripIt](https://www.tripit.com/web/free) | Turning forwarded booking confirmations into one itinerary | The strongest reference for automatic intake. It remains a cloud handoff because using it sends travel and booking data to an external service |
| [Besser Bahn](https://github.com/chuk-development/Besser-Bahn) | Android-first live rail assistance, connection predictions, disruption alerts and split-ticket booking links | Better during a running rail journey; Axon keeps the result in the broader trip plan |
| [BetterBahn](https://betterbahn.de) | A focused, inspectable split-ticket workflow | Product and algorithm inspiration. Upstream currently provides local/self-hosted use, not an official hosted calculator |
| Plan Bahn (`troyriverabusiness/msg-code-create`, gone) | The earlier Vue, FastAPI and LangGraph take on agent-assisted rail planning | Lineage only, and no longer reachable in any form. Upstream was already unavailable at the 2026-07-29 re-check; the one vendored copy, which lived inside the Event Horizon repository, went with that repository on 2026-08-17. Named without a link because the URL resolves to nothing, and the dashboard does not present it as a handoff |

The dashboard renders the useful subset as contextual disclosures: planning shows TREK and
TripIt; connection search shows Besser Bahn and BetterBahn. MapLibre, OpenFreeMap and
Wikimedia are providers used by Axon, so their attribution stays next to the data they
render rather than being mislabeled as alternatives.

## Obsidian sync boundary

Obsidian integration belongs at this capability's contract, not in the Svelte dashboard
and not as direct database access from a vault script. The import slice scans and
previews existing trip notes, then materializes a selected note as a plan with its source
reference preserved.

### The export slice, shipped

This paragraph specified it a month before anything implemented it, and PRD Q47
(2026-08-27) then made it a requirement rather than a roadmap item: `trips_plan_items`
holds 21 rows that exist nowhere else, and a capability holding only-copy rows projects
them to files. Every plan is now one Markdown file under `Resources/Axon/Trips/`, in the
vault named by `<overlay>/config/trips.json`.

The four points above survive with one correction. Point 3 said "a marked Axon-owned
section", which is right for a note a human already writes and wrong here: no human note
exists per trip, so PRD Q31 (2026-08-23) ruled this pattern B — a whole generated file,
in the one vault folder a human never edits. So the file carries an
`<!-- axon:projection -->` header instead of region markers, and a file at that path
without the header is refused rather than overwritten, which is Q31's promotion path.
Points 1, 2 and 4 hold: one file per plan, `axon_trip_id`/`axon_schema`/`axon_revision` in
frontmatter, and every path and personal value in the overlay.

`axon_revision` is the plan's `updated_at` — the same token `expected_updated_at` uses,
rather than a second notion of "which revision is this" that nothing could check against.

The mechanism is shared, per Q49: `libs/markdown-root`'s `projection` module owns
placement, containment, the header and the do-not-write-identical-bytes rule.
`src/projection.rs` owns only what a plan looks like.

What it is for is reconstruction, not reading (Q46: B14's projections are safety copies,
not reading surfaces). Each item's payload is written out verbatim as JSON, because an
unchosen fare cannot be re-queried later at yesterday's price and prose about a booking
reference does not restore a booking.

Two triggers, and both are needed:

- The server re-exports after **any** successful non-GET request, as a layer rather than
  a line in nine handlers — a tenth mutation route added later would otherwise stop
  projecting silently. A failure is logged and the request still succeeds: refusing a plan
  write because the vault is unreachable trades a durable row for a missing file.
- `trips export-vault` (`--dry-run` to look first) is the copy a human can take when the
  server is down, which is exactly when it matters.

Both run the same function, so both also sweep: a projection whose plan was renamed,
deleted or never existed is removed, because a stale safety copy is the one somebody
would restore from. Archived plans **are** exported — `list_plans` hides them from the
dashboard, and letting that filter reach the safety copy would make archiving a silent
data loss.

Two-way synchronization comes only after that export shape has been used. It should accept
only explicit fields such as notes, places and links, validate them into `PlanItem` data,
and record a conflict instead of silently choosing between two changed revisions. Axon's
database remains authoritative for trip identity, stages and item IDs; the vault remains
the authoritative writing surface for human notes.

## Build

```bash
cargo test -p trips
cargo build --locked --release --bin trips-server
```
