// The shared keyboard cursor's guard, which every list that adopts it inherits.
//
// The guard is the part that is easy to get subtly wrong and impossible to notice: press
// `j` while the caret is in the feed's paste box and the page scrolls instead of typing a
// letter. Home had the rule right and kept it in a private function; this proves the
// extracted one still matches, so /feed and Home cannot drift apart.
//
// `createListCursor` itself holds a `$state` rune, which only exists once the Svelte
// compiler has run. The pure halves are exported for exactly this reason and are what a
// plain `bun test` can reach.

import { describe, expect, test } from "bun:test";
import { readdirSync, readFileSync, statSync } from "node:fs";
import { extname, join } from "node:path";
import {
  INTERACTIVE,
  clampIndex,
  clampToSelectable,
  focusRow,
  nextSelectable,
  prevSelectable,
  shouldIgnoreKey,
  type CursorRow,
} from "../dashboard/src/lib/list-cursor.svelte.ts";

const SRC = join(import.meta.dir, "../dashboard/src");

/** Every .svelte and .ts file the dashboard ships, minus its own tests. */
function sources(dir: string): string[] {
  return readdirSync(dir).flatMap((name) => {
    const full = join(dir, name);
    if (statSync(full).isDirectory()) return sources(full);
    return [".svelte", ".ts"].includes(extname(name)) && !name.endsWith(".test.ts") ? [full] : [];
  });
}

/** A keydown whose target reports the `closest()` answer the test wants. */
function keydown(overrides: Partial<KeyboardEvent> & { inside?: boolean } = {}): KeyboardEvent {
  const { inside = false, ...rest } = overrides;
  const target = {
    closest: (selector: string) => (inside && selector === INTERACTIVE ? {} : null),
  };
  return {
    key: "j",
    metaKey: false,
    ctrlKey: false,
    altKey: false,
    target,
    ...rest,
  } as unknown as KeyboardEvent;
}

describe("the cursor ignores keystrokes that are not its own", () => {
  test("the selector covers every control a row can hold, plus a dialog", () => {
    for (const part of ["a", "button", "input", "select", "textarea", "[contenteditable]", "[role='dialog']"]) {
      expect([part, INTERACTIVE.includes(part)]).toEqual([part, true]);
    }
  });

  test("a keystroke inside a control is not the list's", () => {
    expect(shouldIgnoreKey(keydown({ inside: true }))).toBe(true);
  });

  test("a modified keystroke belongs to the browser", () => {
    expect(shouldIgnoreKey(keydown({ metaKey: true }))).toBe(true);
    expect(shouldIgnoreKey(keydown({ ctrlKey: true }))).toBe(true);
    expect(shouldIgnoreKey(keydown({ altKey: true }))).toBe(true);
  });

  test("a bare keystroke on the page is the list's", () => {
    expect(shouldIgnoreKey(keydown())).toBe(false);
  });

  test("a target with no closest() is still handled", () => {
    // `window` is the target when nothing has focus, which is the ordinary case, and
    // window has no `closest`. The old `instanceof Element` guard threw on it outside a
    // browser and silently returned false inside one.
    expect(shouldIgnoreKey({ key: "j", target: globalThis } as unknown as KeyboardEvent)).toBe(false);
    expect(shouldIgnoreKey({ key: "j", target: null } as unknown as KeyboardEvent)).toBe(false);
  });
});

describe("the index clamps rather than running off the list", () => {
  test("it stays inside the list", () => {
    expect(clampIndex(-4, 10)).toBe(0);
    expect(clampIndex(0, 10)).toBe(0);
    expect(clampIndex(9, 10)).toBe(9);
    expect(clampIndex(40, 10)).toBe(9);
  });

  test("an empty list parks the cursor at zero rather than at -1", () => {
    // The regression: a dismissal resolving under the cursor shrank the list to nothing
    // and `count - 1` handed back -1, which then indexed undefined on the next Enter.
    expect(clampIndex(3, 0)).toBe(0);
  });

  test("a list that shrinks under the cursor pulls it back to the last row", () => {
    expect(clampIndex(7, 3)).toBe(2);
  });
});

describe("a list whose rows are not all selectable", () => {
  // The Feed's shape: a collector-run header, then its items. Merged into this module on
  // 2026-09-07 from `$lib/feed/list-cursor`, which held the same arithmetic beside a
  // second copy of the guard above and had no test of its own.
  const rows: CursorRow[] = [
    { kind: "header", id: "h1" },
    { kind: "item", id: "a" },
    { kind: "item", id: "b" },
    { kind: "header", id: "h2" },
    { kind: "item", id: "c" },
  ];

  test("moving down steps over a header", () => {
    expect(nextSelectable(rows, 2)).toBe(4);
  });

  test("moving down stops on the last item rather than wrapping", () => {
    expect(nextSelectable(rows, 4)).toBe(4);
  });

  test("moving up steps over a header and stops on the first item", () => {
    expect(prevSelectable(rows, 4)).toBe(2);
    expect(prevSelectable(rows, 1)).toBe(1);
  });

  test("an unplaced cursor lands on the first item, in both directions", () => {
    expect(nextSelectable(rows, -1)).toBe(1);
    expect(prevSelectable(rows, 0)).toBe(1);
  });

  test("a cursor left on a header after a reload moves forward to an item", () => {
    expect(clampToSelectable(rows, 3)).toBe(4);
  });

  test("a cursor past the end comes back to the last item", () => {
    expect(clampToSelectable(rows, 99)).toBe(4);
  });

  test("a trailing header falls back to the item before it", () => {
    expect(clampToSelectable([...rows, { kind: "header", id: "h3" }], 5)).toBe(4);
  });

  test("an empty list parks at -1, because no row can hold the cursor", () => {
    // Deliberately not clampIndex's 0: there is no row 0 to select, and the page reads
    // this value back as "nothing selected".
    expect(clampToSelectable([], 3)).toBe(-1);
  });

  test("a list of headers alone selects nothing", () => {
    expect(clampToSelectable([{ kind: "header", id: "h1" }], 0)).toBe(-1);
  });
});

describe("moving the cursor moves real focus, not a class", () => {
  /** An element that records what the cursor did to it, in order. */
  function fakeRow() {
    const calls: string[] = [];
    let focusOptions: FocusOptions | undefined;
    let scrollOptions: ScrollIntoViewOptions | undefined;
    const element = {
      focus(options?: FocusOptions) {
        calls.push("focus");
        focusOptions = options;
      },
      scrollIntoView(options?: ScrollIntoViewOptions) {
        calls.push("scrollIntoView");
        scrollOptions = options;
      },
    };
    return {
      element: element as unknown as HTMLElement,
      calls,
      get focusOptions() {
        return focusOptions;
      },
      get scrollOptions() {
        return scrollOptions;
      },
    };
  }

  test("the row is focused, and focused before it is scrolled", () => {
    const row = fakeRow();
    focusRow(row.element);
    expect(row.calls).toEqual(["focus", "scrollIntoView"]);
  });

  test("the scroll focus() would do on its own is suppressed and asked for again", () => {
    // focus() alone puts the row at the top of the viewport and loses the rows above it.
    const row = fakeRow();
    focusRow(row.element);
    expect(row.focusOptions).toEqual({ preventScroll: true });
    expect(row.scrollOptions?.block).toBe("nearest");
  });

  test("a row that is not on screen yet is not an error", () => {
    expect(() => focusRow(null)).not.toThrow();
    expect(() => focusRow(undefined)).not.toThrow();
  });
});

describe("no list scrolls a row into view without focusing it", () => {
  // The defect this replaces: /feed's `moveCursor` called `scrollIntoView` and nothing
  // else, for every row the operator reached with j or k. A `.selected` class and a
  // scroll offset are a paint — no assistive technology is told the selection moved, and
  // a keyboard reader's Tab position never leaves wherever it was before the first key.
  //
  // Stated as "the only scrollIntoView is focusRow's" rather than as a per-file rule,
  // because the failure mode is a NEW page hand-rolling the scroll again. A second call
  // site is exactly the shape of that mistake, so a second call site fails here and its
  // author has to say why.
  const files = sources(SRC);

  test("there are sources to scan, so a passing run means something", () => {
    expect(files.length).toBeGreaterThan(100);
  });

  test("scrollIntoView appears once, inside focusRow", () => {
    const callers = files
      .filter((file) => /\.scrollIntoView\s*\(/.test(readFileSync(file, "utf8")))
      .map((file) => file.slice(SRC.length + 1));
    expect(callers).toEqual(["lib/list-cursor.svelte.ts"]);
  });

  test("/feed's own cursor calls focusRow", () => {
    const feed = readFileSync(join(SRC, "routes/feed/+page.svelte"), "utf8");
    expect(feed).toContain("focusRow(document.getElementById(rowDomId(row.id)))");
  });
});
