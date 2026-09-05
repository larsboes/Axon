/**
 * The pure half of the plan-search surface: polling and labels.
 *
 * Kept out of the component so the page renders and does not compute, and so
 * `dashboard/vite/plan-search.test.ts` can drive it with an injected fetcher.
 * The bug class this exists to make testable is a poll that never stops —
 * against a job number the server has already evicted, a naive loop asks
 * forever.
 */
import type { PlanSearchJob, PlanSearchResult, RankedCandidate, ScoreFactor } from './api';

/** How often the panel asks. Slow enough not to be a load, fast enough to feel live. */
export const POLL_INTERVAL_MS = 1500;

/**
 * When the poller gives up.
 *
 * The server's own budget is `JOB_DEADLINE_S = 180` measured from job start
 * (`capabilities/trips/src/jobs.rs`), and a job that spends it still finishes
 * `done`. This is that plus the slack for one last poll, so the page stops
 * after the server would have, never before.
 */
export const POLL_DEADLINE_MS = 195_000;

export interface PollTimedOut {
  id: number;
  state: 'timeout';
  error: string;
}

export type PollOutcome = PlanSearchJob | PollTimedOut;

export interface PollOptions {
  fetchStatus: (job: number) => Promise<PlanSearchJob>;
  /** Injected so a test does not wait in real time. */
  sleep?: (ms: number) => Promise<void>;
  now?: () => number;
  intervalMs?: number;
  deadlineMs?: number;
  /** Called after every answer, so the panel can show `since_ms`. */
  onUpdate?: (job: PlanSearchJob) => void;
}

const realSleep = (ms: number) => new Promise<void>((resolve) => setTimeout(resolve, ms));

/**
 * Ask until the job is `done` or `failed`, or until the deadline passes.
 *
 * A rejected fetch ends the poll rather than retrying: the shared client turns
 * a 404 into an `ApiError` carrying the server's own "that search result has
 * expired" sentence, and retrying an expired job is exactly the loop that
 * never stops.
 */
export async function pollJob(job: number, options: PollOptions): Promise<PollOutcome> {
  const sleep = options.sleep ?? realSleep;
  const now = options.now ?? (() => Date.now());
  const interval = options.intervalMs ?? POLL_INTERVAL_MS;
  const deadline = now() + (options.deadlineMs ?? POLL_DEADLINE_MS);

  for (;;) {
    const answer = await options.fetchStatus(job);
    options.onUpdate?.(answer);
    if (answer.state === 'done' || answer.state === 'failed') return answer;
    if (now() >= deadline) {
      return {
        id: job,
        state: 'timeout',
        error: 'the search is still running after three minutes — open it again in a moment',
      };
    }
    await sleep(interval);
  }
}

/** Integer minor units to a readable amount. The server does the arithmetic. */
export function money(cents: number | null, currency: string): string {
  if (cents === null) return 'no fare found';
  const whole = Math.trunc(cents / 100);
  const rest = Math.abs(cents % 100);
  return `${whole}.${String(rest).padStart(2, '0')} ${currency}`;
}

/** A factor's weight as a percentage, for the bar's width. */
export const factorPercent = (factor: ScoreFactor): number =>
  Math.round(Math.max(0, Math.min(1, factor.score)) * 100);

/**
 * What could not be measured, in one sentence.
 *
 * The scores in a degraded result are comparable to each other and not to a
 * result with a different `degraded` set — saying so is the whole reason the
 * field is on the response.
 */
export function degradedNotice(result: PlanSearchResult): string | null {
  if (result.degraded.length === 0) return null;
  const named = result.degraded.join(', ');
  return `Scored without ${named}. The ranking is comparable inside this result and not against another one.`;
}

/** `considered / priced / unpriced`, said in words rather than three numbers. */
export function coverageNotice(result: PlanSearchResult): string {
  const unpriced = result.unpriced > 0 ? `, ${result.unpriced} with no fare` : '';
  return `${result.considered} destinations considered, ${result.priced} priced${unpriced}.`;
}

/** The upstreams that answered with an error, for the reach line. */
export function unreachable(result: PlanSearchResult): string[] {
  return Object.entries(result.reach)
    .filter(([, state]) => state !== 'ok')
    .map(([name, state]) => `${name} (${state})`);
}

/**
 * The fields the New-trip form needs to open on a chosen candidate.
 *
 * The same hand-off `seedPlanFromCandidate` already makes for a scouting
 * opportunity: the panel proposes, the operator submits the form.
 */
export function seedFromCandidate(candidate: RankedCandidate): {
  destination: { id: string; name: string; latitude: number | null; longitude: number | null };
  startDate: string;
  endDate: string;
} {
  return {
    destination: {
      id: candidate.destination.id,
      name: candidate.destination.name,
      latitude: candidate.destination.latitude ?? null,
      longitude: candidate.destination.longitude ?? null,
    },
    startDate: candidate.window.starts_on,
    // The window end is exclusive on the wire and inclusive in a date input.
    endDate: previousDay(candidate.window.ends_before),
  };
}

/** One day before an ISO date. Formatting, not arithmetic on a measurement. */
export function previousDay(iso: string): string {
  const parsed = new Date(`${iso}T00:00:00Z`);
  if (Number.isNaN(parsed.getTime())) return iso;
  parsed.setUTCDate(parsed.getUTCDate() - 1);
  return parsed.toISOString().slice(0, 10);
}
