/**
 * The Home ladder's "trip closed, retrospective open" decision kind.
 *
 * Band 610, an ADDITION to PRD §8.1's table, ruled by §8.2: "when a trip
 * closes, the dashboard raises one decision with three fields". It sits between
 * `task` (620) and `opportunity` (600) — a closed trip is a small, bounded thing
 * to answer, and it is never allowed to outrank a task that is actually due.
 *
 * The 45-day expiry is NOT here. It lives in
 * `capabilities/trips/src/store.rs::pending_retrospectives`, which is the route
 * this loader calls, so the rule has exactly one home: a retrospective filled
 * from a memory that is gone is a fabrication, and that judgement belongs to the
 * capability that owns the record.
 *
 * Relative imports throughout, so this module resolves under plain `bun test` —
 * the reason `tools/dashboard-nav-links.test.ts` records for `nav.ts`.
 */
import { link } from "../../nav";
import {
  pendingRetrospectives,
  type PendingRetrospective,
  type PendingRetrospectives,
} from "../../travel/api";

/*
 * MERGE NOTE, 2026-09-05: `decisions.ts` in this directory's parent is owned and created by
 * the dashboard-refresh stream and did not exist when this file was written, so
 * the contract it publishes is mirrored below rather than imported. It is the
 * same vocabulary with the same field names, not a second one.
 *
 * The whole merge is this: delete the block between the two markers and put
 *
 *   import type { DecisionKind, LoadContext, ScoreContext, DataClass } from "../decisions";
 *
 * in its place. Nothing else in this file changes.
 */
// ── mirrored contract, replace with the import above ────────────────────────
type DataClass = "c0" | "c1" | "c2" | "c3";

interface ScoreContext {
  todayKey: string;
  horizonEndKey: string;
  nowMs: number;
  daysUntil(value: string): number;
  peer<Row>(key: string): readonly Row[];
}

interface LoadContext extends ScoreContext {
  signal: AbortSignal;
}

interface DecisionKind<Source = unknown, Row = unknown> {
  key: string;
  band: number;
  label: string;
  capability: string | null;
  lane?: "commitment" | "reading";
  dependsOn?: readonly string[];
  view: string;
  load(ctx: LoadContext): Promise<Source>;
  rows(source: Source, ctx: ScoreContext): Row[];
  id(row: Row): string;
  urgency(row: Row, ctx: ScoreContext): number;
  href(row: Row): string;
  external?(row: Row): boolean;
  whyHere(row: Row, ctx: ScoreContext): string;
  startOrDueAt(row: Row): string | null;
  candidateStatus(row: Row): "proposed" | "accepted" | "open";
  dataClass(row: Row): DataClass | null;
  processingRoute(row: Row): "local" | "cloud" | null;
  scoreFactors?(row: Row, ctx: ScoreContext): { label: string; score: number }[];
}
// ── end mirrored contract ───────────────────────────────────────────────────

/** Used only when the response omits the window, which the route does not do.
 *  The number that matters is `source.window_days`, read below and carried onto
 *  each row — `urgency` is handed a row and no source, so the value has to
 *  travel with the row or it is restated here and drifts. */
const FALLBACK_WINDOW_DAYS = 45;

/** A pending row plus the window it was measured against. */
type PendingRow = PendingRetrospective & { window_days: number };

const kind: DecisionKind<PendingRetrospectives, PendingRow> = {
  key: "trip-retrospective",
  band: 610,
  label: "Trip retrospectives",
  capability: "trips",
  lane: "commitment",
  view: "TripRetrospectiveRow",

  // The route takes no parameters at all: what the ladder should raise today is
  // a judgement the capability owns, not one the caller narrows.
  load: () => pendingRetrospectives(),

  rows: (source: PendingRetrospectives) =>
    source.pending.map((row) => ({
      ...row,
      // From the response, not from a constant here: the 45 days live in
      // `capabilities/trips/src/store.rs::pending_retrospectives` and changing
      // them there must move this ramp with them.
      window_days: source.window_days > 0 ? source.window_days : FALLBACK_WINDOW_DAYS,
    })),

  id: (row: PendingRow) => row.plan_id,

  /**
   * Rises as the window runs out, saturating at its end.
   *
   * The shape is deliberate: a trip that closed yesterday is worth asking about
   * and is not urgent, while one at day 44 is the last chance before the prompt
   * expires and the answer stops being a memory.
   */
  urgency: (row: PendingRow) => {
    const window = row.window_days;
    const elapsed = Math.max(0, Math.min(row.days_since_close, window));
    return Math.round((999 * elapsed) / window);
  },

  href: (row: PendingRow) => link(`/travel?plan=${encodeURIComponent(row.plan_id)}`),

  whyHere: (row: PendingRow) =>
    row.destinations.length > 0
      ? `Back from ${row.destinations.join(", ")} ${row.days_since_close} days ago; no retrospective yet`
      : `Closed ${row.days_since_close} days ago; no retrospective yet`,

  /** The trip's end date: the moment that made this a decision. */
  startOrDueAt: (row: PendingRow) => row.date_end,

  candidateStatus: () => "open",

  /**
   * Null, and never computed here. `trips_plans` carries no class column, and
   * inventing one in the dashboard would be a false provenance claim from a
   * layer that owns no data. The `change_note` a retrospective collects is C1 by
   * default and C2 when it names a person, which is a claim for trips to make
   * with a mechanism behind it, not for this row to assert.
   */
  dataClass: () => null,

  processingRoute: () => null,
};

export default kind;
