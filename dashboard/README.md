<!-- human-voice: ignore em_dash -->
<!-- The remaining em dashes separate an endpoint or a table label from its description,
     the definition-list idiom the rest of this repo's READMEs use. -->

# Axon (dashboard)

The spine's shell. It shows what this machine has enabled, what those capabilities are
saying, and it starts one when you open it. It owns no data: every capability keeps its
own, exposes its own HTTP surface, and this only reads.

## Stack

Svelte 5 (runes) + SvelteKit 2 + Vite, per
[`this file`](../this file).
Bun for packages. No CSS framework and no component library: Svelte scopes a component's
own styles at compile time, so what is worth sharing is a token layer plus four
primitives, and `src/app.css` is that whole design system. Icons are inline SVG in
`src/lib/Icon.svelte` rather than a dependency.

The travel workspace keeps MapLibre and its stylesheet in a separate async bundle. A map
loads only when it approaches the viewport or the reader explicitly selects **Karte
laden**; the trip list remains the complete fallback when map loading fails. OpenFreeMap
supplies the basemap. Destination images come from Wikimedia's free-license page-image
surface and stay validated inert data.

`adapter-static` with an SPA fallback. Nothing here is true at build time, so nothing is
prerendered; the build is a static bundle any server can hand out, which is what makes
the eventual home-server deployment a file copy rather than a second architecture.

```bash
tools/service-runner.sh start dashboard   # :47117, with hot reload, supervised
```

Directly, for a build or the type check:

```bash
cd dashboard
bun install
bun run dev      # or: bun run build / bun run check
```

To show live system metrics (temperature, power, memory) on **/systems**, macmon runs automatically
via a LaunchAgent (`com.axon.macmon.plist`) that starts `macmon serve --port 9911` at login.

macmon is proxied at `/macmon` by Vite's dev server (see `vite.config.ts`). No
Axon capability needed — it is an external tool, discovered by the proxy the same
way LifeOS Pulse is.

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

  Comms is the one authenticated proxy contract. Its mutating routes stay guarded by
  the capability's `api_secret_file`; Vite reads that private reference only in the
  server process, rejects cross-origin mutations, and injects the bearer token without
  exposing it to dashboard JavaScript. A token change therefore requires restarting
  both `comms` and `dashboard`. Direct clients and `axon-clip` authenticate themselves.
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
| LifeOS Pulse (external, not an Axon capability) | `31337` | `/lifeos-pulse/*` |

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

All access goes through `src/lib/api.ts` (`transit`, `trips`, `scouting`, `wikimedia`,
`axonStatus`, `comms`, `lifeosPulse`). Components must not call `fetch` directly: the
client is the one place that knows upstream shapes.

That includes error shapes. Every capability server answers a failure as
`{"error": "..."}`, so `request()` unwraps that field before throwing; without it the feed's
paste box showed a reader the raw JSON on a 404, and so would every other call site.

## Daily information surfaces

The main navigation separates stages of work rather than domains that never meet:

| Surface | Reader's job | Domain owner |
|---|---|---|
| `/feed` | One information workspace with an **Eingang** for incoming observations and **Entdecken** for active opportunity discovery | `comms` owns feed persistence; `scouting` owns opportunity search and ranking |
| `/feed/[id]` | Read one dynamic, provenance-aware entry with its summary, safe plain-text source body and TELOS relevance explanation | `comms`; the route stores nothing itself |
| `/travel` | Turn selected places, connections, events and activities into durable trip plans | `trips`, composed with `transit` and `scouting` |

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
an existing summary. Feed/Postgres is the canonical inbox because every incoming item does not
deserve a permanent knowledge note. A later
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

## Wired into Bazel — the trigger this section named has fired

This section used to argue the opposite, and it was right at the time: real wiring only
once a named consumer exists, because a `BUILD.bazel` that shells out to `bun run dev`
underneath is ceremony without the property Bazel is for. The condition it set — *"once a
deploy step actually consumes the build rather than the dev server"* — came true on
2026-07-31, so the call was reopened under README.md#argue-bazel-per-case rather than left to rot.

What changed is that capability-owned UIs are now served as build outputs over their own
HTTP surface, which forced `bun_vite_build` and `bun_deps` into existence
(`tools/bazel/bun/`). The custom rule work this section correctly priced is already paid
for, and `dist/` is what a server hands out — a served artifact should be reproducible
rather than whatever the last local `bun run build` happened to leave behind.

`bazel build //dashboard:bundle` produces it. Dependencies are installed at fetch time by
a repository rule, so the build action itself needs no network.

**The development path is untouched.** `vite dev` is still the hot-reload server, still
what `service.toml` supervises, and still how you work on this app. Bazel owns the built
bundle — what a machine with nobody editing on it serves.

One thing had to move for that to work: `vite.config.ts` exports a function instead of an
object, so `buildProxy()` runs only when a server is actually starting. It shells out to
`tools/capability.sh` and reads every manifest in the repo, which is fine for a dev server
and impossible inside a sandbox that has neither. Evaluating it at config load was the
concrete thing keeping this app out of the build graph.

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
(preserved in Git history), Bazel-wired properly this time (`rust_binary` target in each
capability's `BUILD.bazel`, crate universe repinned), not left as an untracked `cargo run`-only
binary the way the shelved attempt was.

**Why:** both Cargo.toml files previously carried an explicit comment declining an HTTP
server — transit's: *"No tokio/axum... out of scope here"*; scouting's: *"No HTTP server
binary. The original had one... fronting a dashboard that was never adopted into Axon... zero
consumers is exactly the 'way more machinery than needed' anti-pattern."* Both were correct
when written — there was no consumer. A 2026-07-10 session silently deleted both comments and
added the server binaries anyway, without a named consumer, without Bazel wiring, without a
decision record — reopening the call by fiat instead of by the trigger it was explicitly
waiting for. That work was shelved, not lost — the root `dashboard` is a real,
deliberately-scoped consumer now, which is the actual trigger the
original comments named. This decision is that reopening, done properly: named consumer,
Bazel-wired, recorded.

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
