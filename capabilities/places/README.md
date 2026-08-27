# places

Canonical place registry, geocode cache, and the map layers above finance, trips,
transit and the companion register.

## Why a capability

A 2026-08-25 survey found four place shapes in the system, none shared:

- `trips_plans` serializes `PlaceRef` JSON with optional coordinates
  (`capabilities/trips/src/store.rs`).
- `scouting_opportunities` carries retrofitted `latitude`/`longitude` columns,
  filled on only a small fraction of rows
  (`capabilities/scouting/src/store.rs`).
- `punctuality_stations` maps 5,429 EVA codes to names with no coordinates
  (`capabilities/punctuality/src/store.rs`).
- Vault notes carry hand-written `coordinates:` frontmatter (a handful of place
  and person notes).

The map needs one registry and one geocoder, and four consumers exist on day one:
the spend layer (finance), the travel layer (trips, transit), the people layer
(the companion register from `PRD Axon.md` §8.2), and the dashboard `/map` route.
That names the concrete consumer the no-speculative-surfaces ruling requires
(`dashboard/README.md`, "capabilities expose HTTP").

Cross-capability reads are how the layers assemble: one shared database was chosen
*explicitly* to enable correlation joins (`capabilities/postgres/README.md`), and
PRD Q45 (2026-08-27) kept that property while replacing the schema per capability
with a table prefix per capability in one SQLite file
(`libs/axon-store/README.md`). places reads `finance_*`, `trips_*` and `transit_*`
read-only and owns writes only under its own `places` prefix.

The four tables live in the shared file — `AXON_DB_PATH`, else
`$AXON_PERSONAL_ROOT/data/axon/axon.db` — as `places_places`,
`places_geocode_cache`, `places_transaction_places` and `places_person_places`.

## Decisions, 2026-08-25

Recorded from the crystallize session that scoped this capability. Each was the
principal's call from measured options.

- **D1 — Spend granularity: venue where the data supports it, city elsewhere.**
  Only the raw American Express exports carry street addresses (Adresse/PLZ/Land
  on most rows; measured coverage in the overlay evidence note). The import
  pipeline discards those columns (`capabilities/finance/src/import.rs:27-60`),
  so the raw files in the overlay are the only surviving venue source. Amex rows
  get venue pins by structured geocoding. Every other transaction aggregates at
  city level, parsed from the free-text description. No fabricated precision: a
  city bubble never pretends to be a venue.
- **D2 — places is its own capability.** Chosen over per-capability columns and
  over hledger journal tags. The registry, the geocode cache, the link tables
  and the layer endpoints live here. The finance projection is disposable — it
  is rebuilt from the canonical hledger journal
  (`capabilities/finance/README.md`) — so finance location links key on the
  journal-stable transaction `source_id`, never on projection row identity, and
  survive rebuilds by construction.
- **D3 — Geocoding: external API with a permanent local cache.** Chosen over
  bundled local datasets. Provider is Nominatim's public API (see
  `upstreams.toml`), throttled to 1 request/s per its usage policy, every
  response cached in `places.geocode_cache` so a query leaves the host at most
  once. Constraint, C2-derived (PRD §6.1): **a geocode query carries place text
  or a bare coordinate pair only — never a person name, never an amount, never
  a date.** (The coordinate-pair form is the reverse lookup that names a
  coordinate-only register proposal; `geocode.rs` enforces both shapes by
  construction.)
- **D4 — The companion register now.** Implements the PRD §8.2 ruling
  (2026-08-23) as `places.person_places`: person · place · date range ·
  confidence · source. Machine proposes, the human confirms every row, and
  derivation never writes a confirmed row directly. The register is C2 — the
  most sensitive store in the system. The database file lives in the private
  overlay, the table is never seeded into `axon_demo`, and its rows never reach a
  cloud model raw.

## Tables (prefix `places`, owned here)

- `places_places` — `id, name, kind (venue|city|station|address|region), address, city,
  country_code, latitude, longitude, source, external_ref, created_at`.
  `external_ref` holds a stable foreign key such as an EVA code or an OSM id.
- `places_geocode_cache` — `query_hash, provider, query, response (TEXT holding
  JSON), place_id, status (hit|miss|error), fetched_at`. The cache is permanent. A
  repeat query is served from here and never leaves the host again. `response` was
  `jsonb`; SQLite has no JSON type, and nothing ever queried inside the column —
  every reader took `response::text` and parsed it in Rust — so the JSONB was
  buying validation rather than indexing, and the writer's own serializer now does
  that.
- `places_transaction_places` — `source_id, place_id, precision (venue|city),
  confidence_bp, source, created_at`. `source_id` is the finance journal's
  SHA-256 candidate fingerprint (`capabilities/finance/src/import.rs`), the one
  identity that survives projection rebuilds.
- `places_person_places` — the companion register. `id, person, place_id, date_start,
  date_end (null = current), confidence_bp, source, state
  (proposed|confirmed|dismissed), created_at, reviewed_at`. `person` is the
  vault note name under `Atlas/People/`.

## HTTP surface (port 8093)

`GET /routes` serves the manifest via `libs/route-manifest`, with the standard
coverage test. The layer endpoints return GeoJSON FeatureCollections so the
dashboard map consumes them without translation.

Unlike the sibling servers, places sends no permissive CORS headers and refuses
any request whose `Origin` is not one the dashboard itself is served from
(loopback or a tailnet name — the set `dashboard/vite.config.ts` `allowedHosts`
mirrors). The register behind this surface is C2 (D4), and the refusal is what
keeps a hostile page in the operator's browser from reading it or driving the
confirm route cross-site.

- `GET /health`, `GET /ready`, `GET /routes`
- `GET /api/places` — list/search the registry
- `POST /api/geocode` — cached forward geocode (free-text or structured address)
- `GET /api/layers/spend` — venue features (total, visits, average, category
  mix) plus city aggregates and a summary block (total spent, visit count,
  per-city ranking)
- `GET /api/layers/travel` — trip destinations with past/upcoming phase, transit
  legs as LineStrings between station coordinates, and city-presence evidence
  derived from spend
- `GET /api/layers/people` — confirmed, currently-valid register rows
- `GET /api/unplaced` — expense transactions with no place link, grouped by
  exact description, ranked by total spend, capped at 200 groups
- `POST /api/unplaced/assign` — link every unlinked transaction whose
  description matches exactly to one place, at `venue` or `city` precision; a
  city-kind place is linked at city precision whatever was requested (D1)
- `GET /api/people/proposals`, `POST /api/people/proposals/{id}/confirm`,
  `POST /api/people/proposals/{id}/dismiss` — the §8.2 review path

## Backfills (CLI, one-shot)

Backfills are subcommands on the places binary, not routes. Each is idempotent
and reports counts.

- `backfill amex` — parse the raw Amex exports in the overlay
  (`data/finance/import/raw/`), match rows to `finance.transaction_candidates`
  fingerprints, geocode the structured address, write venue links.
- `backfill cities` — parse city fragments from the remaining transaction
  descriptions, geocode city names, write city links.
- `backfill stations` — resolve coordinates for the EVA codes present in
  `transit.trips`/`transit.trip_legs` through transit's HAFAS suggest surface.
- `backfill travelers` — write `proposed` register rows from the travelers
  named on non-archived `trips.plans`, one per traveler × plan (PRD §8.2;
  proposals only, the human confirms every row per D4).
- `backfill vault` — import the coordinates of the exported vault place notes,
  and write `proposed` register rows from person-note `coordinates` frontmatter.

## Deliberately not built

- **No PostGIS.** The pinned official alpine image ships without it, and a
  personal transaction history does not need a spatial index. Distance math is
  haversine in code,
  as in `dashboard/src/lib/travel/travel-candidates.ts`.
- **No GPS trace.** Travel history is reconstructed from plans, legs and spend
  evidence. Nothing tracks the phone.
- **No photo storage.** PRD non-goal N4 is permanent: Axon indexes and links,
  never copies from Photos.app. A photo layer waits for an indexer and is
  recorded in `ISA.md` here.
