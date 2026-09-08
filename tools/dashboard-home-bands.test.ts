// The Home ladder's ordering law, as a build gate.
//
// The defect this closes was live for a year and invisible in review: PRD §8.1's bands sit
// as little as 10 apart while the additive urgency terms the page carried reached 400, so a
// trip starting today scored 800 + 400 = 1200 and outranked every host-watch finding, whose
// ceiling was 900 + 90 = 990. The comment at the old +page.svelte:112-116 states the rule
// the arithmetic broke — "a runaway process is worse than a plan that can wait".
//
// It lives in tools/ for the reason tools/dashboard-nav-links.test.ts:12-15 gives: SvelteKit
// type-checks everything under dashboard/src/, and `bun:test` is Bun's, not the browser's.

import { describe, expect, test } from "bun:test";
import {
  BAND_STRIDE,
  MAX_URGENCY,
  PRD_BANDS,
  RESCALE,
  bandTone,
  compareDecisions,
  compareStartOrDue,
  score,
  type Decision,
  type DecisionKind,
} from "../dashboard/src/lib/home/decisions.ts";

const kindStub = (key: string, band: number, lane?: "commitment" | "reading") =>
  ({ key, band, lane }) as unknown as DecisionKind<unknown, unknown>;

const decision = (
  key: string,
  band: number,
  urgency: number,
  startOrDueAt: string | null = null,
  lane?: "commitment" | "reading",
): Decision => ({
  key,
  kind: kindStub(key.split(":")[0], band, lane),
  row: {},
  priority: score(band, urgency),
  startOrDueAt,
});

describe("score() makes the band decide the order", () => {
  test("a trip starting today never outranks a host-watch finding", () => {
    // The exact inversion the ruling exists for, asserted by name.
    expect(score(800, MAX_URGENCY)).toBeLessThan(score(900, 0));
  });

  test("a maximally urgent task never outranks the quietest calendar proposal", () => {
    expect(score(620, MAX_URGENCY)).toBeLessThan(score(700, 0));
  });

  test("every adjacent PRD band pair stays ordered at maximum urgency", () => {
    const bands = [...PRD_BANDS].sort((a, b) => a - b);
    for (let i = 0; i + 1 < bands.length; i += 1) {
      expect(score(bands[i], MAX_URGENCY)).toBeLessThan(score(bands[i + 1], 0));
    }
  });

  test("the clamp, not the stride, is what makes the band decisive", () => {
    // Stated as a test because the first draft of the ruling gave a false reason.
    // 630·S + (S−1) < 640·S reduces to S−1 < 10S, true for every S ≥ 1.
    for (const stride of [1, 10, 100, 1000]) {
      const clamped = stride - 1;
      expect(630 * stride + clamped).toBeLessThan(640 * stride);
    }
    expect(BAND_STRIDE).toBe(1000);
    expect(MAX_URGENCY).toBe(999);
  });

  test("urgency is clamped in both directions", () => {
    expect(score(500, 5000)).toBe(score(500, MAX_URGENCY));
    expect(score(500, -3)).toBe(score(500, 0));
    expect(score(500, 12.6)).toBe(score(500, 13));
  });
});

describe("RESCALE preserves a kind's shape and only changes its range", () => {
  test("the endpoints and the midpoint land where the identity says", () => {
    expect(RESCALE(0, 400)).toBe(0);
    expect(RESCALE(400, 400)).toBe(MAX_URGENCY);
    expect(RESCALE(200, 400)).toBe(500);
  });

  test("a value past the stated maximum still clamps", () => {
    expect(RESCALE(900, 400)).toBe(MAX_URGENCY);
    expect(RESCALE(-40, 400)).toBe(0);
  });
});

describe("ties break on start-or-due, then on the stable id", () => {
  test("two undated tasks at the same vault priority order by key", () => {
    // Both land on the same number: 620·1000 + (3−p)·3 with no due date.
    const a = decision("task:notes/b.md", 620, 3);
    const b = decision("task:notes/a.md", 620, 3);
    expect([a, b].sort(compareDecisions).map((d) => d.key)).toEqual([
      "task:notes/a.md",
      "task:notes/b.md",
    ]);
  });

  test("an earlier start-or-due wins before the key is consulted", () => {
    const later = decision("task:a.md", 620, 3, "2026-10-01");
    const sooner = decision("task:z.md", 620, 3, "2026-09-07");
    expect([later, sooner].sort(compareDecisions).map((d) => d.key)).toEqual([
      "task:z.md",
      "task:a.md",
    ]);
  });

  test("a dated row sorts above an undated one at equal priority", () => {
    const undated = decision("task:a.md", 620, 3, null);
    const dated = decision("task:z.md", 620, 3, "2026-12-01");
    expect([undated, dated].sort(compareDecisions).map((d) => d.key)).toEqual([
      "task:z.md",
      "task:a.md",
    ]);
    expect(compareStartOrDue(null, null)).toBe(0);
  });

  test("two kinds sharing band 640 order by urgency, then by both tie-breakers", () => {
    // PRD §8.1's purchase decision and §13.1's budget overrun both sit at 640, so this
    // is the shared-band case.
    const quiet = decision("finance:proposal-1", 640, 10, "2026-11-01");
    const loud = decision("purchase:renewal-9", 640, 800, "2026-12-01");
    const sameUrgency = decision("purchase:renewal-1", 640, 10, "2026-10-01");
    expect([quiet, loud, sameUrgency].sort(compareDecisions).map((d) => d.key)).toEqual([
      "purchase:renewal-9",
      "purchase:renewal-1",
      "finance:proposal-1",
    ]);
  });
});

describe("the date tie-break runs in the direction the lane gives it", () => {
  test("a commitment breaks its tie on the soonest deadline", () => {
    const soon = decision("calendar:b", 700, 5, "2026-09-10");
    const later = decision("calendar:a", 700, 5, "2026-11-01");
    expect([later, soon].sort(compareDecisions).map((d) => d.key)).toEqual([
      "calendar:b",
      "calendar:a",
    ]);
  });

  test("reading breaks its tie on the newest arrival", () => {
    // `startOrDueAt` on the reading lane is `created_at`, an arrival time and not a
    // deadline. Earliest-first there put the six OLDEST unread articles in the collapsed
    // preview of a list comms serves newest-first.
    const oldest = decision("feed:a", 500, 500, "2026-08-10", "reading");
    const middle = decision("feed:b", 500, 500, "2026-08-20", "reading");
    const newest = decision("feed:c", 500, 500, "2026-09-05", "reading");
    expect([oldest, middle, newest].sort(compareDecisions).map((d) => d.key)).toEqual([
      "feed:c",
      "feed:b",
      "feed:a",
    ]);
  });

  test("an undated reading row still sorts last", () => {
    const dated = decision("feed:b", 500, 500, "2026-08-10", "reading");
    const undated = decision("feed:a", 500, 500, null, "reading");
    expect([undated, dated].sort(compareDecisions).map((d) => d.key)).toEqual([
      "feed:b",
      "feed:a",
    ]);
  });
});

describe("the band spine collapses thirteen bands onto four tones", () => {
  test("each tone covers the bands its name claims", () => {
    expect(bandTone(10_000)).toBe("alarm");
    expect(bandTone(900)).toBe("alarm");
    expect(bandTone(800)).toBe("now");
    expect(bandTone(700)).toBe("now");
    expect(bandTone(640)).toBe("owed");
    expect(bandTone(610)).toBe("owed");
    expect(bandTone(600)).toBe("offer");
    expect(bandTone(490)).toBe("offer");
  });
});
