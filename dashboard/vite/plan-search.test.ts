import { describe, expect, test } from "bun:test";

import type { PlanSearchJob, PlanSearchResult } from "../src/lib/travel/api";
import {
  coverageNotice,
  degradedNotice,
  money,
  pollJob,
  previousDay,
  unreachable,
} from "../src/lib/travel/plan-search";

const emptyResult = (over: Partial<PlanSearchResult> = {}): PlanSearchResult => ({
  revision: "plan-search-v1",
  window_source: "calendar",
  windows: [],
  reach: { calendar: "ok", places: "ok", transit: "ok", scouting: "ok", climate: "ok" },
  degraded: [],
  considered: 0,
  priced: 0,
  unpriced: 0,
  candidates: [],
  observed_at: "2026-10-01T09:00:00Z",
  ...over,
});

const done = (id: number): PlanSearchJob => ({ id, state: "done", result: emptyResult() });

describe("pollJob", () => {
  test("stops as soon as the job is done", async () => {
    let asked = 0;
    const outcome = await pollJob(7, {
      fetchStatus: async (job) => {
        asked += 1;
        return asked < 3 ? { id: job, state: "running", since_ms: asked * 100 } : done(job);
      },
      sleep: async () => {},
    });
    expect(outcome.state).toBe("done");
    expect(asked).toBe(3);
  });

  test("stops on a failed job and carries the server's own sentence", async () => {
    let asked = 0;
    const outcome = await pollJob(7, {
      fetchStatus: async (job) => {
        asked += 1;
        return {
          id: job,
          state: "failed",
          error: "calendar unavailable — a month search needs feasible windows",
        };
      },
      sleep: async () => {},
    });
    expect(asked).toBe(1);
    expect(outcome.state).toBe("failed");
    expect(outcome.state === "failed" && outcome.error).toContain("feasible windows");
  });

  /**
   * The real bug class: a job the server has evicted answers `running` forever
   * in a naive loop, or the answer never changes and the page polls until the
   * tab is closed. The deadline is what makes that terminate.
   */
  test("does not poll past its deadline", async () => {
    let asked = 0;
    let clock = 0;
    const outcome = await pollJob(7, {
      fetchStatus: async (job) => {
        asked += 1;
        return { id: job, state: "running", since_ms: clock };
      },
      sleep: async (ms) => {
        clock += ms;
      },
      now: () => clock,
      intervalMs: 1000,
      deadlineMs: 5000,
    });
    expect(outcome.state).toBe("timeout");
    expect(asked).toBeLessThanOrEqual(7);
    expect(outcome.state === "timeout" && outcome.error).toContain("still running");
  });

  test("a rejected status ends the poll rather than retrying an expired job", async () => {
    let asked = 0;
    const rejected = pollJob(7, {
      fetchStatus: async () => {
        asked += 1;
        throw new Error("that search result has expired — run the search again");
      },
      sleep: async () => {},
    });
    await expect(rejected).rejects.toThrow("expired");
    expect(asked).toBe(1);
  });

  test("reports every running answer so the panel can show elapsed time", async () => {
    const seen: number[] = [];
    let asked = 0;
    await pollJob(1, {
      fetchStatus: async (job) => {
        asked += 1;
        return asked < 3 ? { id: job, state: "running", since_ms: asked * 250 } : done(job);
      },
      sleep: async () => {},
      onUpdate: (answer) => {
        if (answer.state === "running") seen.push(answer.since_ms);
      },
    });
    expect(seen).toEqual([250, 500]);
  });
});

describe("labels", () => {
  test("money renders integer minor units, and says so when there is no fare", () => {
    expect(money(4359, "EUR")).toBe("43.59 EUR");
    expect(money(500, "EUR")).toBe("5.00 EUR");
    expect(money(null, "EUR")).toBe("no fare found");
  });

  test("a degraded result says the comparison is narrower than usual", () => {
    expect(degradedNotice(emptyResult())).toBeNull();
    const notice = degradedNotice(emptyResult({ degraded: ["calendar", "climate"] }));
    expect(notice).toContain("calendar, climate");
    expect(notice).toContain("comparable inside this result");
  });

  test("coverage says considered against priced rather than promising a number", () => {
    expect(coverageNotice(emptyResult({ considered: 40, priced: 4, unpriced: 4 }))).toBe(
      "40 destinations considered, 4 priced, 4 with no fare.",
    );
  });

  test("unreachable names each upstream and what it answered", () => {
    const result = emptyResult({
      reach: {
        calendar: "ok",
        places: "ok",
        transit: "error: transit station lookup answered 500",
        scouting: "ok",
        climate: "absent",
      },
    });
    expect(unreachable(result)).toEqual([
      "transit (error: transit station lookup answered 500)",
      "climate (absent)",
    ]);
  });

  test("an exclusive window end becomes an inclusive date input", () => {
    expect(previousDay("2026-10-12")).toBe("2026-10-11");
    expect(previousDay("2026-11-01")).toBe("2026-10-31");
    expect(previousDay("not a date")).toBe("not a date");
  });
});
