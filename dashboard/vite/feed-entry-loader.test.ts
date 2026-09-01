import { describe, expect, test } from "bun:test";

import {
  loadFeedEntry,
  normalizeContentItemDetail,
  normalizeFeedEntryDetail,
  type FeedEntryLoadPhase,
} from "../src/lib/feed/entry-loader";

function hangsUntilAborted(signal: AbortSignal): Promise<never> {
  return new Promise((_, reject) => {
    signal.addEventListener(
      "abort",
      () => reject(new DOMException("Aborted", "AbortError")),
      { once: true },
    );
  });
}

describe("Feed entry loading", () => {
  test("older entry responses receive empty additive metadata", () => {
    const entry = normalizeFeedEntryDetail({ id: "entry:legacy" } as never);

    expect(entry.relevance).toEqual([]);
    expect(entry.processing).toEqual([]);
    expect(entry.origins).toEqual([]);
  });

  test("the shared content reader normalizes additive metadata for mail too", () => {
    const entry = normalizeContentItemDetail({
      schema_version: "content-item-v2",
      source: "mail",
      id: "thread:1",
    } as never);

    expect(entry.source).toBe("mail");
    expect(entry.cloud_processing).toEqual({
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
    });
    expect(entry.relevance).toEqual([]);
    expect(entry.processing).toEqual([]);
    expect(entry.origins).toEqual([]);
  });

  test("the warm path reads stored data immediately without lifecycle work", async () => {
    let starts = 0;
    const phases: FeedEntryLoadPhase[] = [];

    const entry = await loadFeedEntry({
      id: "entry:warm",
      read: async (id) => ({ id }),
      start: async () => { starts += 1; },
      onPhase: (phase) => phases.push(phase),
    });

    expect(entry).toEqual({ id: "entry:warm" });
    expect(starts).toBe(0);
    expect(phases).toEqual(["reading"]);
  });

  test("a cold direct link starts Comms and retries exactly once", async () => {
    let reads = 0;
    let starts = 0;
    const phases: FeedEntryLoadPhase[] = [];

    const entry = await loadFeedEntry({
      id: "entry:cold",
      read: async (id) => {
        reads += 1;
        if (reads === 1) throw new TypeError("connection refused");
        return { id };
      },
      start: async () => { starts += 1; },
      onPhase: (phase) => phases.push(phase),
    });

    expect(entry).toEqual({ id: "entry:cold" });
    expect(reads).toBe(2);
    expect(starts).toBe(1);
    expect(phases).toEqual(["reading", "starting", "retrying"]);
  });

  test("a non-retryable response never starts the service", async () => {
    const missing = new Error("not found");
    let starts = 0;

    await expect(loadFeedEntry({
      id: "entry:missing",
      read: async () => { throw missing; },
      start: async () => { starts += 1; },
      shouldRetry: (error) => error !== missing,
    })).rejects.toBe(missing);
    expect(starts).toBe(0);
  });

  test("deadlines turn stalled reads and startup into a terminal error", async () => {
    await expect(loadFeedEntry({
      id: "entry:stalled",
      read: async (_id, signal) => hangsUntilAborted(signal),
      start: async (signal) => hangsUntilAborted(signal),
      readTimeoutMs: 5,
      startTimeoutMs: 5,
    })).rejects.toThrow("Feed is unavailable: The Feed service did not start in time.");
  });

  test("a failed retry explains that startup completed", async () => {
    await expect(loadFeedEntry({
      id: "entry:broken",
      read: async () => { throw new Error("database unavailable"); },
      start: async () => undefined,
    })).rejects.toThrow(
      "The Feed started, but this entry still could not be loaded: database unavailable",
    );
  });
});
