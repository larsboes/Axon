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
import {
  INTERACTIVE,
  clampIndex,
  shouldIgnoreKey,
} from "../dashboard/src/lib/list-cursor.svelte.ts";

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
