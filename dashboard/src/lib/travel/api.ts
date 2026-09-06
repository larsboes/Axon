/**
 * Travel route functions that are not in the shared client.
 *
 * `dashboard/README.md` says `src/lib/api.ts` is the one place that knows both
 * upstream shapes and error shapes; the parallel-work rule says new route
 * functions go in a per-domain module. Both are kept: `request` and `jsonInit`
 * are imported from the shared client rather than copied, because
 * `request` also unwraps a 200 whose body carries `{"error": …}` — that is a
 * contract, not boilerplate, and a second copy of it is a second place for it
 * to drift.
 */
import { request, jsonInit } from '$lib/api';
import type { PlaceRef, TransportMode } from '$lib/api';

export interface DateWindow {
  from: string;
  /** Inclusive, like the trips route's own `date_to`. */
  to: string;
}

export interface PlanSearchRequest {
  origin: PlaceRef;
  /** `YYYY-MM`. Exactly one of `month` and `date_window`. */
  month?: string;
  date_window?: DateWindow;
  min_days?: number;
  budget_cents?: number;
  currency?: string;
  modes?: TransportMode[];
  interests?: string;
  max_candidates?: number;
}

/** One visible reason behind a candidate's score. */
export interface ScoreFactor {
  key: string;
  label: string;
  score: number;
  /** Re-normalised across the factors that could be measured; they sum to 1. */
  weight: number;
  rationale: string;
}

export interface FeasibleWindow {
  starts_on: string;
  /** Exclusive, as calendar serves it. */
  ends_before: string;
  days: string[];
  verdict: string;
  days_needing_travel_day: string[];
}

export interface CandidateEvent {
  id: string;
  title: string;
  starts_at: string | null;
  url: string;
  distance_km: number | null;
}

export interface RankedCandidate {
  destination: PlaceRef;
  place_id: string;
  window: FeasibleWindow;
  mode: TransportMode;
  estimated_cost_cents: number | null;
  currency: string;
  cost_basis: string;
  priced_at: string | null;
  events: CandidateEvent[];
  season: { month: number; score: number; best_month: number | null } | null;
  /** `null` when no factor could be measured — not a measured zero. */
  score: number | null;
  factors: ScoreFactor[];
  /**
   * Name-free by construction: places answers a count and an overlap, and trips
   * turns that into a sentence. It reaches this page and never the plan.
   */
  companion_hint: string | null;
  why: string[];
}

/** `"ok"`, `"absent"` or `"error: <reason>"` per upstream. */
export interface PlanSearchReach {
  calendar: string;
  places: string;
  transit: string;
  scouting: string;
  climate: string;
}

export interface PlanSearchResult {
  revision: string;
  window_source: 'calendar' | 'caller';
  windows: FeasibleWindow[];
  reach: PlanSearchReach;
  degraded: string[];
  considered: number;
  priced: number;
  unpriced: number;
  candidates: RankedCandidate[];
  observed_at: string;
}

export type PlanSearchJob =
  | { id: number; state: 'running'; since_ms: number }
  | { id: number; state: 'done'; result: PlanSearchResult }
  | { id: number; state: 'failed'; error: string };

export const planSearch = {
  start: (body: PlanSearchRequest) =>
    request<{ job: number }>('/trips/api/plan-search', jsonInit('POST', body)),
  status: (job: number, signal?: AbortSignal) =>
    request<PlanSearchJob>(
      `/trips/api/plan-search/${encodeURIComponent(String(job))}`,
      signal ? { signal } : undefined,
    ),
  /** Writes one `option_set` item onto a plan that already exists. */
  adopt: (job: number, planId: string) =>
    request<{ id: string; item_type: string; external_id: string }>(
      `/trips/api/plan-search/${encodeURIComponent(String(job))}/adopt`,
      jsonInit('POST', { plan_id: planId }),
    ),
};
