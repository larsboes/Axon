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

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const res = await fetch(path, init);
  if (!res.ok) {
    const body = await res.text().catch(() => '');
    throw new ApiError(res.status, errorMessage(body) || `Request failed (${res.status})`);
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

export const axonStatus = {
  health: () => request<AxonStatusHealth>('/axon-status/api/axon-status/health'),
  capabilities: () => request<CapabilityView[]>('/axon-status/api/axon-status/capabilities'),
  self: () => request<SelfModelResponse>('/axon-status/api/axon-status/self'),
  repos: () => request<{ repos: RepoStatus[] }>('/axon-status/api/axon-status/repos'),
  upstreams: () => request<UpstreamAudit>('/axon-status/api/axon-status/upstreams'),
  start: (name: string) =>
    request<{ name: string; up: boolean; detail: string }>(
      `/axon-status/api/axon-status/capabilities/${encodeURIComponent(name)}/start`,
      { method: 'POST' },
    ),
  stop: (name: string) =>
    request<{ name: string; up: boolean; detail: string }>(
      `/axon-status/api/axon-status/capabilities/${encodeURIComponent(name)}/stop`,
      { method: 'POST' },
    ),
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

export interface TriageItem {
  id: string;
  from_addr: string;
  subject: string;
  snippet: string;
  stream: FeedStream;
  rationale: string;
  status: string;
  internal_date: string | null;
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
  entry: (id: string) => request<FeedEntryDetail>(`/comms/feed/${encodeURIComponent(id)}`),
  runs: (days = 7) => request<FeedRun[]>(`/comms/feed/runs?days=${days}`),
  evaluationStatus: () =>
    request<CommsEvaluationStatus>('/comms/feed/evaluation/status'),
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
  triage: (status = 'proposed') =>
    request<TriageItem[]>(`/comms/triage?status=${encodeURIComponent(status)}`),
};
