// The Feed domain's HTTP client.
//
// One client module per domain, under `$lib/<domain>/api.ts`. `src/lib/api.ts` is
// 3472 lines and is edited by several streams at once, so a domain that needs two
// new calls adds a file instead of a hunk. Error shaping stays in `$lib/api`:
// `ApiError` and `describeFailure` are imported, never re-implemented, so a reader
// gets the same sentence whichever module made the call.
//
// `request` and `jsonInit` are module-private in api.ts, which is why the ten
// lines below exist rather than an import.

import {
  ApiError,
  describeFailure,
  type CommsEvaluationStatus,
  type FeedStatus,
  type TriageItem,
} from '$lib/api';

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

/** Where a press happened. The server's CHECK constraint holds the same list. */
export type InteractionSurface = 'inbox' | 'reader' | 'library' | 'home' | 'cli' | 'api';

/**
 * The two verbs a client may write.
 *
 * `kept`, `dismissed` and `unkept` are refused by the route with a 400 naming
 * `POST /feed/:id/status`, which writes them inside the same transaction as the
 * status change. Two paths writing one decision would double every count the
 * learned factor is gated on.
 */
export type ClientInteraction = 'opened' | 'reopened';

/** What the last relevance pass did, and in which mode it actually answered. */
export interface FeedLastPass {
  mode: string | null;
  at: string;
  considered: number;
  written: number;
  error_class: string | null;
  consecutive_fallbacks: number;
  completed_revision: string;
}

/** The learned factor's state, reported whether or not it is active. */
export interface FeedbackModelStatus {
  revision: string;
  active: boolean;
  gate_reason: string;
  samples: {
    kept: number;
    dismissed: number;
    total: number;
    seeded_from_status: number;
    skipped_class: number;
  };
  holdout: { n: number; auc: number };
  thresholds: { min_labels: number; min_minority: number; min_auc: number; weight: number };
  trained_at: string;
  feature_count: number;
  top_features?: Array<{ name: string; weight: number }>;
}

/**
 * Fields `GET /feed/evaluation/status` gained in this stream.
 *
 * Declared here rather than on `CommsEvaluationStatus`: every member is optional,
 * so a plain `CommsEvaluationStatus` stays assignable to the intersection and the
 * second consumer of `ModelStatus.svelte` compiles unchanged.
 */
export interface FeedStatusExtras {
  last_pass: FeedLastPass | null;
  feedback_model: FeedbackModelStatus | null;
}

export interface RelevanceRefreshResult {
  scored: number;
  evaluated: number;
  considered: number;
  skipped_current: number;
  rescored: number;
  reused_relevance: number;
  refused_class: number;
  refused_lower_tier: number;
  /** Stored matches actually deleted from refused items on this pass. */
  refused_matches_cleared: number;
  missing_ids: string[];
  profile_count: number;
  mode: string | null;
  offset: number;
  limit: number;
  has_more: boolean;
  relevance_revision: string;
  evaluator_revision: string;
  embedding: {
    mode: string;
    error_class: string | null;
    chunks: number;
    chunks_failed: number;
  };
}

/**
 * A mail proposal carrying the mail evaluator's score.
 *
 * `score_bp` is basis points, 0..=10000, and `null` when the mail has no stored
 * evaluation — which is deliberately not the same as 0. One writer: comms'
 * mail evaluator. Declared here as an interface extension rather than added to
 * `TriageItem` in `$lib/api.ts`, so the two can be merged in either order; when
 * that field lands on `TriageItem` this collapses to a re-export.
 */
export interface TriageItemScored extends TriageItem {
  score_bp: number | null;
  evaluated_at: string | null;
}

export const feedPersonalization = {
  /**
   * Keep, dismiss or retract an entry, saying where the press happened.
   *
   * This route is the ONLY writer of `kept`, `dismissed` and `unkept`: the
   * server writes the ledger row inside the same transaction as the status
   * change. `comms.setStatus` in `$lib/api` sends no surface and so records
   * `api`; this is the call a surface that knows its own name should use.
   */
  setStatus: (id: string, status: FeedStatus, surface: InteractionSurface) =>
    request<void>(
      `/comms/feed/${encodeURIComponent(id)}/status`,
      jsonInit('POST', { status, surface }),
    ),

  /**
   * Record that an entry was opened. Fire-and-forget on purpose: the published
   * demo answers 403 to any POST, and a failed telemetry write must never be
   * what stops a reader opening an article.
   */
  recordInteraction: (id: string, event: ClientInteraction, surface: InteractionSurface) =>
    request<{ ok: boolean; recorded: string }>(
      `/comms/feed/${encodeURIComponent(id)}/interactions`,
      jsonInit('POST', { event, surface }),
    ).catch(() => undefined),

  /** The status endpoint, typed with the two blocks this stream added. */
  evaluationStatus: () =>
    request<CommsEvaluationStatus & FeedStatusExtras>('/comms/feed/evaluation/status'),

  /** Retrain the learned factor. `dry_run` reports the gate without writing. */
  trainFeedbackModel: (dryRun = false) =>
    request<FeedbackModelStatus>('/comms/feed/model/train', jsonInit('POST', { dry_run: dryRun })),

  refreshRelevance: (body: {
    days?: number;
    limit?: number;
    offset?: number;
    ids?: string[];
    force?: boolean;
  }) => request<RelevanceRefreshResult>('/comms/feed/relevance/refresh', jsonInit('POST', body)),
};
