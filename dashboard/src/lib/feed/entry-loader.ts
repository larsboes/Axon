import type { ContentItemDetail, FeedEntryDetail } from "../api";

export type FeedEntryLoadPhase = "reading" | "starting" | "retrying";

interface FeedEntryLoader<T> {
  id: string;
  read: (id: string, signal: AbortSignal) => Promise<T>;
  start: (signal: AbortSignal) => Promise<unknown>;
  shouldRetry?: (error: unknown) => boolean;
  onPhase?: (phase: FeedEntryLoadPhase) => void;
  readTimeoutMs?: number;
  startTimeoutMs?: number;
}

const DEFAULT_READ_TIMEOUT_MS = 3_000;
const DEFAULT_START_TIMEOUT_MS = 35_000;

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

/**
 * A dashboard can stay open while Comms is upgraded independently. Older
 * processes do not return the additive metadata arrays yet, so give the
 * reader the same empty-array meaning instead of letting rendering fail.
 */
export function normalizeFeedEntryDetail(entry: FeedEntryDetail): FeedEntryDetail {
  return {
    ...entry,
    relevance: entry.relevance ?? [],
    processing: entry.processing ?? [],
    origins: entry.origins ?? [],
  };
}

export function normalizeContentItemDetail(entry: ContentItemDetail): ContentItemDetail {
  return {
    ...entry,
    cloud_processing: entry.cloud_processing ?? {
      status: "not_prepared",
      preview_hash: null,
      approved_at: null,
      dispatch_status: "not_queued",
      job_id: null,
      provider_role: null,
      queued_at: null,
      provider_calls: 0,
      task: null,
      started_at: null,
      completed_at: null,
      last_error: null,
      result: null,
    },
    relevance: entry.relevance ?? [],
    processing: entry.processing ?? [],
    origins: entry.origins ?? [],
    // Added with the calendar source. A capability that has not shipped it yet
    // returns neither field, and the reader treats that as "no links" rather
    // than failing — same additive contract as the arrays above.
    links: entry.links ?? [],
    mail: entry.mail ?? null,
    calendar: entry.calendar ?? null,
  };
}

async function withDeadline<T>(
  operation: (signal: AbortSignal) => Promise<T>,
  timeoutMs: number,
  timeoutMessage: string,
): Promise<T> {
  const controller = new AbortController();
  let timedOut = false;
  const timer = setTimeout(() => {
    timedOut = true;
    controller.abort();
  }, timeoutMs);

  try {
    return await operation(controller.signal);
  } catch (error) {
    if (timedOut) throw new Error(timeoutMessage);
    throw error;
  } finally {
    clearTimeout(timer);
  }
}

export async function loadFeedEntry<T>({
  id,
  read,
  start,
  shouldRetry = () => true,
  onPhase = () => undefined,
  readTimeoutMs = DEFAULT_READ_TIMEOUT_MS,
  startTimeoutMs = DEFAULT_START_TIMEOUT_MS,
}: FeedEntryLoader<T>): Promise<T> {
  onPhase("reading");
  try {
    return await withDeadline(
      (signal) => read(id, signal),
      readTimeoutMs,
      "The stored Feed entry did not answer in time.",
    );
  } catch (initialError) {
    if (!shouldRetry(initialError)) throw initialError;

    onPhase("starting");
    try {
      await withDeadline(
        start,
        startTimeoutMs,
        "The Feed service did not start in time.",
      );
    } catch (startError) {
      throw new Error(`Feed is unavailable: ${errorMessage(startError)}`);
    }

    onPhase("retrying");
    try {
      return await withDeadline(
        (signal) => read(id, signal),
        readTimeoutMs,
        "The stored Feed entry did not answer in time after startup.",
      );
    } catch (retryError) {
      throw new Error(
        `The Feed started, but this entry still could not be loaded: ${errorMessage(retryError)}`,
      );
    }
  }
}
