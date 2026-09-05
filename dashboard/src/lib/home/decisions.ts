/**
 * The Home ladder's contract: what a decision kind is, and how two of them are ordered.
 *
 * This module imports NOTHING at runtime, on purpose. `registry.ts` beside it is the only
 * half that needs Vite (`import.meta.glob`), so this file — and every `kinds/<key>.ts` that
 * implements the interface — stays readable by plain `bun test`. That is the same rule
 * `nav.ts:14-19` follows for `$app/paths`, and the reason `DecisionKind.view` is the NAME
 * of a row component rather than an imported component: a name costs nothing to test, an
 * import drags the Svelte compiler into every unit test that touches a kind.
 *
 * PRD §8.1 (lines 2207-2220) is the law this file implements. The dashboard owns no tables
 * (dashboard/README.md:7-9), so this interface is the durable contract in a schema's place.
 */

/** The four data classes. c2 and c3 never leave the host and never reach a cloud model. */
export type DataClass = "c0" | "c1" | "c2" | "c3";

export interface ScoreContext {
  /** Local YYYY-MM-DD. Recomputed at local midnight — a tab left open overnight used to
   *  keep yesterday's `today` and rank every date one day too urgent. */
  todayKey: string;
  /** `todayKey` plus four months, the window the calendar reads. */
  horizonEndKey: string;
  nowMs: number;
  /** Whole days from today. Negative in the past. 365 for an unparseable value. */
  daysUntil(value: string): number;
  /**
   * Another kind's rows, for a kind that declares that kind in `dependsOn`.
   *
   * Returns `[]` when the dependency has not settled or its capability failed, so a kind
   * that reads a peer must still produce a sensible row from nothing. `opportunity` needs
   * this: its gate and both of its rank adjustments read calendar entries and contexts.
   */
  peer<Row>(key: string): readonly Row[];
}

export interface LoadContext extends ScoreContext {
  /** Aborted when Home unmounts or reloads. Clients that accept a signal are handed it;
   *  the rest are simply not awaited into state once this is aborted. */
  signal: AbortSignal;
}

export interface DecisionViewProps<Row> {
  row: Row;
  /** True while this row's own action is in flight. */
  busy: boolean;
  /**
   * Runs a capability write and, only once it RESOLVES, drops the row from the ladder.
   *
   * The order is the point. An optimistic dismissal shows a decision as made that the
   * capability never recorded — the dashboard contradicting the owner of the record. On
   * rejection the row stays and the error surfaces. See +page.svelte:430-437 for the shape
   * this generalises.
   */
  act(run: () => Promise<void>, options?: { dismiss?: boolean }): void;
}

export interface DecisionKind<Source = unknown, Row = unknown> {
  /** Matches the filename under `home/kinds/`, and prefixes every row key. */
  key: string;
  /** PRD §8.1's own number. NOT unique — see the band table below. */
  band: number;
  /** The name used in the "Unavailable: …" line when `load` rejects. */
  label: string;
  /** The capability this kind reads, or null for axon-status itself. */
  capability: string | null;
  /** Commitments expire whether or not you look at them; reading never does. */
  lane?: "commitment" | "reading";
  /** Kind keys whose rows this kind reads through `ctx.peer()`. Rows are held back until
   *  each has settled; a failed dependency yields `[]` and `whyHere` names it. */
  dependsOn?: readonly string[];
  /** A component filename under `home/rows/`, e.g. `"MailRow"`. A STRING, not an import:
   *  `registry.ts` resolves it, so this module and every kind stay bun-importable. */
  view: string;

  load(ctx: LoadContext): Promise<Source>;
  /** The gate lives here: a source row that earns no decision is simply not returned. */
  rows(source: Source, ctx: ScoreContext): Row[];
  id(row: Row): string;
  /** 0..999. `score()` clamps, so a kind may return an unbounded expression. */
  urgency(row: Row, ctx: ScoreContext): number;
  /** `link(...)` for an internal route, or an absolute URL with `external` set. */
  href(row: Row): string;
  external?(row: Row): boolean;

  // --- the attention contract, field names adopted from the private attention policy ---

  /** Why this row is here, in the row's own words. For a model-ranked kind this is the
   *  model's own rationale, never a restatement of the subject (PRD:227). */
  whyHere(row: Row, ctx: ScoreContext): string;
  /** ISO date or datetime, the first declared tie-breaker. null sorts last. */
  startOrDueAt(row: Row): string | null;
  candidateStatus(row: Row): "proposed" | "accepted" | "open";
  /** null wherever the capability publishes no class. NEVER computed here: the dashboard
   *  owns no data, so inventing a class would be a false provenance claim. */
  dataClass(row: Row): DataClass | null;
  /** null on every kind today. The gap is the capabilities', not this layer's. */
  processingRoute(row: Row): "local" | "cloud" | null;
  scoreFactors?(row: Row, ctx: ScoreContext): { label: string; score: number }[];
}

export interface Decision<Row = unknown> {
  /** `${kind.key}:${kind.id(row)}` — stable across polls, which is what `dismissed` keys on. */
  key: string;
  kind: DecisionKind<unknown, Row>;
  row: Row;
  priority: number;
  startOrDueAt: string | null;
}

/**
 * The urgency resolution budget: three significant digits.
 *
 * NOT a guard against crossing the tightest band gap. Clamping is what makes the band
 * decisive — once urgency cannot reach the stride, `(b-1)·S + (S-1) < b·S` holds for every
 * S >= 1, including PRD's tightest 640/630/620 run. The stride only decides how finely
 * urgency can rank rows INSIDE one band.
 */
export const BAND_STRIDE = 1000;
export const MAX_URGENCY = BAND_STRIDE - 1;

export const score = (band: number, urgency: number): number =>
  band * BAND_STRIDE + Math.min(MAX_URGENCY, Math.max(0, Math.round(urgency)));

/** Shape-preserving rescale of a kind's own expression onto 0..999. */
export const RESCALE = (value: number, max: number): number =>
  Math.min(MAX_URGENCY, Math.max(0, Math.round((MAX_URGENCY * value) / max)));

/** Earlier first; a row with no date sorts last. */
export const compareStartOrDue = (a: string | null, b: string | null): number => {
  if (a === b) return 0;
  if (a === null) return 1;
  if (b === null) return -1;
  return a < b ? -1 : 1;
};

/**
 * Highest priority first, then the two declared tie-breakers in order.
 *
 * Ties are routine rather than rare once urgency is clamped: every undated task lands on
 * the same number and so does every unscored mail. Without a declared tie-break, intra-band
 * order was whatever the capability happened to return that poll, and the ladder visibly
 * reshuffled between fifteen-second refreshes.
 */
export const compareDecisions = (a: Decision, b: Decision): number =>
  b.priority - a.priority ||
  compareStartOrDue(a.startOrDueAt, b.startOrDueAt) ||
  (a.key < b.key ? -1 : a.key > b.key ? 1 : 0);

/**
 * The band table. PRD §8.1's own numbers, plus 610 for the trip retrospective (§8.2).
 *
 * band  | key         | owner stream       | PRD row
 * ------|-------------|--------------------|--------------------------------------
 * 10000 | system      | dashboard-refresh  | System health
 *   900 | host        | dashboard-refresh  | §9 resource rule broken
 *   800 | trip        | dashboard-refresh  | Trip needing planning
 *   700 | calendar    | dashboard-refresh  | Calendar `possible`
 *   640 | finance     | finance-invest     | Purchase decision      (SHARED)
 *   640 | (open)      | —                  | purchase renewal / wishlist  PRD:2213
 *   640 | (open)      | —                  | budget overrun               PRD:3115
 *   630 | (open)      | —                  | stalled project              PRD:2214
 *   620 | task        | dashboard-refresh  | Task
 *   610 | retro       | travel-season-cost | ADDITION, PRD §8.2
 *   600 | opportunity | dashboard-refresh  | Opportunity
 *   550 | mail        | dashboard-refresh  | Mail
 *   540 | (open)      | —                  | person contact frequency     PRD:2218
 *   500 | feed        | dashboard-refresh  | Feed item
 *   490 | (open)      | —                  | note due for review          PRD:2220
 *
 * Bands are NOT unique. PRD:2213 gives 640 to the purchase decision and PRD:3115 routes a
 * doubled month of metered spend to the same band, so a band is a rank and not a slot. A
 * kind that joins an occupied band is correct; a kind that invents a band the table does
 * not name has to say which PRD row it extends, and `tools/dashboard-home-registry.test.ts`
 * asks for exactly that.
 */
export const PRD_BANDS: readonly number[] = [
  10_000, 900, 800, 700, 640, 630, 620, 610, 600, 550, 540, 500, 490,
];

/** The visual spine. Four tones over thirteen bands, because a reader distinguishes four. */
export type BandTone = "alarm" | "now" | "owed" | "offer";

export const bandTone = (band: number): BandTone =>
  band >= 900 ? "alarm" : band >= 700 ? "now" : band >= 610 ? "owed" : "offer";

export const bandLabel = (band: number): string =>
  band >= 900 ? "Needs attention now" : band >= 700 ? "Dated commitments" : band >= 610 ? "Owed" : "Offered";
