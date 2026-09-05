import { describe, expect, test } from "bun:test";

import {
  clampAfterDecision,
  isTypingTarget,
  next,
  prev,
  type CursorRow,
} from "../src/lib/feed/list-cursor";

/** Three day groups, two of which hold a collector run. The Inbox used to
 *  collapse a run of two or more into ONE clickable line, so 185 arXiv items
 *  and 147 GitHub items rendered as a handful of rows and the daily triage
 *  surface showed almost nothing to triage. */
const rows: CursorRow[] = [
  { kind: "header", id: "2026-09-05" },
  { kind: "item", id: "a" },
  { kind: "header", id: "2026-09-05/arxiv-run" },
  { kind: "item", id: "b" },
  { kind: "item", id: "c" },
  { kind: "header", id: "2026-09-04" },
  { kind: "item", id: "d" },
  { kind: "header", id: "2026-09-03/github-run" },
  { kind: "item", id: "e" },
  { kind: "item", id: "f" },
];

describe("the feed list cursor", () => {
  test("every rendered item is reachable exactly once, and no header is", () => {
    const visited: string[] = [];
    let index = clampAfterDecision(rows, 0);
    for (let step = 0; step < rows.length * 2; step += 1) {
      expect(rows[index].kind).toBe("item");
      if (visited[visited.length - 1] === rows[index].id) break;
      visited.push(rows[index].id);
      index = next(rows, index);
    }
    expect(visited).toEqual(["a", "b", "c", "d", "e", "f"]);
    expect(new Set(visited).size).toBe(visited.length);
  });

  test("j and k walk the same list in both directions and do not wrap", () => {
    const last = clampAfterDecision(rows, rows.length - 1);
    expect(rows[last].id).toBe("f");
    expect(next(rows, last)).toBe(last);

    const first = clampAfterDecision(rows, 0);
    expect(rows[first].id).toBe("a");
    expect(prev(rows, first)).toBe(first);

    // A header between two items is stepped over, not landed on.
    const onB = next(rows, first);
    expect(rows[onB].id).toBe("b");
    expect(rows[prev(rows, onB)].id).toBe("a");
  });

  test("the cursor clamps after a decision", () => {
    // The last row is decided and the next load removes it.
    const shortened = rows.slice(0, rows.length - 1);
    const clamped = clampAfterDecision(shortened, rows.length - 1);
    expect(clamped).toBeGreaterThanOrEqual(0);
    expect(shortened[clamped].kind).toBe("item");
    expect(shortened[clamped].id).toBe("e");

    // A list that is only headers has nothing to select, and says so rather
    // than pointing at a header.
    expect(clampAfterDecision([{ kind: "header", id: "day" }], 0)).toBe(-1);
    expect(clampAfterDecision([], 3)).toBe(-1);
  });

  test("typing in a form control is typing, not a shortcut", () => {
    const inField = {
      metaKey: false,
      ctrlKey: false,
      altKey: false,
      target: { closest: (selector: string) => (selector.includes("input") ? {} : null) },
    } as unknown as KeyboardEvent;
    expect(isTypingTarget(inField)).toBe(true);

    const onPage = {
      metaKey: false,
      ctrlKey: false,
      altKey: false,
      target: { closest: () => null },
    } as unknown as KeyboardEvent;
    expect(isTypingTarget(onPage)).toBe(false);

    const withModifier = { ...onPage, metaKey: true } as unknown as KeyboardEvent;
    expect(isTypingTarget(withModifier)).toBe(true);
  });
});
