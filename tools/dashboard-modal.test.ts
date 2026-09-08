// A modal dialog is modal to a keyboard, or it is a panel with a dark rectangle behind it.
//
// Measured in `dashboard/src/` on 2026-09-08, before `$lib/modal.ts` existed:
//
//   role="dialog" aria-modal="true"   4   Overlay, RhythmForm, ContextPanel,
//                                         feed/[id]'s cloud preview
//   traps Tab                         1   Overlay only
//   focuses the sheet on mount        2   Overlay, RhythmForm
//   restores focus on close           0
//
// Each missing piece has its own reader-visible failure. No trap: the first Tab leaves the
// sheet for a page hidden behind the backdrop, and Escape is the only way back — and two of
// the three had no Escape handler either. No mount focus: the reader is still on the page
// behind, so the dialog is never announced and the keys they press go somewhere else. No
// restore: closing drops focus to `<body>`, and on /calendar that is about forty tab stops
// from the button that opened it.
//
// The trap's arithmetic is what this file mostly tests, because it is the half that is
// wrong quietly — a trap that skips the last control, or that pins focus on the sheet, is
// still a trap and still broken. `nextTrapStop` is generic over the element type precisely
// so it can be driven here: `bun test` has no DOM.

import { describe, expect, test } from "bun:test";
import { readdirSync, readFileSync, statSync } from "node:fs";
import { extname, join } from "node:path";
import { FOCUSABLE, nextTrapStop } from "../dashboard/src/lib/modal.ts";

const SRC = join(import.meta.dir, "../dashboard/src");

function sources(dir: string): string[] {
  return readdirSync(dir).flatMap((name) => {
    const full = join(dir, name);
    if (statSync(full).isDirectory()) return sources(full);
    return extname(name) === ".svelte" ? [full] : [];
  });
}

/**
 * The whole opening tag of every element that declares `role="dialog"`.
 *
 * Walks out from the attribute rather than matching a tag pattern, because these tags are
 * written over six lines with an expression in them and a single-line regex misses them —
 * which is the failure a gate must not have.
 *
 * A `>` preceded by `=` is an arrow function inside an attribute value, not the end of the
 * tag. `use:modal={{ onClose: () => (cloudPreview = null) }}` is real and in the tree.
 */
function dialogTags(text: string): string[] {
  const found: string[] = [];
  for (const match of text.matchAll(/role="dialog"/g)) {
    const open = text.lastIndexOf("<", match.index);
    let close = match.index;
    do {
      close = text.indexOf(">", close + 1);
    } while (close > 0 && text[close - 1] === "=");
    if (open < 0 || close < 0) continue;
    found.push(text.slice(open, close + 1));
  }
  return found;
}

describe("the Tab trap keeps focus inside the sheet", () => {
  const stops = ["first", "middle", "last"];
  const sheet = "sheet";

  test("Tab off the last control wraps to the first", () => {
    expect(nextTrapStop(stops, sheet, "last", false)).toBe("first");
  });

  test("Shift+Tab off the first control wraps to the last", () => {
    expect(nextTrapStop(stops, sheet, "first", true)).toBe("last");
  });

  test("Shift+Tab from the sheet itself goes to the last control", () => {
    // The sheet is focused on mount and sits before its own children in tab order, so
    // the browser's answer to Shift+Tab here is the page behind the backdrop.
    expect(nextTrapStop(stops, sheet, sheet, true)).toBe("last");
  });

  test("in the middle of the sheet the browser is left alone", () => {
    // null, not the next stop. Re-deriving document order inside the trap is how a trap
    // starts skipping controls it does not know about.
    expect(nextTrapStop(stops, sheet, "middle", false)).toBeNull();
    expect(nextTrapStop(stops, sheet, "middle", true)).toBeNull();
    expect(nextTrapStop(stops, sheet, "first", false)).toBeNull();
    expect(nextTrapStop(stops, sheet, "last", true)).toBeNull();
  });

  test("Tab from the sheet itself is the browser's to answer", () => {
    expect(nextTrapStop(stops, sheet, sheet, false)).toBeNull();
  });

  test("focus that has already escaped is pulled back to the near end", () => {
    // The backdrop button, or `<body>` after a control was disabled out from under it.
    expect(nextTrapStop(stops, sheet, "backdrop", false)).toBe("first");
    expect(nextTrapStop(stops, sheet, "backdrop", true)).toBe("last");
    expect(nextTrapStop(stops, sheet, null, false)).toBe("first");
    expect(nextTrapStop(stops, sheet, null, true)).toBe("last");
  });

  test("a sheet with one control keeps focus on it in both directions", () => {
    expect(nextTrapStop(["only"], sheet, "only", false)).toBe("only");
    expect(nextTrapStop(["only"], sheet, "only", true)).toBe("only");
  });

  test("a sheet with nothing to focus holds the sheet rather than leaking", () => {
    expect(nextTrapStop([], sheet, sheet, false)).toBe(sheet);
    expect(nextTrapStop([], sheet, "anything", true)).toBe(sheet);
  });
});

describe("the focusable selector", () => {
  test("it excludes the sheet's own tabindex=-1", () => {
    // The sheet carries tabindex="-1" so it can be focused on mount. A bare `[tabindex]`
    // here would make the container one of its own tab stops, and Tab would then bounce
    // between the sheet and its first control forever.
    expect(FOCUSABLE).toContain('[tabindex]:not([tabindex="-1"])');
    expect(FOCUSABLE).not.toContain("[tabindex],");
  });

  test("a disabled control is not a stop", () => {
    for (const part of ["button:not([disabled])", "input:not([disabled])", "select:not([disabled])"]) {
      expect([part, FOCUSABLE.includes(part)]).toEqual([part, true]);
    }
  });
});

describe("every dialog in the dashboard is wired to the shared behaviour", () => {
  const files = sources(SRC);
  const tags = files.flatMap((file) =>
    dialogTags(readFileSync(file, "utf8")).map((tag) => [file.slice(SRC.length + 1), tag] as const),
  );

  test("the scan finds the dialogs, so a passing run means something", () => {
    // Four on 2026-09-08. Stated as a floor rather than an equality: a fifth dialog must
    // fail the two rules below, not this line.
    expect(files.length).toBeGreaterThan(80);
    expect(tags.length).toBeGreaterThanOrEqual(4);
  });

  test("each one uses the shared trap, mount focus and focus restore", () => {
    const offenders = tags
      .filter(([, tag]) => !tag.includes("use:modal"))
      .map(([file]) => file)
      .sort();
    expect(offenders).toEqual([]);
  });

  test("each one is focusable, or the mount focus has nowhere to land", () => {
    const offenders = tags
      .filter(([, tag]) => !tag.includes('tabindex="-1"'))
      .map(([file]) => file)
      .sort();
    expect(offenders).toEqual([]);
  });

  test("the tag reader survives a dialog written over several lines", () => {
    // Every dialog in this tree is. A single-line regex finds none of them.
    const multiline = `<div\n  class="sheet"\n  use:modal={{ onClose }}\n  role="dialog"\n  tabindex="-1"\n>`;
    expect(dialogTags(multiline)).toEqual([multiline]);
    expect(dialogTags(`<div role="dialog" tabindex="-1">`)).toHaveLength(1);
    expect(dialogTags(`<div class="sheet">`)).toEqual([]);
  });

  test("an arrow function after the role attribute does not truncate the tag", () => {
    // Reading the tag short is the dangerous failure: `use:modal` would fall outside the
    // slice and the gate would fail a dialog that is correctly wired, or — with the
    // attributes the other way round — pass one that is not.
    const arrowLast = `<div role="dialog" use:modal={{ onClose: () => (open = null) }} tabindex="-1">`;
    expect(dialogTags(arrowLast)).toEqual([arrowLast]);
  });
});
