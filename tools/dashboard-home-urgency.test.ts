// Every kind's urgency stays inside 0..999, and none of them silently changed slope.
//
// The rescale rule is what makes the band redesign safe to apply to eight kinds at once:
// each kind keeps its own expression exactly and only its range changes. A hand-written
// replacement is where a kind quietly loses a floor or moves its saturation point, and
// nothing downstream would ever say so — the ladder would just be subtly wrong.
//
// This imports the kind files themselves, which works because api.ts has no imports at all
// and every kind reaches it and nav.ts by relative path rather than through `$lib`.

import { describe, expect, test } from "bun:test";
import { MAX_URGENCY, RESCALE, type ScoreContext } from "../dashboard/src/lib/home/decisions.ts";
import host from "../dashboard/src/lib/home/kinds/host.ts";
import trip from "../dashboard/src/lib/home/kinds/trip.ts";
import calendarKind from "../dashboard/src/lib/home/kinds/calendar.ts";
import task from "../dashboard/src/lib/home/kinds/task.ts";
import opportunity from "../dashboard/src/lib/home/kinds/opportunity.ts";
import mail from "../dashboard/src/lib/home/kinds/mail.ts";
import feed from "../dashboard/src/lib/home/kinds/feed.ts";
import system from "../dashboard/src/lib/home/kinds/system.ts";

const TODAY = "2026-09-05";

/** A context with no peers, which is the case every kind must survive. */
function context(overrides: Partial<ScoreContext> = {}): ScoreContext {
  return {
    todayKey: TODAY,
    horizonEndKey: "2027-01-05",
    nowMs: Date.parse(`${TODAY}T12:00:00Z`),
    daysUntil: (value: string) => {
      const date = new Date(`${value.slice(0, 10)}T12:00:00`);
      if (Number.isNaN(date.getTime())) return 365;
      const start = new Date(`${TODAY}T12:00:00`);
      return Math.ceil((date.getTime() - start.getTime()) / 86_400_000);
    },
    peer: () => [],
    peerSource: () => null,
    ...overrides,
  };
}

/** Every synthetic row here is invented. No overlay value reaches a test. */
const anyRow = <T,>(value: unknown): T => value as T;

describe("no kind's urgency can leave 0..999", () => {
  const adversarial: [string, number][] = [
    // A finding first seen 400 days ago, far past the nine-day saturation point.
    ["host", host.urgency(anyRow({ first_seen: "2025-08-01" }), context())],
    // A trip that started yesterday: the old expression's maximum and then some.
    ["trip", trip.urgency(anyRow({ date_start: "2026-09-04" }), context())],
    // A deliberate web entry today, both terms at their ceiling.
    ["calendar", calendarKind.urgency(anyRow({ starts_at: TODAY, source: "web" }), context())],
    // 900 days overdue at the highest vault priority.
    ["task", task.urgency(anyRow({ due: "2024-03-20", priority: 1 }), context())],
    ["mail-bp", mail.urgency(anyRow({ score_bp: 10_000 }))],
    ["mail-bp-over", mail.urgency(anyRow({ score_bp: 99_999 }))],
    ["feed", feed.urgency(anyRow({ evaluation: { overall_score: 1 }, created_at: `${TODAY}T11:00:00Z` }), context())],
    ["system", system.urgency(anyRow({}), context())],
  ];

  for (const [name, value] of adversarial) {
    test(`${name} stays in range`, () => {
      expect([name, Number.isFinite(value)]).toEqual([name, true]);
      expect(value).toBeGreaterThanOrEqual(0);
      expect(value).toBeLessThanOrEqual(MAX_URGENCY);
    });
  }

  test("an opportunity at score 1.0 with both adjustments at maximum stays in range", () => {
    const ctx = context({
      peerSource: <S,>() =>
        ({
          entries: [
            {
              id: "e1",
              commitment: "planned",
              starts_at: `${TODAY}T09:00:00`,
              location: "Testville",
              payload: null,
            },
          ],
          contexts: [
            {
              id: "c1",
              kind: "focus",
              title: "Testville week",
              details: "testville",
              valid_from: "2026-09-01",
              valid_until: "2026-12-01",
            },
          ],
        }) as S,
    });
    const row = anyRow({
      id: "o1",
      opportunity_type: "conference",
      starts_at: TODAY,
      ends_at: TODAY,
      city: "Testville",
      matched_focus: "systems",
      score: 1,
      status: "new",
      url: "https://example.invalid/o1",
      rationale: "",
    });
    const value = opportunity.urgency(row, ctx);
    expect(value).toBeGreaterThanOrEqual(0);
    expect(value).toBeLessThanOrEqual(MAX_URGENCY);
  });

  test("a negative-adjustment opportunity floors at 0 rather than going negative", () => {
    const ctx = context({
      peerSource: <S,>() =>
        ({
          entries: [
            {
              id: "e1",
              commitment: "committed",
              starts_at: `${TODAY}T09:00:00`,
              location: "Elsewhere",
              payload: null,
            },
          ],
          contexts: [],
        }) as S,
    });
    const row = anyRow({
      id: "o2",
      opportunity_type: "conference",
      starts_at: TODAY,
      ends_at: TODAY,
      city: "Testville",
      matched_focus: "systems",
      score: 0,
      status: "new",
      url: "https://example.invalid/o2",
      rationale: "",
    });
    expect(opportunity.urgency(row, ctx)).toBe(0);
  });
});

describe("each rescale reproduces the expression it replaced", () => {
  const ctx = context();

  test("host keeps its ten-points-a-day slope and its nine-day saturation", () => {
    // Old: 900 + min(90, days * 10). Sampled at 0, 4.5 and 9+ days.
    expect(host.urgency(anyRow({ first_seen: TODAY }), ctx)).toBe(RESCALE(0, 90));
    expect(host.urgency(anyRow({ first_seen: "2026-09-01" }), ctx)).toBe(RESCALE(40, 90));
    expect(host.urgency(anyRow({ first_seen: "2026-08-01" }), ctx)).toBe(MAX_URGENCY);
  });

  test("trip keeps its fourteen-day cliff", () => {
    // Old: days <= 14 ? max(0, 400 - days*20) : 0. Day 15 is zero, day 14 is 120.
    expect(trip.urgency(anyRow({ date_start: TODAY }), ctx)).toBe(RESCALE(400, 400));
    expect(trip.urgency(anyRow({ date_start: "2026-09-12" }), ctx)).toBe(RESCALE(260, 400));
    expect(trip.urgency(anyRow({ date_start: "2026-09-19" }), ctx)).toBe(RESCALE(120, 400));
    expect(trip.urgency(anyRow({ date_start: "2026-09-20" }), ctx)).toBe(0);
  });

  test("calendar keeps the fifty-point lift for a deliberately added entry", () => {
    const imported = calendarKind.urgency(anyRow({ starts_at: TODAY, source: "gmail" }), ctx);
    const deliberate = calendarKind.urgency(anyRow({ starts_at: TODAY, source: "web" }), ctx);
    expect(imported).toBe(RESCALE(240, 290));
    expect(deliberate).toBe(RESCALE(290, 290));
    expect(deliberate).toBeGreaterThan(imported);
  });

  test("task's vault priority is worth less than one day of the due-date slope", () => {
    // The stated rule: a note marked high is not more urgent than one due a day sooner.
    const highLater = task.urgency(anyRow({ due: "2026-09-15", priority: 1 }), ctx);
    const lowSooner = task.urgency(anyRow({ due: "2026-09-14", priority: 3 }), ctx);
    expect(lowSooner).toBeGreaterThan(highLater);
    expect(task.urgency(anyRow({ due: null, priority: 2 }), ctx)).toBe(RESCALE(3, 266));
    expect(task.urgency(anyRow({ due: "2026-09-04", priority: 2 }), ctx)).toBe(RESCALE(263, 266));
  });

  test("feed prefers the factorised evaluation over the legacy relevance row", () => {
    // The bug this fixes: an item with an evaluation and no relevance row scored 0 on
    // Home while /feed put it at the top of the list.
    const evaluated = anyRow({
      evaluation: { overall_score: 0.9 },
      relevance: null,
      created_at: "2026-08-01T00:00:00Z",
    });
    const legacy = anyRow({
      evaluation: null,
      relevance: { score: 0.2 },
      created_at: "2026-08-01T00:00:00Z",
    });
    expect(feed.urgency(evaluated, ctx)).toBe(RESCALE(90, 140));
    expect(feed.urgency(legacy, ctx)).toBe(RESCALE(20, 140));
  });

  test("mail is a nudge until score_bp arrives, and then it is the whole rank", () => {
    // Deliberately NOT a rescale: the only term is a 48-hour recency nudge worth 30
    // points, and rescaling 30 by its own maximum would put every recent mail at the top
    // of the band.
    const recent = anyRow({ internal_date: new Date(Date.now() - 3_600_000).toISOString() });
    const old = anyRow({ internal_date: "2026-01-01T00:00:00Z" });
    expect(mail.urgency(recent)).toBe(120);
    expect(mail.urgency(old)).toBe(0);
    expect(mail.urgency(anyRow({ score_bp: 0, internal_date: null }))).toBe(0);
    expect(mail.urgency(anyRow({ score_bp: 5000, internal_date: null }))).toBe(500);
    expect(mail.urgency(anyRow({ score_bp: 10_000, internal_date: null }))).toBe(MAX_URGENCY);
  });
});

describe("a kind that reads a peer survives the peer failing", () => {
  test("opportunity ranks and explains without the calendar rather than throwing", () => {
    const ctx = context({ peerSource: () => null });
    const row = anyRow({
      id: "o3",
      opportunity_type: "conference",
      starts_at: TODAY,
      ends_at: TODAY,
      city: "Testville",
      matched_focus: "systems",
      score: 0.5,
      status: "new",
      url: "https://example.invalid/o3",
      rationale: "",
    });
    expect(opportunity.urgency(row, ctx)).toBe(RESCALE(250, 435));
    expect(opportunity.whyHere(row, ctx)).toBe("ranked without the calendar");
  });

  test("opportunity's gate does not raise a row already in the diary", () => {
    const ctx = context({
      peerSource: <S,>() =>
        ({
          entries: [
            {
              id: "e9",
              commitment: "committed",
              starts_at: `${TODAY}T09:00:00`,
              location: "Testville",
              payload: { opportunity_id: "o4" },
            },
          ],
          contexts: [],
        }) as S,
    });
    const row = {
      id: "o4",
      opportunity_type: "conference",
      starts_at: TODAY,
      ends_at: TODAY,
      city: "Testville",
      matched_focus: "systems",
      score: 0.9,
      status: "new",
      url: "https://example.invalid/o4",
      rationale: "",
    };
    expect(opportunity.rows(anyRow({ opportunities: [row], sources: [] }), ctx)).toEqual([]);
  });
});
