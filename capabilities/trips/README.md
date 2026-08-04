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
options, events, activities, places, stays, images and notes. The payload is JSON text so
provider-specific evidence can be preserved without making provider fields part of the
durable plan contract.

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

The process resolves the shared Postgres connection from
`$AXON_PERSONAL_ROOT/config/postgres.env`; `AXON_TRIPS_DATABASE_URL` is the explicit
development override. No personal station, destination or credential is tracked here.

Obsidian scanning is enabled by `$AXON_PERSONAL_ROOT/config/trips.json`, shaped like
[`schemas/trips.json.example`](../../schemas/trips.json.example). The scanner stays inside
that configured root and considers only Markdown notes with `category: trip`. Scanning is
read-only. Import is explicit, idempotent by vault-relative path and requires a chosen
origin instead of guessing one. This contract does not scan Comms keeper notes or Scouting
opportunities; those vault surfaces remain owned by their respective capabilities.

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
| [Plan Bahn](https://github.com/troyriverabusiness/msg-code-create) | The earlier Vue, FastAPI and LangGraph take on agent-assisted rail planning | Historical implementation lineage only. Its repository was unavailable during the 2026-07-29 re-check, so the dashboard does not present it as a working handoff |

The dashboard renders the useful subset as contextual disclosures: planning shows TREK and
TripIt; connection search shows Besser Bahn and BetterBahn. MapLibre, OpenFreeMap and
Wikimedia are providers used by Axon, so their attribution stays next to the data they
render rather than being mislabeled as alternatives.

## Obsidian sync boundary

Obsidian integration belongs at this capability's contract, not in the Svelte dashboard
and not as direct database access from a vault script. The current import slice scans and
previews existing trip notes, then materializes a selected note as a plan with its source
reference preserved. The next export slice should:

1. export each plan to one Markdown file in a configured personal-vault folder;
2. put `axon_trip_id`, schema version and Axon revision in frontmatter;
3. regenerate only a marked Axon-owned section and preserve notes outside that section;
4. keep the vault path and personal content in `axon-overlay`, never in this repository.

Two-way synchronization comes only after that export shape has been used. It should accept
only explicit fields such as notes, places and links, validate them into `PlanItem` data,
and record a conflict instead of silently choosing between two changed revisions. Axon's
database remains authoritative for trip identity, stages and item IDs; the vault remains
the authoritative writing surface for human notes.

## Build

```bash
cargo test --manifest-path capabilities/trips/Cargo.toml
bazel test //capabilities/trips:trips_test
bazel build //capabilities/trips:trips-server
```
