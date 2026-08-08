export class ApiError extends Error {
  status: number;
  constructor(status: number, message: string) {
    super(message);
    this.name = 'ApiError';
    this.status = status;
  }
}

/**
 * The message inside an error response body. Every capability server answers
 * `{"error": "..."}`, so without this the UI shows a reader the raw JSON —
 * which is what the feed's paste box did on a 404 until this was fixed.
 */
function errorMessage(body: string): string {
  try {
    const parsed = JSON.parse(body);
    if (parsed && typeof parsed === 'object' && typeof parsed.error === 'string') return parsed.error;
  } catch {
    // Not JSON — the raw body is the best message available.
  }
  return body;
}

/**
 * The capability a proxied path belongs to. Every capability call goes through
 * `/<name>/…`, which is the dev proxy's uniform rule, so the first segment is the
 * name without anything having to declare it twice.
 */
function capabilityFrom(path: string): string | null {
  const segment = path.replace(/^\/+/, '').split(/[/?#]/)[0];
  return segment && !segment.startsWith('api') ? segment : null;
}

/**
 * What to show a reader when a request fails.
 *
 * The case worth naming: when a capability's process is not running, the dev proxy
 * cannot reach it and answers with a bare 5xx and an empty, non-JSON body. Every
 * page then rendered `Request failed (500)`, which says nothing at all — it reads
 * as a bug in the page rather than as a service that was never started, and it
 * sends the reader looking in the wrong place. There is nothing wrong in that
 * situation except that nobody started the thing, so it says so, with the command.
 */
export function describeFailure(status: number, body: string, path: string): string {
  const capability = capabilityFrom(path);
  const named = errorMessage(body).trim();
  if (named) return capability ? `${capability}: ${named}` : named;
  if (capability && status >= 500) {
    return `${capability} is not running — start it with: tools/service-runner.sh start ${capability}`;
  }
  return capability ? `${capability}: request failed (${status})` : `Request failed (${status})`;
}

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const res = await fetch(path, init);
  if (!res.ok) {
    const body = await res.text().catch(() => '');
    throw new ApiError(res.status, describeFailure(res.status, body, path));
  }
  const text = await res.text();
  const parsed = text ? JSON.parse(text) : undefined;
  if (parsed && typeof parsed === 'object' && 'error' in parsed && parsed.error) {
    throw new ApiError(res.status, String(parsed.error));
  }
  return parsed as T;
}

const jsonInit = (method: string, body: unknown): RequestInit => ({
  method,
  headers: { 'Content-Type': 'application/json' },
  body: JSON.stringify(body),
});

// ─── Types ───────────────────────────────────────────────────────────────────

export type PlaceKind = 'address' | 'airport' | 'city' | 'station' | 'venue';
export type TransportMode = 'bike' | 'bus' | 'car' | 'ferry' | 'flight' | 'train' | 'walk';

export interface PlaceRef {
  id: string;
  name: string;
  kind?: PlaceKind;
  address?: string | null;
  latitude?: number | null;
  longitude?: number | null;
}

export interface Station extends PlaceRef {
  kind?: 'station';
}

export interface TripStage {
  id: string;
  sequence: number;
  origin: PlaceRef;
  destination: PlaceRef;
  date: string | null;
  transport_modes: TransportMode[];
  travelers: string[];
  status: 'planning' | 'option_selected' | 'booked' | 'completed';
  selected_option_id: string | null;
}

export interface PlanSource {
  kind: string;
  reference: string;
}

export interface TripPlan {
  id: string;
  title: string;
  origin: PlaceRef;
  destinations: PlaceRef[];
  date_start: string;
  date_end: string;
  interests: string;
  status: 'draft' | 'saved' | 'archived';
  travelers: string[];
  transport_modes: TransportMode[];
  stages: TripStage[];
  cover_image_url: string | null;
  source: PlanSource | null;
  created_at: string;
  updated_at: string;
}

export type PlanItemType =
  | 'journey'
  | 'transport'
  | 'event'
  | 'activity'
  | 'place'
  | 'stay'
  | 'image'
  | 'note';

export interface PlanItem {
  id: string;
  plan_id: string;
  item_type: PlanItemType;
  day: string | null;
  external_id: string;
  title: string;
  payload: unknown;
  created_at: string;
}

export interface PlanDetails extends TripPlan {
  items: PlanItem[];
}

export interface ObsidianTripCandidate {
  reference: string;
  title: string;
  summary: string;
  date_start: string | null;
  date_end: string | null;
  destination: PlaceRef | null;
  status: string;
  travelers: string[];
  transport_modes: TransportMode[];
  cover: string | null;
  issues: string[];
  imported_plan_id: string | null;
}

export interface ObsidianImportAllResult {
  imported: TripPlan[];
  existing: TripPlan[];
  skipped: Array<{
    reference: string;
    title: string;
    issues: string[];
  }>;
}

export interface ScoredResult {
  id: string;
  rank: number;
  score: number;
  source: string;
  title: string;
  date: string | null;
  location: string | null;
  city: string | null;
  matched_focus: string | null;
  rationale: string;
  url: string;
  opportunity_type: string;
  status: OpportunityStatus;
  vault_link: string | null;
  event_route: EventRoute | null;
}

export interface DiscoverResponse {
  adapter: string;
  opportunity_type: string;
  total_scored: number;
  new_count: number;
  vault_links: number;
  store_total: number;
  results: ScoredResult[];
}

export type OpportunityStatus = 'new' | 'saved' | 'dismissed';

export type EventRouteKind = 'local' | 'travel_candidate' | 'online' | 'unresolved';

export interface EventRoute {
  route: EventRouteKind;
  basis:
    | 'source_metadata'
    | 'location_text'
    | 'coordinates'
    | 'country'
    | 'timezone'
    | 'operator_override'
    | 'missing_policy'
    | 'missing_evidence';
  reason: string;
  distance_km: number | null;
}

export interface ScoutingOpportunity {
  id: string;
  opportunity_type: string;
  source: string;
  title: string;
  city: string;
  starts_at: string;
  ends_at: string;
  location: string;
  score: number;
  matched_focus: string;
  rationale: string;
  url: string;
  vault_link: string | null;
  status: OpportunityStatus;
  country_code: string | null;
  latitude: number | null;
  longitude: number | null;
  event_route: EventRoute | null;
}

export interface ScoutingSource {
  id: string;
  adapter: string;
  enabled: boolean;
  configured: boolean;
  root_path: string | null;
  url: string | null;
  opportunity_type: string;
}

export interface AxonStatusHealth {
  ok: boolean;
  version: string;
  uptime_seconds: number;
  capabilities: Record<string, { up: boolean; url: string }>;
}

export const transit = {
  suggest: (q: string) => request<Station[]>(`/api/suggest?q=${encodeURIComponent(q)}`),
  // Journey, not a narrower shape of its own: /api/search and /api/split's segments are
  // the same serialized type on the server, start_station and end_station included. The
  // old client declared a second interface without those two fields, so a caller could
  // not name where a journey actually started.
  search: (from: string, to: string, time: string) =>
    request<Journey[]>(
      `/api/search?from=${encodeURIComponent(from)}&to=${encodeURIComponent(to)}&time=${encodeURIComponent(time)}`,
    ),
  split: (from: string, to: string, time: string) =>
    request<SplitResult>(
      `/api/split?from=${encodeURIComponent(from)}&to=${encodeURIComponent(to)}&time=${encodeURIComponent(time)}`,
    ),
};

export const trips = {
  list: () => request<TripPlan[]>('/trips/api/plans'),
  create: (plan: {
    title: string;
    origin: PlaceRef;
    destinations: PlaceRef[];
    date_start: string;
    date_end: string;
    interests: string;
    travelers: string[];
    transport_modes: TransportMode[];
  }) => request<TripPlan>('/trips/api/plans', jsonInit('POST', plan)),
  update: (
    id: string,
    patch: Partial<
      Pick<
        TripPlan,
        | 'title'
        | 'origin'
        | 'destinations'
        | 'date_start'
        | 'date_end'
        | 'interests'
        | 'status'
        | 'travelers'
        | 'transport_modes'
        | 'stages'
        | 'cover_image_url'
      >
    >,
  ) =>
    request<TripPlan>(
      `/trips/api/plans/${encodeURIComponent(id)}`,
      jsonInit('PATCH', patch),
    ),
  get: (id: string) => request<PlanDetails>(`/trips/api/plans/${encodeURIComponent(id)}`),
  delete: (id: string) =>
    request<void>(`/trips/api/plans/${encodeURIComponent(id)}`, { method: 'DELETE' }),
  addItem: (
    planId: string,
    item: {
      item_type: PlanItemType;
      day: string | null;
      external_id: string;
      title: string;
      payload: unknown;
    },
  ) =>
    request<PlanItem>(
      `/trips/api/plans/${encodeURIComponent(planId)}/items`,
      jsonInit('POST', item),
    ),
  deleteItem: (planId: string, itemId: string) =>
    request<void>(
      `/trips/api/plans/${encodeURIComponent(planId)}/items/${encodeURIComponent(itemId)}`,
      { method: 'DELETE' },
    ),
  scanObsidian: () =>
    request<ObsidianTripCandidate[]>('/trips/api/import/obsidian/scan'),
  importObsidian: (reference: string, origin: PlaceRef) =>
    request<TripPlan>(
      '/trips/api/import/obsidian',
      jsonInit('POST', { reference, origin }),
    ),
  importAllObsidian: (origin: PlaceRef) =>
    request<ObsidianImportAllResult>(
      '/trips/api/import/obsidian/all',
      jsonInit('POST', { origin }),
    ),
};

export const wikimedia = {
  placeImage: (title: string) => {
    const params = new URLSearchParams({
      action: 'query',
      format: 'json',
      origin: '*',
      prop: 'pageimages|info',
      inprop: 'url',
      piprop: 'thumbnail|name',
      pithumbsize: '1280',
      pilicense: 'free',
      redirects: '1',
      titles: title,
    });
    return request<unknown>(`https://de.wikipedia.org/w/api.php?${params}`);
  },
  nearby: (latitude: number, longitude: number) => {
    const params = new URLSearchParams({
      action: 'query',
      format: 'json',
      origin: '*',
      generator: 'geosearch',
      ggscoord: `${latitude}|${longitude}`,
      ggsradius: '10000',
      ggslimit: '12',
      prop: 'pageimages|info|coordinates|description',
      inprop: 'url',
      piprop: 'thumbnail|name',
      pithumbsize: '640',
    });
    return request<unknown>(`https://de.wikipedia.org/w/api.php?${params}`);
  },
};

export interface SplitResult {
  segments: Journey[];
  original_price: number;
  split_price: number;
  savings: number;
}

export interface Journey {
  id: string;
  start_station: Station;
  end_station: Station;
  legs: ConnectionLeg[];
  total_duration_minutes: number;
  total_price: number | null;
  delay_risk_score: number | null;
}

export interface ConnectionLeg {
  origin: { name: string; id: string };
  destination: { name: string; id: string };
  departure_time: string;
  arrival_time: string;
  train_name: string;
  train_number: string;
  train_category: string;
  is_regional: boolean;
  platform?: string | null;
}

export const scouting = {
  discover: (opts: { adapter: string; location?: string; query?: string }) => {
    const params = new URLSearchParams({ adapter: opts.adapter });
    if (opts.location) params.set('location', opts.location);
    if (opts.query) params.set('query', opts.query);
    return request<DiscoverResponse>(`/discover?${params.toString()}`);
  },
  // /scouting, not /scout-server: every capability is proxied at /<capability name>,
  // derived from the registry rather than hand-listed (see vite.config.ts).
  health: () => request<unknown>('/scouting/health'),
  sources: () =>
    request<{ sources: ScoutingSource[]; count: number }>('/scouting/sources'),
  opportunities: (includeDismissed = true) =>
    request<{ opportunities: ScoutingOpportunity[]; count: number; store_total: number }>(
      `/scouting/opportunities?include_dismissed=${includeDismissed}`,
    ),
  setStatus: (id: string, status: OpportunityStatus) =>
    request<{ id: string; status: OpportunityStatus }>(
      `/scouting/opportunities/${encodeURIComponent(id)}/status`,
      jsonInit('POST', { status }),
    ),
};

export interface CapabilityView {
  name: string;
  kind: 'process' | 'container';
  scope: 'capability' | 'spine';
  port: string;
  panel_port: string;
  panel_path: string;
  autostart: string;
  requires: string[];
  /** null when the capability declares no health surface — unknown, not down. */
  up: boolean | null;
  health_url: string | null;
}

/** A capability that serves its own UI (README.md#three-architectural-nouns) declares a panel port. */
export const hasPanel = (c: CapabilityView): boolean => c.panel_port !== '';

/**
 * Where a panel lives, as seen from THIS browser.
 *
 * Composed here rather than served by axon-status, because a panel is loaded by a
 * browser and has to be reachable on the host that browser is already on. Serving
 * `127.0.0.1:<port>` to a shell opened at `localhost` makes the two different sites:
 * Chrome then partitions the frame's storage, and a framework whose client init touches
 * storage never hydrates, so the panel renders blank while every request returns 200.
 * Over Tailscale the same absolute address would point at the phone itself.
 */
export function panelUrl(c: CapabilityView): string {
  return `${location.protocol}//${location.hostname}:${c.panel_port}${c.panel_path}`;
}

/** One unit in the committed self-model. `service` exists only where a manifest does. */
export interface SelfUnit {
  name: string;
  kind: 'capability' | 'lib' | 'spine' | 'pack' | 'unknown';
  service?: { kind: string; port?: string; requires: string[]; image?: string };
  code?: { files: number; nodes: number };
}

/** Compile-time coupling: what is pulled into what. Not `requires`, which is runtime. */
export interface SelfCoupling {
  from: string;
  to: string;
  kinds: string[];
  evidence: string[];
}

export interface SelfModel {
  schema: number;
  generator: string;
  units: SelfUnit[];
  coupling: SelfCoupling[];
  upstreams: Array<{ name: string; verdict: string; pin: string }>;
  graph: { present: boolean; nodes: number; external: number; stale: string[]; unmatched: string[] };
}

/**
 * The self-model as served: the committed artifact plus this machine's live state.
 *
 * Two keys, not one merged object, mirroring the endpoint. `model` is a fact about the
 * repo and is identical on every machine; `live` is a fact about this machine right now.
 * `up` is `null` where a capability declares nothing to poll — unknown, not down.
 */
export interface SelfModelResponse {
  model: SelfModel;
  live: Record<string, boolean | null>;
}

/**
 * Version identity for one of the two repos this installation is made of.
 *
 * Read-only by design: the shell shows which Axon is running and links to it, and
 * deliberately cannot tag, commit or push — a browser button that writes to git needs a
 * gate in front of it, and there is no versioning scheme to write against yet.
 *
 * `tag` is null on a repo that has never been tagged, which today is both of them.
 * `describe` then degrades to a short sha, and that is the honest answer rather than a
 * missing one. `ahead`/`behind` are null when the branch tracks no upstream — not 0,
 * which would claim it is in sync with something.
 */
export interface RepoStatus {
  name: string;
  role: 'spine' | 'overlay';
  remote_url: string | null;
  branch: string | null;
  describe: string | null;
  tag: string | null;
  commits_since_tag: number | null;
  ahead: number | null;
  behind: number | null;
  dirty: boolean;
  last_commit_date: string | null;
  error: string | null;
}

/**
 * The upstream-dependency audit, as `tools/upstream-checker --json` renders it.
 *
 * `status` mirrors the checker's own per-entry verdict: `ok` clean, `na` release-drift
 * checking declared inapplicable, `warn` a pin/cooldown caution, `fail` a
 * completeness/verdict violation. `na` is its own group rather than a kind of `ok`: it marks
 * an entry nothing checks, which stays worth seeing (`pin_kind` names the shape, and the
 * entry's `tracked_by` note says what governs it instead). `notes` are the checker's own
 * lines, emoji and all — the one source, shared with the human report. `offline` is true when
 * drift/cooldown were skipped (the dashboard poll always is; see the endpoint's README).
 */
export interface UpstreamEntry {
  name: string;
  verdict: string;
  pin: string;
  pin_kind: string;
  url: string;
  status: 'ok' | 'na' | 'warn' | 'fail';
  notes: string[];
}
export interface UpstreamAudit {
  manifest: string;
  offline: boolean;
  totals: { count: number; ok: number; na: number; warn: number; fail: number };
  entries: UpstreamEntry[];
}

/** How a capability's last backup stands against its own declared thresholds. */
export type BackupState =
  /** Has a backup contract and no receipt at all. Outranks every threshold. */
  | 'never'
  | 'ok'
  /** Past `advise_days`: you could take one. */
  | 'due'
  /** Past `stale_days`: you have a problem. */
  | 'overdue'
  /** The capability declares neither threshold, so nothing knows what timely means for
   *  its data — and Axon will not invent a cadence to fill the gap. */
  | 'unknown';

/** An in-flight or finished run, as the server remembers it. Null when axon-status has
 *  not been asked for a backup of this capability since it started. */
export interface BackupRun {
  state: 'running' | 'succeeded' | 'failed';
  started_at: number;
  finished_at?: number;
  detail?: string;
}

/**
 * One capability's backup standing.
 *
 * Note what is absent: no target, no tarball, no hash. The receipt on disk carries all
 * three and the server drops them before answering — where a backup goes is the
 * overlay's business, and a dashboard has no use for it.
 */
export interface BackupStatus {
  capability: string;
  state: BackupState;
  /** A run stops this capability while it takes a cold copy. Say so before confirming. */
  holds_service: boolean;
  advise_days: number | null;
  stale_days: number | null;
  last_success: string | null;
  age_seconds: number | null;
  bytes: number | null;
  contents: string | null;
  run: BackupRun | null;
}

export const axonStatus = {
  health: () => request<AxonStatusHealth>('/axon-status/api/axon-status/health'),
  capabilities: () => request<CapabilityView[]>('/axon-status/api/axon-status/capabilities'),
  backups: () => request<{ backups: BackupStatus[] }>('/axon-status/api/axon-status/backups'),
  /** Accepts the run and returns — it does not wait for it. Poll `backups()` for the
   *  outcome, which is also what lets a slow run survive a page refresh. */
  backup: (name: string) =>
    request<{ name: string; accepted: boolean; holds_service: boolean }>(
      `/axon-status/api/axon-status/capabilities/${encodeURIComponent(name)}/backup`,
      { method: 'POST' },
    ),
  self: () => request<SelfModelResponse>('/axon-status/api/axon-status/self'),
  repos: () => request<{ repos: RepoStatus[] }>('/axon-status/api/axon-status/repos'),
  upstreams: () => request<UpstreamAudit>('/axon-status/api/axon-status/upstreams'),
  start: (name: string, signal?: AbortSignal) =>
    request<{ name: string; up: boolean; detail: string }>(
      `/axon-status/api/axon-status/capabilities/${encodeURIComponent(name)}/start`,
      { method: 'POST', signal },
    ),
  stop: (name: string) =>
    request<{ name: string; up: boolean; detail: string }>(
      `/axon-status/api/axon-status/capabilities/${encodeURIComponent(name)}/stop`,
      { method: 'POST' },
    ),
};

/** One graphify node inside a unit: a file or a symbol the extractor found. */
export interface UnitGraphNode {
  id: string;
  label: string;
  source_file: string;
  file_type: string;
  community: number;
}

/**
 * One unit's slice of the graphify graph, capped by the server.
 *
 * `total` and `truncated` are the honest part of the contract: the full graph
 * is thousands of nodes, a drill-down returns the busiest few hundred, and a
 * reader has to be able to tell a complete unit from a sampled one.
 */
export interface UnitGraph {
  unit: string;
  prefixes: string[];
  total: number;
  returned: number;
  truncated: boolean;
  cap: number;
  nodes: UnitGraphNode[];
  edges: Array<{ from: string; to: string; label?: string }>;
}

export const knowledgeGraph = {
  unit: (name: string) =>
    request<UnitGraph>(`/knowledge-graph/api/graph/unit/${encodeURIComponent(name)}`),
};

/** One sample from macmon pipe/serve. Every field present on Apple Silicon. */
export interface MacmonSample {
  all_power: number;
  ane_power: number;
  cpu_power: number;
  cpu_usage_pct: number;
  ecpu_usage: [number, number];  // [frequency_mhz, utilisation_0_1]
  gpu_power: number;
  gpu_ram_power: number;
  gpu_usage: [number, number];   // [frequency_mhz, utilisation_0_1]
  memory: {
    ram_total: number;
    ram_usage: number;
    swap_total: number;
    swap_usage: number;
  };
  pcpu_usage: [number, number];  // [frequency_mhz, utilisation_0_1]
  ram_power: number;
  sys_power: number;
  temp: {
    cpu_temp_avg: number;
    gpu_temp_avg: number;
  };
  timestamp: string;
}

// The real, external LifeOS Life Dashboard — not an Axon capability, never
// conflated with axon-status. See vite.config.ts's /lifeos-pulse proxy.
export const lifeosPulse = {
  // /healthz, not /health: Pulse deliberately moved the JSON health API off /health
  // to make room for the Life Dashboard's HTML /health page — probing /health gets
  // 200 text/html, JSON.parse throws, and the Systems page reports Pulse "down"
  // while it is demonstrably up. That exact bug shipped once; the path is load-bearing.
  health: () => request<unknown>('/lifeos-pulse/healthz'),
};

// macmon — sudoless Apple Silicon performance monitor. Not an Axon capability.
// Proxied at /macmon → http://localhost:9911 (see vite.config.ts).
export const macmon = {
  json: () => request<MacmonSample>('/macmon/json'),
};

// ─── Comms feed ──────────────────────────────────────────────────────────────

export type FeedStream = 'news' | 'media';
export type FeedKind =
  | 'youtube'
  | 'instagram'
  | 'podcast'
  | 'article'
  | 'mail'
  | 'github'
  | 'arxiv'
  | 'reddit';
export type FeedStatus = 'new' | 'keeper' | 'dismissed';

export interface FeedRelevance {
  profile_key: string;
  profile_label: string;
  score: number;
  rationale: string;
  mode: 'reranked' | 'semantic' | 'lexical';
  profile_revision: string;
}

export interface FeedEvaluationFactor {
  key: string;
  label: string;
  score: number;
  weight: number;
  rationale: string;
  context: {
    kind: 'trip' | string;
    id: string;
    label: string;
    date_start: string | null;
    date_end: string | null;
    matched_terms: string[];
  } | null;
}

export interface FeedEvaluation {
  overall_score: number;
  explanation: string;
  mode: 'reranked' | 'semantic' | 'lexical' | 'unscored';
  item_revision: string;
  context_revision: string;
  evaluator_revision: string;
  evaluated_at: string;
  factors: FeedEvaluationFactor[];
}

export interface CommsEvaluationStatus {
  evaluator_revision: string;
  context_revision: string;
  ledger: {
    evaluated: number;
    reranked: number;
    semantic: number;
    lexical: number;
    unscored: number;
  };
  summarizer: {
    provider: string;
    model: string;
    configured: boolean;
    reachable: boolean;
  };
  relevance: {
    provider: string;
    model: string;
    configured: boolean;
    reachable: boolean;
    profile_count: number;
    active_mode: 'reranked' | 'semantic' | 'lexical';
  };
  reranker: {
    provider: string;
    model: string;
    configured: boolean;
    reachable: boolean;
  };
  travel_context: {
    enabled: boolean;
    source: string;
    upcoming_count: number;
    reachable: boolean;
    from_cache: boolean;
    refreshed_at: string;
    plans: Array<{
      id: string;
      label: string;
      date_start: string;
      date_end: string;
    }>;
  };
}

export interface FeedOrigin {
  source_id: string;
  source_ref: string;
  label: string | null;
}

export interface FeedStageProvenance {
  stage: "extraction" | "normalization" | "summary" | "ranking";
  tier: "legacy" | "deterministic" | "model" | "human";
  revision: string;
  completed_at: string;
}

export interface FeedQualityFlag {
  feed_id: string;
  title: string | null;
  url: string;
  status: FeedStatus;
  content_status: "full" | "thin" | "none" | "unknown";
  signal:
    | "content_status"
    | "extraction_path"
    | "retention"
    | "boilerplate_leakage"
    | "summary_attempts"
    | "ranking_basis"
    | string;
  reason: string;
  evidence: string;
  derived_at: string;
}

export interface FeedQualityRefresh {
  reviewed: number;
  flagged_items: number;
  flag_count: number;
  bounded_to: number;
  days: number;
  provider_calls: 0;
}

interface FeedEntryBase {
  id: string;
  stream: FeedStream;
  kind: FeedKind;
  title: string | null;
  url: string;
  author: string | null;
  summary: string | null;
  day: string; // YYYY-MM-DD grouping key
  created_at: string;
  status: FeedStatus;
}

export interface FeedEntry extends FeedEntryBase {
  relevance: FeedRelevance | null;
  evaluation: FeedEvaluation | null;
}

export interface FeedEntryDetail extends FeedEntryBase {
  transcript: string | null;
  relevance: FeedRelevance[];
  evaluation: FeedEvaluation | null;
  content_status: "full" | "thin" | "none" | "unknown";
  /** Which client handed the content over; null when the server fetched it. */
  captured_via: string | null;
  processing: FeedStageProvenance[];
  origins: FeedOrigin[];
}

export type ContentSource = 'feed' | 'mail' | 'calendar';

/**
 * Which capability owns each content source.
 *
 * The contract is shared; the data is not. A source is served by the capability
 * that stores it, and every one of them exposes the same `/content/:source/:id`
 * shape — so the reader resolves an item from its source alone and no caller
 * carries a per-capability special case. Adding a source is one line here.
 */
const CONTENT_BASE: Record<ContentSource, string> = {
  feed: '/comms',
  mail: '/comms',
  calendar: '/calendar/api',
};

/** Any content item, from whichever capability owns that source. */
export function contentItem(source: ContentSource, id: string, signal?: AbortSignal) {
  return request<ContentItemDetail>(
    `${CONTENT_BASE[source]}/content/${source}/${encodeURIComponent(id)}`,
    signal ? { signal } : undefined,
  );
}
export type DataClass = 'public' | 'personal' | 'vault';

export interface ContentDataClass {
  value: DataClass;
  label: 'Public' | 'Personal' | 'Private';
  rationale: string;
  method: 'source-default' | 'rules' | 'human';
  version: string;
}

export interface ContentProcessingPolicy {
  local_processing: 'allowed';
  cloud_handling: 'eligible' | 'pseudonymization_required' | 'blocked';
  pseudonymization_required: boolean;
  rationale: string;
}

export interface CloudProcessingState {
  status: 'not_prepared' | 'staged' | 'stale';
  preview_hash: string | null;
  approved_at: string | null;
  dispatch_status: 'not_queued' | 'queued' | 'running' | 'succeeded' | 'failed';
  job_id: string | null;
  provider_role: string | null;
  queued_at: string | null;
  provider_calls: number;
  task: 'content-analysis-v1' | null;
  started_at: string | null;
  completed_at: string | null;
  last_error: string | null;
  result: CloudContentAnalysis | null;
}

export interface CloudContentAnalysis {
  schema_version: 'cloud-content-analysis-v1';
  summary: string;
  importance: 'low' | 'medium' | 'high';
  importance_rationale: string;
  important_dates: Array<{ label: string; date: string | null; source_text: string }>;
  action_items: Array<{ text: string; due_date: string | null }>;
  topics: string[];
}

export interface CloudProvider {
  role: string;
  name: string;
  model: string;
  provider_label: string;
  location: 'cloud';
  data_tier: 'public' | 'pseudonymized_personal';
  billing_mode: 'free_only' | 'prepaid_credit';
  available: boolean;
}

export interface RedactionFinding {
  entity_type: string;
  marker: string;
  count: number;
}

export interface CloudDerivativePreview {
  schema_version: 'cloud-derivative-preview-v1';
  source: ContentSource;
  id: string;
  source_revision: string;
  preview_hash: string;
  original_data_class: DataClass;
  derivative_data_class: 'public' | 'personal';
  transformation: 'bounded-public-v1' | 'deterministic-entity-redaction-v2';
  document: string;
  redaction_count: number;
  redactions: RedactionFinding[];
  entity_detection: 'not-required' | 'local-deterministic-v2';
  truncated: boolean;
  approval_required: true;
  provider_calls: 0;
  limitations: string[];
}

export interface MailContentExtension {
  category: MailCategory;
  rationale: string;
  classification_method: 'rules' | 'human';
  classification_version: string;
  gmail_action: 'archive' | 'trash' | 'restore' | null;
  gmail_action_at: string | null;
  purge_after: string | null;
  gmail_location: 'inbox' | 'archive' | 'trash' | 'missing' | null;
  gmail_observed_at: string | null;
  gmail_sync_status: 'synced' | 'queued' | 'retrying' | 'attention' | null;
  gmail_sync_action: 'archive' | 'trash' | 'restore' | null;
  gmail_sync_error: string | null;
  /** The doctrine's one state label, mirrored from Gmail. Separate from `status`
   *  on purpose: status is what Axon decided about a proposal, waiting is what you
   *  decided about the conversation. */
  waiting: boolean;
  waiting_since: string | null;
}

/** Versioned reader contract shared by Feed sources and mail proposals. */
export interface ContentItemDetail {
  schema_version: 'content-item-v1';
  source: ContentSource;
  id: string;
  /** The source's own type discriminator, not a shared enum — a feed article is
   *  an `article`, a calendar entry is a `nightlife` or `work_onsite`. Open
   *  string, exactly as the schema types it: a source may add a kind without a
   *  dashboard release, and every reader already falls back for unknown ones. */
  kind: string;
  title: string | null;
  url: string;
  author: string | null;
  summary: string | null;
  content: string | null;
  content_label: string;
  day: string;
  created_at: string;
  /** Each source's triage axis, in one field: feed keeps or dismisses, mail
   *  moves through Gmail states, calendar commits. */
  status: FeedStatus | TriageStatus | CalendarCommitment;
  content_status: 'full' | 'thin' | 'none' | 'unknown';
  data_class: ContentDataClass;
  processing_policy: ContentProcessingPolicy;
  cloud_processing: CloudProcessingState;
  relevance: FeedRelevance[];
  evaluation: FeedEvaluation | null;
  processing: FeedStageProvenance[];
  origins: FeedOrigin[];
  links: ContentLink[];
  /** What the local model wrote about this item. Deliberately not `summary`,
   *  which is what the *source* said it is — calendar reads that from the
   *  entry's own description, and a generated paragraph written over it would
   *  destroy the only verbatim text an entry has. A reader wanting the short
   *  version prefers `digest.text` and falls back to `summary`. */
  digest: ContentDigest | null;
  mail: MailContentExtension | null;
  calendar: CalendarContentExtension | null;
}

/** Why there is no digest text. `skipped_short` is a verdict about the source —
 *  it is already shorter than any honest digest of it — not a failure. */
export type DigestState =
  | 'generated'
  | 'skipped_short'
  | 'remote_refused'
  | 'unconfigured'
  | 'http_error'
  | 'model_error'
  /** The server took the request and then ran out of room for it. A fact about
   *  the machine rather than the request, so it is worth retrying. */
  | 'capacity_aborted'
  | 'empty_response'
  | 'timeout';

/** The rung the length ladder landed on. Derived from `source_chars`, never
 *  chosen directly — see libs/summarize/README.md. */
export type DigestShape = 'none' | 'brief' | 'standard' | 'sectioned';

/** `detailed` moves the shape exactly one rung up that same ladder. It is not a
 *  separate instruction to the model, which is why it stays inspectable. */
export type DigestDepth = 'standard' | 'detailed';

export interface ContentDigest {
  text: string | null;
  state: DigestState;
  shape: DigestShape;
  depth: DigestDepth;
  /** The operator's focus terms, as typed. Shown back so a differently-shaped
   *  digest is explained rather than mysterious. */
  focus: string[];
  producer: string;
  source_chars: number;
  /** Entities the deterministic redactor removed before this text was stored.
   *  Non-zero only for Private content. */
  redactions: number;
  attempts: number;
  last_error: string | null;
  /** Mermaid source, validated against the known diagram headers before it was
   *  stored — a string the renderer cannot draw never gets here. */
  diagram: string | null;
  diagram_state: string | null;
  diagram_error: string | null;
  /** The table pulled out of the source, or null. Deliberately data rather than
   *  a chart specification: the reader compiles one, so a model never reaches
   *  the rendering layer. Every value appeared verbatim in the source text
   *  before it was admitted. */
  chart: ContentChartData | null;
  /** `generated`, or `skipped_short` when the source holds no comparable
   *  numbers — the answer for most prose, and not a failure. */
  chart_state: string | null;
  chart_error: string | null;
  generated_at: string;
}

/** One measure over a handful of categories. One series is the maximum by
 *  design: the figure palette is a low-chroma print palette that cannot carry
 *  categorical identity, so a chart drawn from it must not need to. */
export interface ContentChartData {
  title: string;
  category_label: string;
  measure_label: string;
  unit: string | null;
  /** Derived from the categories, never chosen by a model: an ordered run of
   *  three or more gets a line, everything else bars. */
  mark: "bar" | "line";
  note: string;
  rows: { category: string; value: number }[];
}

/** A named way out of an item: source page, the mail that carried the ticket,
 *  a map, a vault note. `kind` is a presentation hint and stays open — an
 *  unknown one still renders a working link. */
export interface ContentLink {
  label: string;
  kind: string;
  url: string;
}

/** What only a calendar entry has. No score on purpose: a decided item is not
 *  ranked, so `commitment` is its triage axis. */
export interface CalendarContentExtension {
  starts_at: string;
  /** Exclusive, as everywhere in calendar. */
  ends_at: string;
  all_day: boolean;
  commitment: 'possible' | 'planned' | 'committed';
  location: string | null;
  /** The operator's own note — why they care, not what the thing is. */
  notes: string | null;
  /** Which adapter contributed the row (`manual`, `luma`, `google`) — not the
   *  item's `source`, which is always `calendar`. Decides which actions are
   *  honest: an entry imported from Google must not offer to export back. */
  entry_source: string;
  /** Set when materialized from a rhythm. Such an instance is not exported
   *  individually, and any patch detaches it from its rhythm. */
  rhythm_id: string | null;
}

// One item's place in a collector run. Derived server-side per request, so a
// run never becomes stale state on the entry itself.
export interface FeedRun {
  feed_id: string;
  source_id: string;
  label: string | null;
  run_key: string;
  run_started: string | null;
}

export interface VaultLinkCandidate {
  id: string;
  source_id: string;
  source_ref: string;
  label: string | null;
  url: string;
  imported: boolean;
}

export interface FeedSource {
  id: string;
  adapter: 'github-trending' | 'arxiv' | string;
  enabled: boolean;
  source_url: string;
  query_configured: boolean;
  limit: number;
  last_run_at: string | null;
}

export interface FeedSourceScan {
  fetched: number;
  new_count: number;
  sources: Array<{
    source_id: string;
    adapter: string;
    fetched: number;
    new_count: number;
    known_count: number;
  }>;
}

export type MailCategory =
  | 'aktiv'
  | 'issue'
  | 'feed'
  | 'werbung'
  | 'belege'
  | 'steuern'
  | 'sonstiges';

export type TriageStatus =
  | 'proposed'
  | 'approved'
  | 'executed'
  | 'archived'
  | 'trashed'
  | 'missing'
  | 'dismissed';

export interface TriageItem {
  id: string;
  from_addr: string | null;
  subject: string | null;
  snippet: string | null;
  stream: MailCategory;
  rationale: string;
  classification_method: 'rules' | 'human';
  classification_version: string;
  data_class: DataClass;
  data_class_rationale: string;
  data_classification_method: 'rules' | 'human';
  data_classification_version: string;
  status: TriageStatus;
  gmail_action: 'archive' | 'trash' | 'restore' | null;
  gmail_action_at: string | null;
  purge_after: string | null;
  gmail_location: 'inbox' | 'archive' | 'trash' | 'missing' | null;
  gmail_observed_at: string | null;
  gmail_sync_status: 'synced' | 'queued' | 'retrying' | 'attention' | null;
  gmail_sync_action: 'archive' | 'trash' | 'restore' | null;
  gmail_sync_error: string | null;
  /** The doctrine's one state label, mirrored from Gmail. Separate from `status`
   *  on purpose: status is what Axon decided about a proposal, waiting is what you
   *  decided about the conversation. */
  waiting: boolean;
  waiting_since: string | null;
  internal_date: string | null;
  relevance: FeedRelevance[];
}

export interface TriageSweepResult {
  fetched: number;
  new_count: number;
  skipped: number;
  /** Threads whose subject or snippet was redacted before being stored. */
  redacted: number;
  total_stored: number;
  next_cursor: string | null;
  exhausted: boolean;
}

/** Freshness of the unattended inbox sweep. Counts, times and an error class;
 *  no mail reaches this shape. `last_success_at` is deliberately separate from
 *  `last_run_at` — a failing run still ran, and "when did collection last
 *  actually work" is the question a red schedule raises. */
export interface TriageSweepStatus {
  enabled: boolean;
  every_minutes: number;
  max_threads: number;
  quiet_hours: { start: number; end: number } | null;
  last_run_at: string | null;
  last_success_at: string | null;
  last_failure_at: string | null;
  last_error: 'auth' | 'quota' | 'network' | 'unknown' | null;
  considered_count: number;
  new_count: number;
  consecutive_failures: number;
}

/** What a redaction pass removed, by kind and count — never the values. */
export interface TriageRedactResult {
  reviewed: number;
  in_scope: number;
  changed: number;
  dry_run: boolean;
  entity_types: Record<string, number>;
  audit: { id: string; digest: string }[];
  transformation: string;
  provider_calls: number;
}

export interface TriageRelevanceResult {
  scored: number;
  profile_count: number;
  mode: 'reranked' | 'semantic' | 'lexical' | null;
  local_only: true;
}

export interface TriageDataClassRefreshResult {
  reviewed: number;
  updated: number;
  preserved_human: number;
  classifier_version: string;
  content_inputs: ['sender', 'subject', 'category'];
  provider_calls: 0;
}

export interface TriageBulkResult {
  succeeded: string[];
  failures: Array<{ id: string; error: string }>;
  gmail_changed: boolean;
}

export interface GmailMaintenanceResult {
  retried: number;
  recovered: number;
  retry_failures: number;
  reconciled: number;
  changed: number;
  read_failures: number;
  missing: number;
  content_fetched: false;
}

/** How binding an entry is, orthogonal to `kind`. The calendar capability
 * caps an entry's feasibility impact by this: `possible` can never block a
 * day, `committed` lets the kind decide. */
export type CalendarCommitment = "possible" | "planned" | "committed";

export interface CalendarEntry {
  id: string;
  kind: string;
  commitment: CalendarCommitment;
  title: string;
  starts_at: string;
  ends_at: string;
  all_day: boolean;
  location: string | null;
  notes: string | null;
  source: string;
  external_id: string | null;
  rhythm_id: string | null;
  payload: unknown;
  created_at: string;
  updated_at: string;
}

export interface CalendarNewEntry {
  kind: string;
  /** Omitted means `possible` server-side. The form always sends it. */
  commitment?: CalendarCommitment;
  title: string;
  starts_at: string;
  ends_at: string;
  all_day?: boolean;
  location?: string | null;
  notes?: string | null;
  source?: string;
  external_id?: string | null;
  rhythm_id?: string | null;
  payload?: unknown;
}

export interface CalendarUpdateEntry {
  kind?: string;
  commitment?: CalendarCommitment;
  title?: string;
  starts_at?: string;
  ends_at?: string;
  all_day?: boolean;
  location?: string | null;
  notes?: string | null;
}

export interface CalendarContext {
  id: string;
  kind: string;
  title: string;
  details: string;
  valid_from: string;
  valid_until: string;
  source: string;
  created_at: string;
  updated_at: string;
}

export interface CalendarNewContext {
  kind: string;
  title: string;
  details?: string;
  valid_from: string;
  valid_until: string;
  source?: string;
}

export interface CalendarUpdateContext {
  kind?: string;
  title?: string;
  details?: string;
  valid_from?: string;
  valid_until?: string;
}

/** A run of days travel is possible at all — the calendar capability's own
 * verdict, not something the UI recomputes. */
export interface CalendarFeasibleWindow {
  starts_on: string;
  /** Exclusive, like every end in this capability. */
  ends_before: string;
  days: string[];
  verdict: "free" | "needs-travel-day" | "conflicts";
  days_needing_travel_day: string[];
}

export interface CalendarWindows {
  from: string;
  to: string;
  min_days: number;
  windows: CalendarFeasibleWindow[];
}

/** Calendar's explanation of how one external opportunity fits a real time
 * window. The dashboard renders this evidence; it never recomputes conflicts. */
export interface CalendarCandidateVerdict {
  id: string;
  verdict: 'free' | 'needs-travel-day' | 'conflicts';
  starts_at: string;
  ends_at: string;
  already_in_calendar: boolean;
  evidence: Array<{
    entry_id: string;
    kind: string;
    commitment: CalendarCommitment;
    title: string;
    starts_at: string;
    ends_at: string;
    all_day: boolean;
    impact: 'free' | 'needs-travel-day' | 'conflicts';
  }>;
}

export interface CalendarRhythm {
  id: string;
  kind: string;
  title: string;
  location: string | null;
  byweekday: string[];
  start_time: string | null;
  end_time: string | null;
  valid_from: string;
  valid_until: string;
  active: boolean;
  created_at: string;
  updated_at: string;
}

export interface CalendarNewRhythm {
  kind: string;
  title: string;
  location?: string | null;
  byweekday: string[];
  start_time?: string | null;
  end_time?: string | null;
  valid_from: string;
  valid_until: string;
}

export interface CalendarUpdateRhythm {
  kind?: string;
  title?: string;
  location?: string;
  byweekday?: string[];
  start_time?: string;
  end_time?: string;
  valid_from?: string;
  valid_until?: string;
  active?: boolean;
}

export type GoogleImportReviewStatus =
  | 'importable'
  | 'likely-duplicate'
  | 'already-in-axon'
  | 'cancelled'
  | 'invalid';

/** One Google event in a read-only, date-bounded import review. The revision
 * is returned to the server with the selection, preventing a changed event
 * from being imported behind the operator's back. */
export interface CalendarGoogleImportCandidate {
  google_event_id: string;
  google_updated: string | null;
  title: string;
  starts_at: string | null;
  ends_at: string | null;
  all_day: boolean | null;
  location: string | null;
  html_link: string | null;
  recurring_event_id: string | null;
  status: GoogleImportReviewStatus;
  reason: string | null;
  duplicate_group: string | null;
}

export interface CalendarGoogleImportPreview {
  calendar_id: string;
  home_timezone: string;
  from: string;
  to: string;
  fetched: number;
  at_event_limit: boolean;
  candidates: CalendarGoogleImportCandidate[];
}

export interface CalendarGoogleImportReport {
  fetched: number;
  created: number;
  refreshed: number;
  unchanged: number;
  skipped: Array<{ google_event_id: string; reason: string }>;
}

/** A deliberate per-entry permission to publish an Axon entry to Google.
 * Its presence is the opt-in; it does not itself contact Google. */
export interface CalendarGoogleExportOptIn {
  entry_id: string;
  google_calendar_id: string;
  google_event_id: string | null;
  pushed_at: string | null;
  created_at: string;
}

export interface CalendarGoogleExportReport {
  calendar_id: string;
  home_timezone: string;
  opted_in: number;
  inserted: number;
  patched: number;
  pushed: Array<{
    entry_id: string;
    title: string;
    operation: 'inserted' | 'patched';
    google_event_id: string | null;
  }>;
  skipped: Array<{ google_event_id: string; reason: string }>;
  dry_run: boolean;
}

/** A proposed journey derived from dated Calendar entries at one place. This
 * remains recomputed evidence until the operator explicitly materialises it. */
export interface CalendarTripDraft {
  place: string;
  starts_on: string;
  /** Exclusive, following Calendar's own time model. */
  ends_before: string;
  entry_ids: string[];
  titles: string[];
  commitment: CalendarCommitment;
}

export interface CalendarTripDrafts {
  from: string;
  to: string;
  max_gap_days: number;
  home: string | null;
  drafts: CalendarTripDraft[];
  unclustered: Array<{ entry_id: string; title: string; reason: string }>;
}

export interface CalendarTripMaterialization {
  plan_id: string;
  created: boolean;
  reason?: string;
}

export const calendar = {
  proposals: (from: string, to: string) =>
    request<CalendarEntry[]>(
      `/calendar/api/proposals?from=${encodeURIComponent(from)}&to=${encodeURIComponent(to)}`,
    ),
  tripDrafts: {
    list: (from: string, to: string, maxGapDays = 5) =>
      request<CalendarTripDrafts>(
        `/calendar/api/trip-drafts?${new URLSearchParams({ from, to, max_gap_days: String(maxGapDays) })}`,
      ),
    materialize: (entryIds: string[], title?: string) =>
      request<CalendarTripMaterialization>(
        '/calendar/api/trip-drafts/materialize',
        jsonInit('POST', { entry_ids: entryIds, title: title?.trim() || null }),
      ),
  },
  google: {
    exports: () => request<CalendarGoogleExportOptIn[]>('/calendar/api/google/exports'),
    previewExport: () =>
      request<CalendarGoogleExportReport>('/calendar/api/google/export', jsonInit('POST', { dry_run: true })),
    export: () =>
      request<CalendarGoogleExportReport>('/calendar/api/google/export', jsonInit('POST', { dry_run: false })),
    optInExport: (entryId: string) =>
      request<CalendarGoogleExportOptIn>(
        `/calendar/api/entries/${encodeURIComponent(entryId)}/google-export`,
        jsonInit('PUT', {}),
      ),
    optOutExport: (entryId: string) =>
      request<void>(
        `/calendar/api/entries/${encodeURIComponent(entryId)}/google-export`,
        { method: 'DELETE' },
      ),
    drafts: (from: string, to: string) =>
      request<CalendarEntry[]>(
        `/calendar/api/google/drafts?from=${encodeURIComponent(from)}&to=${encodeURIComponent(to)}`,
      ),
    previewImport: (from: string, to: string) =>
      request<CalendarGoogleImportPreview>(
        '/calendar/api/google/import-preview',
        jsonInit('POST', { from, to }),
      ),
    importSelected: (
      from: string,
      to: string,
      selected: Array<{ google_event_id: string; google_updated: string | null }>,
    ) =>
      request<CalendarGoogleImportReport>(
        '/calendar/api/google/import-selected',
        jsonInit('POST', { from, to, selected }),
      ),
  },
  windows: (from: string, to: string) =>
    request<CalendarWindows>(
      `/calendar/api/windows?from=${encodeURIComponent(from)}&to=${encodeURIComponent(to)}`,
    ),
  verdicts: (candidates: Array<{ id: string; starts_at: string; ends_at?: string | null }>) =>
    request<{ verdicts: CalendarCandidateVerdict[] }>(
      '/calendar/api/verdicts',
      jsonInit('POST', { candidates }),
    ),
  entries: {
    list: (from: string, to: string, kind?: string) => {
      const params = new URLSearchParams({ from, to });
      if (kind) params.set('kind', kind);
      return request<CalendarEntry[]>(`/calendar/api/entries?${params}`);
    },
    get: (id: string) =>
      request<CalendarEntry>(`/calendar/api/entries/${encodeURIComponent(id)}`),
    create: (entry: CalendarNewEntry) =>
      request<CalendarEntry>('/calendar/api/entries', jsonInit('POST', entry)),
    upsertExternal: (entry: CalendarNewEntry) =>
      request<CalendarEntry>('/calendar/api/entries/external', jsonInit('PUT', entry)),
    update: (id: string, entry: CalendarUpdateEntry) =>
      request<CalendarEntry>(
        `/calendar/api/entries/${encodeURIComponent(id)}`,
        jsonInit('PATCH', entry),
      ),
    delete: (id: string) =>
      request<void>(
        `/calendar/api/entries/${encodeURIComponent(id)}`,
        { method: 'DELETE' },
      ),
  },
  contexts: {
    list: (from: string, to: string) =>
      request<CalendarContext[]>(
        `/calendar/api/contexts?from=${encodeURIComponent(from)}&to=${encodeURIComponent(to)}`,
      ),
    create: (context: CalendarNewContext) =>
      request<CalendarContext>('/calendar/api/contexts', jsonInit('POST', context)),
    update: (id: string, context: CalendarUpdateContext) =>
      request<CalendarContext>(
        `/calendar/api/contexts/${encodeURIComponent(id)}`,
        jsonInit('PATCH', context),
      ),
    delete: (id: string) =>
      request<void>(
        `/calendar/api/contexts/${encodeURIComponent(id)}`,
        { method: 'DELETE' },
      ),
  },
  rhythms: {
    list: () => request<CalendarRhythm[]>('/calendar/api/rhythms'),
    get: (id: string) =>
      request<CalendarRhythm>(`/calendar/api/rhythms/${encodeURIComponent(id)}`),
    create: (rhythm: CalendarNewRhythm) =>
      request<{ rhythm: CalendarRhythm; instances_created: number }>(
        '/calendar/api/rhythms',
        jsonInit('POST', rhythm),
      ),
    update: (id: string, rhythm: CalendarUpdateRhythm) =>
      request<{ rhythm: CalendarRhythm; future_instances_affected: number }>(
        `/calendar/api/rhythms/${encodeURIComponent(id)}`,
        jsonInit('PATCH', rhythm),
      ),
    delete: (id: string, deleteInstances?: boolean) =>
      request<void>(
        `/calendar/api/rhythms/${encodeURIComponent(id)}${deleteInstances ? '?delete_instances=true' : ''}`,
        { method: 'DELETE' },
      ),
    materialize: (id: string) =>
      request<{ instances_created: number }>(
        `/calendar/api/rhythms/${encodeURIComponent(id)}/materialize`,
        { method: 'POST' },
      ),
  },
};

export const comms = {
  feed: (opts: { stream?: FeedStream; days?: number; includeDismissed?: boolean } = {}) => {
    const params = new URLSearchParams();
    if (opts.stream) params.set('stream', opts.stream);
    if (opts.days != null) params.set('days', String(opts.days));
    if (opts.includeDismissed) params.set('include_dismissed', 'true');
    const qs = params.toString();
    return request<FeedEntry[]>(`/comms/feed${qs ? `?${qs}` : ''}`);
  },
  entry: (id: string, signal?: AbortSignal) =>
    request<FeedEntryDetail>(
      `/comms/feed/${encodeURIComponent(id)}`,
      signal ? { signal } : undefined,
    ),
  /** Kept as the comms-shaped alias so existing comms callers read naturally;
   *  the routing itself lives in `contentItem` so there is one map, not two. */
  content: (source: ContentSource, id: string, signal?: AbortSignal) =>
    contentItem(source, id, signal),
  /** Generate or refine an item's digest. Synchronous on the server: this is a
   *  button the operator is watching, and answering early would show them the
   *  previous digest. */
  digest: (
    source: ContentSource,
    id: string,
    body: { depth?: DigestDepth; focus?: string[] } = {},
  ) =>
    request<ContentDigest>(
      `/comms/content/${source}/${encodeURIComponent(id)}/digest`,
      jsonInit('POST', body),
    ),
  chart: (source: ContentSource, id: string) =>
    request<ContentDigest>(
      `/comms/content/${source}/${encodeURIComponent(id)}/chart`,
      { method: 'POST' },
    ),
  diagram: (source: ContentSource, id: string) =>
    request<ContentDigest>(
      `/comms/content/${source}/${encodeURIComponent(id)}/diagram`,
      { method: 'POST' },
    ),
  refreshDigests: (source: ContentSource, limit?: number) =>
    request<{ source: string; digested: number }>(
      '/comms/content/digests/refresh',
      jsonInit('POST', limit === undefined ? { source } : { source, limit }),
    ),
  prepareCloudPreview: (source: ContentSource, id: string) =>
    request<CloudDerivativePreview>(
      `/comms/content/${source}/${encodeURIComponent(id)}/cloud-preview`,
      { method: 'POST' },
    ),
  approveCloudPreview: (source: ContentSource, id: string, preview_hash: string) =>
    request<CloudProcessingState>(
      `/comms/content/${source}/${encodeURIComponent(id)}/cloud-approval`,
      jsonInit('POST', { preview_hash }),
    ),
  cloudProviders: () => request<CloudProvider[]>('/comms/content/cloud-providers'),
  queueCloudDerivative: (
    source: ContentSource,
    id: string,
    preview_hash: string,
    provider_role: string,
  ) =>
    request<CloudProcessingState>(
      `/comms/content/${source}/${encodeURIComponent(id)}/cloud-queue`,
      jsonInit('POST', { preview_hash, provider_role }),
    ),
  runCloudJob: (jobId: string) =>
    request<CloudProcessingState>(
      `/comms/content/cloud-jobs/${encodeURIComponent(jobId)}/run`,
      jsonInit('POST', {}),
    ),
  runs: (days = 7) => request<FeedRun[]>(`/comms/feed/runs?days=${days}`),
  evaluationStatus: () =>
    request<CommsEvaluationStatus>('/comms/feed/evaluation/status'),
  qualityFlags: (limit = 500) =>
    request<FeedQualityFlag[]>(`/comms/feed/quality?limit=${limit}`),
  refreshQualityFlags: (days = 3650) =>
    request<FeedQualityRefresh>(
      '/comms/feed/quality/refresh',
      jsonInit('POST', { days }),
    ),
  // Answers once the item is stored; the summary is written behind the response, so a
  // freshly ingested entry legitimately comes back with `summary: null`.
  ingest: (url: string) => request<FeedEntryDetail>('/comms/ingest', jsonInit('POST', { url })),
  refreshRelevance: (days = 90) =>
    request<{
      scored: number;
      evaluated: number;
      considered: number;
      skipped_current: number;
      profile_count: number;
      mode: 'reranked' | 'semantic' | 'lexical' | null;
      evaluator_revision: string;
      travel_context: {
        upcoming_count: number;
        reachable: boolean;
        from_cache: boolean;
        refreshed_at: string;
      };
    }>(
      '/comms/feed/relevance/refresh',
      jsonInit('POST', { days }),
    ),
  scanVaultLinks: () =>
    request<VaultLinkCandidate[]>('/comms/vault-links/scan', { method: 'POST' }),
  sources: () => request<{ sources: FeedSource[] }>('/comms/sources'),
  scanSources: (source_id?: string) =>
    request<FeedSourceScan>(
      '/comms/sources/scan',
      jsonInit('POST', { source_id: source_id ?? null }),
    ),
  importVaultLink: (source_id: string, url: string) =>
    request<FeedEntryDetail>(
      '/comms/vault-links/import',
      jsonInit('POST', { source_id, url }),
    ),
  setStatus: (id: string, status: FeedStatus) =>
    request<void>(`/comms/feed/${encodeURIComponent(id)}/status`, jsonInit('POST', { status })),
  triage: (status?: TriageStatus) =>
    request<TriageItem[]>(
      `/comms/triage${status ? `?status=${encodeURIComponent(status)}` : ''}`,
    ),
  setTriageStatus: (id: string, status: 'proposed' | 'approved' | 'dismissed') =>
    request<void>(
      `/comms/triage/${encodeURIComponent(id)}/status`,
      jsonInit('POST', { status }),
    ),
  setTriageCategory: (id: string, stream: MailCategory) =>
    request<void>(
      `/comms/triage/${encodeURIComponent(id)}/stream`,
      jsonInit('POST', { stream }),
    ),
  setTriageDataClass: (id: string, data_class: DataClass) =>
    request<void>(
      `/comms/triage/${encodeURIComponent(id)}/data-class`,
      jsonInit('POST', { data_class }),
    ),
  applyGmailAction: (id: string, action: 'archive' | 'trash' | 'restore') =>
    request<{ ok: true; action: 'archive' | 'trash' | 'restore'; gmail_changed: boolean; gmail_confirmed: true }>(
      `/comms/triage/${encodeURIComponent(id)}/gmail`,
      jsonInit('POST', { action }),
    ),
  decideGmailJob: (id: string, decision: 'retry' | 'cancel') =>
    request<{ ok: true; state: 'completed' | 'canceled'; gmail_changed?: boolean }>(
      `/comms/triage/${encodeURIComponent(id)}/gmail-job`,
      jsonInit('POST', { decision }),
    ),
  reconcileGmail: () =>
    request<GmailMaintenanceResult>('/comms/triage/reconcile', jsonInit('POST', {})),
  triageSweepStatus: (signal?: AbortSignal) =>
    request<TriageSweepStatus>('/comms/triage/sweep/status', signal ? { signal } : undefined),
  sweepTriage: (limit = 100, cursor?: string | null) =>
    request<TriageSweepResult>(
      '/comms/triage/sweep',
      jsonInit('POST', { limit, cursor: cursor ?? null }),
    ),
  refreshTriageRelevance: (limit = 200) =>
    request<TriageRelevanceResult>(
      '/comms/triage/relevance/refresh',
      jsonInit('POST', { limit }),
    ),
  refreshTriageDataClasses: (limit = 500) =>
    request<TriageDataClassRefreshResult>(
      '/comms/triage/data-class/refresh',
      jsonInit('POST', { limit }),
    ),
  /** Remediate rows stored before the sweep redacted Private mail in place.
   *  Idempotent: a second run reports `changed: 0`. */
  redactTriage: (limit = 500, dryRun = false) =>
    request<TriageRedactResult>(
      '/comms/triage/redact',
      jsonInit('POST', { limit, dry_run: dryRun }),
    ),
  bulkTriage: (
    ids: string[],
    action:
      | 'dismiss'
      | 'categorize'
      | 'set-data-class'
      | 'archive'
      | 'trash'
      // The doctrine's one state label. It only labels — it does not archive,
      // and archiving does not set it.
      | 'waiting'
      | 'clear-waiting',
    stream?: MailCategory,
    data_class?: DataClass,
  ) =>
    request<TriageBulkResult>(
      '/comms/triage/bulk',
      jsonInit('POST', { ids, action, stream: stream ?? null, data_class: data_class ?? null }),
    ),
};

/** One thing to do, plus a way back to whatever said so.
 *
 *  `data_class` is inherited from the source rather than re-derived: a task
 *  promoted from a Private mail is Private, because the subject travelled into
 *  the title. */
export interface Task {
  id: string;
  title: string;
  status: TaskStatus;
  due: string | null;
  note: string | null;
  source_capability: string | null;
  source_id: string | null;
  source_url: string | null;
  data_class: DataClass;
  created_at: string;
  updated_at: string;
  completed_at: string | null;
}

export type TaskStatus = 'open' | 'done' | 'dropped';

export interface NewTask {
  title: string;
  due?: string | null;
  note?: string | null;
  source_capability?: string | null;
  source_id?: string | null;
  source_url?: string | null;
  data_class?: DataClass | null;
}

/** `created: false` means this source already owned a task — the expected
 *  result of promoting twice, not an error. */
export interface TaskCreated {
  task: Task;
  created: boolean;
}

export const tasks = {
  list: (status?: TaskStatus, signal?: AbortSignal) =>
    request<{ tasks: Task[] }>(
      `/tasks/api/tasks${status ? `?status=${status}` : ''}`,
      signal ? { signal } : undefined,
    ).then((response) => response.tasks),
  create: (task: NewTask) =>
    request<TaskCreated>('/tasks/api/tasks', jsonInit('POST', task)),
  /** Omit a field to leave it; pass `null` to clear it. */
  patch: (
    id: string,
    patch: { title?: string; status?: TaskStatus; due?: string | null; note?: string | null },
  ) => request<{ task: Task }>(`/tasks/api/tasks/${encodeURIComponent(id)}`, jsonInit('PATCH', patch)),
  counts: (signal?: AbortSignal) =>
    request<{ open: number; overdue: number }>(
      '/tasks/api/counts',
      signal ? { signal } : undefined,
    ),
};

// ─── Finance ─────────────────────────────────────────────────────────────────

export type BillingCycle = 'weekly' | 'monthly' | 'quarterly' | 'yearly' | 'one_off';
export type SubscriptionState = 'considering' | 'trial' | 'active' | 'paused' | 'cancelled';

/** Append-only. A price change adds one of these; it never edits the one before. */
export interface PricePoint {
  valid_from: string;
  amount_cents: number;
  currency: string;
  cycle: BillingCycle;
  /** Which tier: "Pro", "Max", "2TB". Absent for a subscription with only one. */
  plan?: string | null;
  reason: string;
}

/** Append-only, same reasoning. `paused → active` is two rows, not an edit. */
export interface StateChange {
  effective: string;
  state: SubscriptionState;
  note: string;
}

export interface Subscription {
  id: string;
  name: string;
  source_path: string;
  category: string | null;
  value_rating: number | null;
  prices: PricePoint[];
  states: StateChange[];
}

/** Computed from the series at a date, never read from a stored total. */
export interface Burn {
  at: string;
  monthly_cents: number;
  annual_cents: number;
  monthly: string;
  annual: string;
  billing_count: number;
  total_count: number;
}

export interface WritebackResult {
  ok: boolean;
  written: number;
  unchanged: number;
  conflicts: string[];
  not_imported: string[];
}

export type FinanceTransactionKind = 'income' | 'expense' | 'transfer';
export type CandidateState = 'pending' | 'confirmed' | 'rejected';

export interface TransactionCandidate {
  id: string;
  fingerprint: string;
  booked_at: string;
  description: string;
  amount_cents: number;
  currency: string;
  source_account: string;
  source_reference: string | null;
  proposed_account: string;
  confidence_basis_points: number;
  state: CandidateState;
}

export interface CsvMapping {
  delimiter: string;
  decimal_separator: string;
  date_column: string;
  amount_column: string;
  description_column: string;
  reference_column?: string | null;
  currency_column?: string | null;
  default_currency: string;
  source_account: string;
}

export interface CsvMappingProfile {
  label: string;
  mapping: CsvMapping;
}

export interface FinanceTransaction {
  id: string;
  date: string;
  description: string;
  kind: FinanceTransactionKind;
  account: string;
  category: string;
  amount_cents: number;
  currency: string;
}

export interface FinanceDashboard {
  summary: {
    income_cents: number;
    expense_cents: number;
    net_cash_flow_cents: number;
    savings_rate_percent: number | null;
    budget_cents: number;
    budget_variance_cents: number;
    currency: string;
  };
  trend: Array<{
    month: string;
    income_cents: number;
    expense_cents: number;
    net_cash_flow_cents: number;
  }>;
  budgets: Array<{
    account: string;
    budget_cents: number;
    actual_cents: number;
    variance_cents: number;
  }>;
  transactions: FinanceTransaction[];
  sankey: Array<{
    source: string;
    target: string;
    amount_cents: number;
    account: string;
    category: string;
  }>;
  accounts: string[];
  categories: string[];
}

export const finance = {
  subscriptions: (signal?: AbortSignal) =>
    request<Subscription[]>('/finance/api/subscriptions', signal ? { signal } : undefined),
  /** `at` is the whole point: the price series makes "what will this cost in
   *  October" a different answer from "what does it cost today". */
  burn: (at?: string, signal?: AbortSignal) =>
    request<Burn>(
      `/finance/api/subscriptions/burn${at ? `?at=${encodeURIComponent(at)}` : ''}`,
      signal ? { signal } : undefined,
    ),
  appendPrice: (id: string, price: PricePoint) =>
    request<{ ok: boolean; id: string; created: boolean }>(
      `/finance/api/subscriptions/${encodeURIComponent(id)}/price`,
      jsonInit('POST', price),
    ),
  appendState: (id: string, change: StateChange) =>
    request<{ ok: boolean; id: string; created: boolean }>(
      `/finance/api/subscriptions/${encodeURIComponent(id)}/state`,
      jsonInit('POST', change),
    ),
  importVault: () =>
    request<{ ok: boolean; created: number; already_present: number }>(
      '/finance/api/import/obsidian',
      { method: 'POST' },
    ),
  /** Conflicts come back named, never resolved — the caller shows them. */
  writeback: () => request<WritebackResult>('/finance/api/writeback', { method: 'POST' }),
  dashboard: (filters: {
    start?: string;
    end?: string;
    account?: string;
    category?: string;
    currency?: string;
  } = {}, signal?: AbortSignal) => {
    const query = new URLSearchParams();
    for (const [key, value] of Object.entries(filters)) if (value) query.set(key, value);
    return request<FinanceDashboard>(
      `/finance/api/dashboard${query.size ? `?${query}` : ''}`,
      signal ? { signal } : undefined,
    );
  },
  rebuildLedger: () =>
    request<{ ok: boolean; rows: number }>('/finance/api/ledger/rebuild', { method: 'POST' }),
  csvMappings: () =>
    request<CsvMappingProfile[]>('/finance/api/import/csv/mappings'),
  candidates: () => request<TransactionCandidate[]>('/finance/api/import/candidates'),
  importCsv: (content: string, mapping: CsvMapping) =>
    request<{ ok: boolean; created: number; already_present: number }>(
      '/finance/api/import/csv',
      jsonInit('POST', { content, mapping }),
    ),
  reviewCandidate: (id: string, decision: 'confirm' | 'reject', account?: string) =>
    request<{ ok: boolean; id: string; state: CandidateState; journal_written: boolean }>(
      `/finance/api/import/candidates/${encodeURIComponent(id)}/review`,
      jsonInit('POST', { decision, account }),
    ),
};
