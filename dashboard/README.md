<!-- human-voice: ignore em_dash -->
<!-- The remaining em dashes separate an endpoint or a table label from its description,
     the definition-list idiom the rest of this repo's READMEs use. -->

# Axon (dashboard)

The spine's shell. It shows what this machine has enabled, what those capabilities are
saying, and it starts one when you open it. It owns no data: every capability keeps its
own, exposes its own HTTP surface, and this only reads.

## Stack

Svelte 5 (runes) + SvelteKit 2 + Vite, ruled below under *Why this shape: Svelte 5 is the
single frontend standard*. Bun for packages. No CSS framework and no component library:
Svelte scopes a component's own styles at compile time, so what is worth sharing is a token
layer plus a handful of primitives, and `src/app.css` is that whole design system. It
declares a named type scale, a spacing rhythm, three breakpoints and one focus ring, and the
classes `.card` / `.card-interactive`, `.tag`, `.btn`, `.input` and `.table`. What a class
cannot express once is a component: `ListRow`, `RowMeta`, `StateLine`, `FactorBars`,
`PageTabs` and `PageHeader`. `src/lib/feed/FeedItemRow.svelte` is the feed triage row built
on them; Home's reading lane renders it today and the `/feed` list is meant to adopt the same
component rather than keep a second one. Icons are inline SVG in `src/lib/Icon.svelte`,
quarried from Lucide (ISC), rather than a dependency.

### The token layer

PRD Q87 (2026-09-05) grew it, and the measurement is why. `app.css` was 289 lines and about
forty tokens, while `src/` held 786 `font-size` declarations over 72 distinct values, 222
distinct `padding` values, 80 distinct `gap` values and 32 distinct `@media` conditions —
including near-duplicates at 620, 640, 650, 680, 700, 720 and 760 px. 62 custom properties
were referenced and 35 declared; `--radius-lg` was referenced at four call sites and declared
at none, so two Home cards rendered square. There was no `:focus-visible` rule anywhere and
`.input:focus` removed the ring outright.

Most of that spread is legitimate — a component declaring its own map height is the right use
of the mechanism — so the gate accepts a property declared in the component that uses it,
which is the difference between a gate and a nuisance (`tools/dashboard-tokens.test.ts`).
Contrast is the other gate. `--text-tertiary` carries the meta line on every decision row and
failed WCAG AA in both themes at 2.56:1 on a light card, 2.33:1 on the light page and 3.67:1
on a dark card; it is now `#6b6b76` light and `#8a8a94` dark, measured after at 5.26:1 card
and 4.79:1 page in light, 5.18:1 and 5.82:1 in dark (`tools/dashboard-contrast.test.ts`).

Three breakpoints are named in the file — phone 38 rem, tablet 48 rem, rail 64 rem — as
documented constants rather than custom properties, because no shipping browser resolves
`var()` inside a media *condition*. A property redeclared inside a media *block* does resolve,
which is how one `--header-h` covers a bar that is 3.5 rem at desktop and 3.25 rem on a phone.
The ladder's four band tones collapse thirteen bands, and **no band is identified by colour
alone**: the break above each band carries its name in words.

**Two debts, recorded rather than closed** (PRD B50). `--warning` still fails AA as a text
colour at 3.19:1 on white, in the fourteen files that pass was not allowed to touch;
`--warning-ink` at 5.02:1 exists and everything written for the refresh uses it, and the
contrast gate deliberately does not assert on `--warning`, so the gate is green while the debt
is real. And `PinnedLinks.svelte` still renders an all-caps eyebrow, the last
`text-transform: uppercase` label after the nav sections dropped theirs. Not verified at all:
no browser was driven at the accessibility assertions — the VoiceOver announcement of a focused
row, the sticky rail at both bar heights and the ten-rows-above-the-fold target are unverified.

### Home's decision ladder

A registry (PRD Q86, 2026-09-05): a kind is one file under `src/lib/home/kinds/` and its row
one file under `src/lib/home/rows/`, discovered by `import.meta.glob` and joined on the kind's
`view` string — a name rather than an import, so a kind stays readable by plain `bun test`
outside Vite. `src/lib/home/decisions.ts` is the contract and imports nothing at runtime. Ten
kinds ship; adding one no longer touches a discriminated union, a seven-slot fan-out, the
briefing, the open handler and a 210-line snippet.

Rank is `score(band, urgency) = band × BAND_STRIDE + min(MAX_URGENCY, max(0, urgency))`, with
`BAND_STRIDE = 1000` and `MAX_URGENCY = 999`. **The clamp is what makes the band decisive, not
the size of the stride** — once urgency cannot reach the stride, every band gap of one or more
holds, so `tools/dashboard-home-bands.test.ts` asserts the clamp for strides of 1, 10, 100 and
1000 rather than asserting the number. Bands are not unique: PRD §8.1 already puts two kinds on
640, so a band is a rank and not a slot. Order inside a band is urgency, then earliest
start-or-due, then stable id — except in the reading lane, whose start-or-due is a creation
date and which breaks ties **newest**-first, because unscored feed items all land on one
priority and the oldest unread article is not the one to show first. A band §8.1 does not name
is declared in the kind's own file as `BAND <band> EXTENDS PRD <section and row>`, carrying the
same number the kind sets, and a kind at an undeclared band fails the gate.

Each kind writes its own slice on arrival and the ladder is derived over that. One
`Promise.allSettled` over seven reads used to hold `loading` true until the slowest capability
settled, so the flagship page rendered its heading and nothing else. The cold-start path is
memoised rather than flagged: `createStarter` in `registry.ts` keeps a `Map` of in-flight start
promises, so a second kind awaits the same POST the first one issued. A kind naming a row
component that does not exist returns null and warns once, so Home degrades to a blank row
rather than a blank page and the build gate names the owner. **The empty-state rule is applied,
not reargued:** a page still answering renders the loading line, which is a state and not an
empty state, and there is no "nothing is waiting" block — §8.1 already rules that a dashboard
blank on a quiet day is working correctly.

A capability-supplied destination on a scheme outside `https?|obsidian` is refused with a
stated error instead of being handed to `location.assign`, which closes the path from a hostile
feed link to script in the origin that renders the mail snippets. The keyboard handler is bound
only while the ladder is on screen, so Enter on another view cannot navigate to a row nobody
can see.

**Data class is shown only where a capability publishes one.** Mail publishes one, and
`GET /comms/feed` began publishing one on 2026-09-06 (`capabilities/comms/README.md`); the
calendar entry, the task, the trip plan and the scouting opportunity still do not, and this
shell will not invent a class it does not own. `FeedEntry.data_class` in `src/lib/api.ts` is
therefore typed **optional**, and the `?` records a contract gap rather than caution: comms'
detail contract carries no class, so `toListEntry` in `routes/feed/+page.svelte` builds a list
row out of an ingest response that has none. A reader must treat `undefined` as *not stated*
and fail closed. The field stops being optional the day the detail contract states one too.

### One map surface, and a basemap that is half local

Every map in this shell is `src/lib/map/MapSurface.svelte` over `src/lib/map/surface.ts`.
The component owns the frame and the deferred / loading / failed states; the module owns the
MapLibre instance, keeps it in a small pool, and hands it out on lease. A view brings its own
sources and layer specs and nothing else — `/map`'s spend, travel and people layers are
written in `routes/map/+page.svelte`, `/travel`'s three trip layers in
`src/lib/travel/trip-layers.ts`. What a view stops owning is WebGL.

The pool is what makes a route change cheap. Leaving `/map` for `/travel` detaches the map's
container and parks the instance rather than calling `remove()`, so the next view re-parents
it and swaps data: the parsed library, the parsed style, the decoded sprite and the uploaded
tiles all survive. It is a pool and not a singleton because `/travel` can legitimately show
two maps at once.

MapLibre and its stylesheet stay in a separate async bundle — `vite.config.ts`'s `bundleGuard`
fails the build if either reaches the eager import graph, **and if MapLibre's worker asset is
not emitted**. That second assertion exists because its absence is silent: MapLibre asks for its
worker through a template literal Rollup cannot follow, so no asset was built, the request fell
through axon-status' SPA fallback as `200 text/html`, `new Worker` was handed the app shell and
died — and every map on the served bundle rendered a blank canvas and sat on "Loading map…"
forever, with no error and no failed request. `surface.ts` hands MapLibre a Vite-built worker
through `setWorkerUrl` instead. A map loads when it approaches the
viewport, or immediately where the map *is* the page (`eager`), or on the reader's explicit
**Load map**; the list beside it is the complete fallback when loading fails.

OpenFreeMap supplies the basemap, and its fixed half is vendored under `static/basemap` by
`tools/fetch-basemap` — the style, the sprite and the Latin glyph ranges, with provenance and
the licence obligations in that directory's `LICENSE.md`. That removes four serialized
transatlantic round trips from the critical path (measured 2026-09-06: 0.23 s style, 0.19 s
TileJSON, 0.31 s sprite, ~0.2 s per glyph range) and replaces them with ~1 ms loopback reads.
The **vector tiles and the Natural Earth raster stay remote**; they are the large half, and
self-hosting them is an open decision in `capabilities/places/ISA.md`.

Labels are Latin-script only. Liberty renders `name:latin` concatenated with `name:nonlatin`, so
a European overview asked for Greek, Cyrillic, Arabic, Devanagari and more — measured at **29
glyph requests across 19 ranges** on the default view. `name:latin` already carries the romanised
form of every place, so dropping the second line closes the question instead of vendoring 4.5 MB:
four Latin ranges are vendored and **no glyph request leaves the machine**. A range outside the
set still falls back upstream, so getting it wrong costs a slow label, not a missing one.

Destination images come from Wikimedia's free-license page-image surface and stay validated
inert data.

`adapter-static` with an SPA fallback. Nothing here is true at build time, so nothing is
prerendered; the build is a static bundle any server can hand out, which is what makes
the eventual home-server deployment a file copy rather than a second architecture.

```bash
tools/service-runner.sh start dashboard   # :47117, with hot reload, supervised
```

Directly, for a build or the type check:

```bash
cd dashboard
bun install --frozen-lockfile --ignore-scripts
bun run dev      # or: bun run build / bun run check
```

Live system metrics (temperature, power, memory) on **/systems** come from macmon, which is
`capabilities/macmon` — enabled and started the ordinary way:

```bash
tools/capability.sh enable macmon
tools/service-runner.sh start macmon
tools/service-runner.sh install-persistence macmon   # if it should survive a reboot
```

Nothing about it is special-cased here any more. Its `port` feeds both the process and the
`/macmon` proxy entry, which the registry loop in `vite.config.ts` generates the same way it
does for every other capability. It was a hand-written LaunchAgent and a hardcoded proxy
target until then; both are gone.

## What the shell discovers, and what it is told

Nothing here lists capabilities. `tools/capability.sh registry` renders the `service.toml`
manifests as JSON, and two consumers read it:

- `vite.config.ts` builds the dev proxy from it. Every capability is reachable at
  `/<name>` with the prefix stripped; a surface whose paths predate that rule (transit's
  `/api`, scouting's `/discover`) declares them as `proxy_extra` and they pass through
  unstripped. A capability whose name is also a dashboard page declares
  `proxy_api_only = "true"`; `/calendar` therefore remains the workspace while
  `/calendar/api` reaches Calendar. Changing the enabled set or a port means restarting
  this dev server, since the registry is read once at startup.

  Comms is the one authenticated proxy contract. Every one of its routes except
  `/health` and `/ready` is guarded by the capability's `api_secret_file` — reads
  included, since it moved onto `libs/axon-server`'s shared inbound gate. Vite reads
  that private reference only in the server process, rejects cross-origin mutations,
  and injects the bearer token on every proxied request without exposing it to
  dashboard JavaScript. A token change therefore requires restarting both `comms` and
  `dashboard`. Direct clients and `axon-clip` authenticate themselves.

  The other capabilities are proxied unauthenticated, which is correct only while they
  are loopback-only. A deployment that declares `AXON_INBOUND_TOKEN_FILE` gates them
  too, and this proxy does not yet inject that token — the wiring belongs with
  `tailscale serve`, which is what makes the token necessary in the first place.
- `axon-status` serves the same registry plus live health at
  `/axon-status/api/axon-status/capabilities`, which is what the nav, the home page and
  the Capabilities page render.

A capability that declares `panel_port` appears on `/projects`. It does not grow the main
navigation or render inside the shell. Panel sites stay discoverable without running until
the user starts one, then open as separate pages.

| Capability | Port | Reached at |
|---|---|---|
| transit | `3000` | `/transit/*`, plus `/api/*` unstripped |
| scouting | `8084` | `/scouting/*`, plus `/discover` unstripped |
| axon-status | `8082` | `/axon-status/*` |
| comms | `8083` | `/comms/*` |
| punctuality | `8085` | `/punctuality/*` (no UI consumer yet — transit reads it server-side) |
| trips | `8086` | `/trips/*` |
| server | `4243` | interactive deployment plan, opened from `/projects` when enabled |

The authoritative list is `tools/capability.sh registry` (which is where the dev server
builds its proxy table from at startup); this table is the snapshot of the proxy quirks,
and it already went stale once — trust the registry when the two disagree.

### Project pages are addressed from the browser, never from the server

`panelUrl()` in `src/lib/api.ts` composes a panel's address from `location.hostname` and
the manifest's `panel_port`. axon-status deliberately does not send an absolute URL.

Same-host addressing also survives the shell being reached over Tailscale, where
`127.0.0.1` would point at the phone. The dashboard deliberately links to the project
instead of embedding it, so each SvelteKit site owns its own navigation and storage.

## Client

One client module per domain, and no component calls `fetch`. `src/lib/api.ts` holds the
shared clients (`transit`, `trips`, `scouting`, `wikimedia`, `axonStatus`, `comms`); a domain
that needs calls of its own puts them in `src/lib/<domain>/api.ts` — `src/lib/feed/api.ts` is
the first. The rule that matters is unchanged: a component never knows an upstream shape, and
a client module is the only place that does.

The split is a merge rule, not a taste. `src/lib/api.ts` was 3,530 lines on 2026-09-06 and is
appended to by several concurrent branches, so a domain adding two calls adds a file instead
of a hunk in the middle of everyone else's. `src/lib/travel/api.ts` is the second such module.

Error shaping stays central wherever the module lives. A domain module imports `ApiError` and
`describeFailure` from `$lib/api` rather than re-deriving them, so a reader gets the same
sentence whichever module made the call. Every capability server answers a failure as
`{"error": "..."}`, so `request()` unwraps that field before throwing; without it the feed's
paste box showed a reader the raw JSON on a 404, and so would every other call site.

What does **not** move with a domain is the contract: `request`, `jsonInit` and `ApiError`
stay in `src/lib/api.ts` and are imported, never copied — copying `request` would copy the
200-with-`{"error": …}` unwrap above, which is the half of this rule that has a bug behind it.

### Two list cursors, and the consolidation they owe

Two modules implement J/K/Enter over a list, and both ship:

| Module | Reader | What it owns |
|---|---|---|
| `src/lib/feed/list-cursor.ts` | the `/feed` inbox | pure arithmetic over `header`/`item` rows; a plain module with no runes, so bare `bun test` can exercise it (`tools/dashboard-list-cursor.test.ts`) |
| `src/lib/list-cursor.svelte.ts` | Home's ladder | the same keystrokes plus **real DOM focus** — `elFor(index)?.focus()`, so a screen reader is told the selection moved, which a `.selected` class never does |

They landed on two branches for two pages and this pass merged the branches, not the
modules. The constraint that keeps a naive merge from working is recorded so the next
attempt starts from it: `bunfig.toml` declares no Svelte plugin for the test runner, so a
`.svelte.ts` importing `$state` fails at module evaluation inside a bare `bun test`. So the
consolidation owed is the arithmetic moving down into a rune-free module that the focus-aware
one wraps — one behaviour, one test surface — rather than either page adopting the other's
file wholesale. Until that happens, a keyboard fix has to be made twice, and this paragraph
is the only thing that says so.

## Daily information surfaces

The main navigation separates stages of work rather than domains that never meet:

| Surface | Reader's job | Domain owner |
|---|---|---|
| `/` | Answer the decisions that are waiting, ranked by PRD §8.1's bands | every capability that ships a kind; the shell only ranks |
| `/feed` | One information workspace with an **Eingang** for incoming observations and **Entdecken** for active opportunity discovery | `comms` owns feed persistence; `scouting` owns opportunity search and ranking |
| `/feed/[id]` | Read one dynamic, provenance-aware entry with its summary, safe plain-text source body and TELOS relevance explanation | `comms`; the route stores nothing itself |
| `/travel` | Turn selected places, connections, events and activities into durable trip plans | `trips`, composed with `transit` and `scouting`; the Companions rail is read-only and links to `/map` to decide |
| `/finance` | Overview, Planning, Transactions, Investments, Subscriptions. Investments opens on the decision inbox, then the position table | `finance`; the money on this page is decisions and never a ledger view |

`nav.ts` is the authoritative list and holds more entries than this table; these are the
surfaces whose reader's job needs a sentence.


Feed is not a Scouting inbox. Scouting is one possible specialist path for a feed item, and a
Scouting result may appear in Feed as a typed observation. The UI should connect those cases
with contextual actions such as “evaluate as opportunity” or “add to trip”, while leaving
security notices, release notes and ordinary reading items in Feed.

The UI integrates both jobs under `/feed`, but it does not collapse their data models. The
**Eingang** view starts `comms`; **Entdecken** starts `scouting` only when opened. They call
their capability APIs independently, and there is not yet a feed-to-scout handoff or shared
read model. The old `/scout` path redirects to `/feed?view=discover` for saved links. When a
real handoff is added, it carries typed IDs and provenance through the HTTP contracts rather
than joining capability tables in Svelte.

The feed can be ordered by recency or by its revisioned evaluation. **Mit TELOS abgleichen**
starts an explicit server-side refresh, but unchanged rows are skipped. The UI names how many
rows were considered, evaluated and already current. A compact factor chart shows the
deterministic 0–100 rank as interests, freshness and content basis; the raw TELOS match remains
available and stays labelled `semantic` or `lexical`. A shared model-status strip reports the
configured local summarizer, whether its endpoint is reachable, the active relevance fallback
and ledger counts without exposing endpoint credentials.
**Vault-Links** scans only privately configured exact notes or headings, shows the candidates,
and fetches one only after an import click.
**Quellen** lists the general Comms collectors and can scan GitHub Trending, arXiv or all
enabled sources. It shows last-run state and bounded limits; the result separates new from
already-known targets and tells the reader that enrichment continues behind the response.

The reader is one dynamic `/feed/[id]` route, not a generated Svelte page per source item. Its
wide layout keeps the readable document column separate from provenance and TELOS context.
Summary and source text stay inert: an allow-listed renderer handles headings, paragraphs,
lists, links, code and simple tables without `{@html}`, while embedded HTML and images are
discarded. A real summary is shown as the note; the UI does not substitute an empty placeholder
when summarization is unavailable. Long media transcripts remain explicitly expandable beneath
an existing summary. The feed's own tables are the canonical inbox because every incoming item
does not deserve a permanent knowledge note. A later
**In Vault behalten** action may create a typed Atlas or Media Markdown note explicitly; until
that contract exists, **Behalten** only persists feed status.

The evaluation model also consumes a bounded, cached Trips snapshot. A fourth **Reisebezug**
factor names the matching plan and matched destination or interest terms. `/feed/library`
turns those structured references into a compact trip timeline with counts, average trip-fit,
top entries and a per-trip filter. The timeline is not a second Trips database; every label
links back to `/travel`.

`/feed` is intentionally the unresolved-new queue: choosing **Behalten** or **Verwerfen** removes
the item from that work surface without deleting it. `/feed/library` is the durable collection
view over up to ten years of Feed state, including dismissed entries. It groups by the strongest
stored TELOS lens and falls back to source type where no lens exists; search, type, status, lens
and ordering remain independent controls. Smart ordering uses the persisted factorized
evaluation, and every tile exposes its component bars rather than only a single score. Its
counts and source distribution are computed from stored rows and are descriptive collection
facts, not model-generated success metrics.

The Feed's **Entdecken** view is a persistent triage surface, not a one-shot search form. It
gets enabled source IDs from `GET /scouting/sources`, shows the existing ranked backlog from
`GET /scouting/opportunities`, triggers a selected-source scan through `/discover`, and
persists `new`/`saved`/`dismissed` decisions through the Scouting API. Personal sources such
as `scholarship-radar` therefore appear from overlay configuration; they are never duplicated
in a Svelte constant. Fixture-only built-in adapters stay out of the picker until they have
live verification.

Obsidian keeps capability ownership intact. The Trips page imports `category: trip` notes,
Scouting reads only configured opportunity/profile globs, and the Comms CLI can export a
distilled keeper. These are separate capability contracts, not one dashboard-level vault scan.

## A built bundle is what a served machine hands out

This section used to argue that the dashboard needed no build step at all, and it was right
at the time: the condition it set — *"once a deploy step actually consumes the build rather
than the dev server"* — came true on 2026-07-31, when capability-owned UIs started being
served as build outputs over their own HTTP surface. `dist/` is what a server hands out, and a
served artifact should be reproducible rather than whatever the last local build happened to
leave behind.

`bun run build` produces it (`svelte-kit sync`, `svelte-check`, then `vite build`).
`tools/service-runner.sh` runs that command in the package's own directory before it starts the
service, so a machine that nobody edits on serves a bundle built from the checkout it is running.

**The development path is untouched.** `vite dev` is still the hot-reload server, still what
`service.toml` supervises, and still how you work on this app.

One thing had to move for that to work: `vite.config.ts` exports a function instead of an
object, so `buildProxy()` runs only when a server is actually starting. It shells out to
`tools/capability.sh` and reads every manifest in the repo, which is fine for a dev server and
wrong for a build that produces static files. Evaluating it at config load made every build
depend on the whole manifest tree.

Between 2026-07-31 and 2026-08-25 the bundle was a Bazel target with a hand-rolled bun
toolchain; PRD Q44 retired both. The trigger and the artifact survived the build tool.

## Trips and connection search

`/travel` is the persistent planning workspace. The dashboard composes the `trips`,
`transit`, and `scouting` HTTP contracts with validated image and map data; it does not
move either capability's domain state into the shell. A plan keeps the travel intent and
saved itinerary items, while current connection and event searches can be refreshed.
Plans are place-to-place rather than station-to-station: each stage records its own date,
travelers, allowed transport modes and booking state. Rail search is the first live
transport provider; other selected modes remain explicit pending their own providers.
The map and plan list select each other, and connection options can be sorted and expanded
to their individual legs before being saved. Plans whose end date has passed move into the
history view without re-querying time-bound external results.

Nearby Wikipedia places provide evidence-linked activity and highlight candidates with
images; saving one persists the inert data and image URL in Trips. The Obsidian scan button
calls Trips' configured vault importer, previews only `category: trip` notes, and imports
one only after the user supplies a missing origin.

`/travel/connections` is the focused one-off connection and split-ticket search. It was
rewritten, not translated: the earlier React version was Tailwind classes over a
`lucide-react` import, and this shell has neither.

Two things the live API taught that page. A regional leg comes back with
`total_price: null`, so a missing fare renders as `—` and never as `0,00 €`. And a route
with no cheaper combination answers `404` with `{"error": "..."}`, which the page catches
into the split tab as an outcome while still rendering the direct connections that did
arrive.

That 404 used to be a 500 carrying a bare sentence rather than the `{"error": "..."}`
every capability is supposed to answer with, so the reader got raw text on a perfectly
normal "this route has no bargain". Fixed on the transit side, where it belonged.

## Why this shape: capabilities expose HTTP, the shell only mounts

Migrated from its dissolved `decisions/` entry on 2026-07-28: this governs one
thing, so it lives with that thing (README.md#decisions-live-with-their-owner).

**Decision:** `capabilities/transit` and `capabilities/scouting` each grow a second binary —
`transit-server` / `scout-server` (Rust, Axum, per README.md#implementation-languages-and-intelligence) — alongside their existing CLI
binary (`transit`, `scout`). Migrated back in one capability at a time from a shelved bulk port
(preserved in Git history), declared properly this time — a real `[[bin]]` in each capability's
`Cargo.toml`, named by its `service.toml` — not left as an untracked `cargo run`-only binary the
way the shelved attempt was.

**Why:** both Cargo.toml files previously carried an explicit comment declining an HTTP
server — transit's: *"No tokio/axum... out of scope here"*; scouting's: *"No HTTP server
binary. The original had one... fronting a dashboard that was never adopted into Axon... zero
consumers is exactly the 'way more machinery than needed' anti-pattern."* Both were correct
when written — there was no consumer. A 2026-07-10 session silently deleted both comments and
added the server binaries anyway, without a named consumer, without a declared build target,
without a decision record — reopening the call by fiat instead of by the trigger it was
explicitly waiting for. That work was shelved, not lost — the root `dashboard` is a real,
deliberately-scoped consumer now, which is the actual trigger the
original comments named. This decision is that reopening, done properly: named consumer,
declared binary, recorded.

**Forecloses:** the "no server, zero consumers" comment doesn't get silently deleted again for
the next capability that wants one — cite this decision instead, or write a new one if the
trigger differs. A capability HTTP surface is added *only* when a concrete consumer is named
(here: the root `dashboard`), never speculatively "because the pattern exists." `pulse`'s
HTTP surface is a separate decision (name/port collision with the real LifeOS Pulse dashboard —
resolved by `capabilities/axon-status/README.md`).

## Why this shape: Svelte 5 is the single frontend standard

Migrated from its dissolved `decisions/` entry on 2026-07-28: this governs one
thing, so it lives with that thing (README.md#decisions-live-with-their-owner).

# Decision: Svelte is Axon's frontend standard

**Status:** accepted · **Date:** 2026-07-28 · **Domain:** every Axon web surface

## Decision

Axon standardizes on **Svelte 5 in runes mode and SvelteKit** for dashboards, capability
panels, project sites and future web surfaces.

- Svelte components own presentation and local interaction state.
- SvelteKit owns routing, prerendering and deployment adapters.
- Static projects use `@sveltejs/adapter-static`.
- The home-server deployment may use `adapter-node` when server rendering or a colocated web
  process becomes necessary.
- Tauri may package the same web application when a measured desktop or mobile-native
  requirement appears. It is not a second UI architecture.
- WASM remains a measured optimization for compute-heavy parsing, simulation or visualization,
  never a default UI layer.

Tried first: React, selected 2026-07-16 on the assumption that Axon would assemble arbitrary
AI-generated React components. That assumption is what this decision removes; the separate
entry recording it was folded in here 2026-07-28.

## Boundary: Svelte is the renderer, not the Axon core

Axon's durable contracts must not depend on `.svelte` files:

- capability manifests and HTTP/event contracts;
- typed data and action schemas;
- evidence records and provenance;
- provider-neutral Pack outputs;
- allow-listed `VisualSpec` and Reader Lens plans.

Agents and model providers produce inert data, actions or validated layout specifications.
They do not generate arbitrary framework component source at runtime. Codex, Claude Code,
local models and later providers therefore share one contract even though Axon's maintained
renderer is Svelte.

This removes the former React decision's load-bearing assumption: Axon no longer optimizes for
assembling arbitrary AI-generated React components. It optimizes for a small, inspectable UI
surface over provider-independent contracts.

## Why this fits Axon

Axon is primarily a solo-maintained, self-hosted system with custom live feeds,
evidence-oriented dashboards and a phone-accessible web surface. Its recurring work is local
state, filters, WebSockets, progressive disclosure and purpose-built visualization. Svelte's
compiled reactivity and component-local HTML/CSS fit that profile more directly than a hooks
and memoization model.

React still has a broader off-the-shelf ecosystem. In particular, vis.gl provides first-party
React bindings. That advantage is real but bounded:

- deck.gl core and MapLibre remain framework-independent;
- Axon accepts a small maintained Svelte lifecycle wrapper for map/WebGL surfaces;
- a wrapper becomes shared infrastructure only after two real consumers need it;
- a missing integration is evaluated against a concrete feature, not treated as a standing
  reason to keep every surface in React.

Headless Svelte primitives such as Bits UI are candidates for accessible dialogs, menus and
comboboxes. They enter only when a real repeated primitive needs them.

## Evidence and migration learnings

Two measurements informed the decision. They answer different questions and must not be
collapsed into one headline percentage.

### Controlled interaction spike

The same small stateful interaction was built with React 19.2.8/Vite 7.3.6 and Svelte
5.56.8/Vite 7.3.6:

| Implementation | JavaScript raw | JavaScript gzip | UI lines including CSS |
|---|---:|---:|---:|
| React | 193.92 kB | 60.93 kB | 75 |
| Svelte | 32.59 kB | 12.89 kB | 68 |

The minimal Svelte output was about 79% smaller gzip, while source lines fell only about 9%.
The meaningful win was runtime weight and a simpler state model, not a claim that Svelte
automatically removes a third of all application code.

## Migration rule

Existing React surfaces are migration inputs, not exceptions to the standard:

1. the dashboard shell and its live capability widgets establish the realtime pattern;
2. remaining panels and prototypes are ported or deleted according to current value.

During the transition, existing React code may receive only fixes required to preserve a
working migration source. New features and shared UI primitives are implemented in Svelte.
React dependencies leave each workspace as soon as its Svelte replacement reaches functional,
accessibility and build parity.

## Forecloses

- no new React, Next.js, JSX or TSX surface;
- no permanent React/Svelte split design system;
- no provider-specific generated component code as a Pack contract;
- no native rewrite merely to add Tauri;
- no Kubernetes, WASM or desktop wrapper before a measured requirement justifies it.

## Considered and declined

<!-- human-voice: ignore bold_bullets -->
<!-- README.md#decisions-live-with-their-owner defines this section: one bolded thing evaluated, then why it lost.
     The bold IS the index, so the linter's "convert some to prose" would remove the
     structure the rule asks for. -->

- **A CV tab** — the first attempt iframed a CV dev server that does not exist;
  `capabilities/cv` is CLI-only. Revisit once it grows a real server.
- **transit-server proxying the other capabilities** — cross-capability aggregation
  belongs to this shell's proxy config, not baked into one capability's server.
- **Proxying panels through this origin** — a dev server emitting absolute asset paths
  (`/@vite`, `/_app`) breaks behind a stripped prefix. Panels load from their own port on
  the same host instead, which is same-site and therefore unpartitioned.
