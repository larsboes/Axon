// The row discloses its reason, and the reason stays reachable.
//
// Q96 ruled the ladder discloses by band; B52 said the row does not disclose at all —
// kind, title, rationale and actions all at once. `ListRow` now collapses the meta line
// and opens it on the cursor, hover or focus.
//
// This is a SOURCE assertion, not a rendering one, and the difference matters: the sixth
// silent failure of the night build was a UI change verified as numbers and never looked
// at. What a test can hold here is the part that is easy to regress silently — that the
// collapse never becomes `display: none`, that a pointer-less device is exempt, and that
// all three open states survive an edit. Whether it reads well is a human's call.

import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";

const listRow = readFileSync(join(import.meta.dir, "../dashboard/src/lib/ListRow.svelte"), "utf8");

describe("a row's reason is disclosed, not deleted", () => {
  test("the collapse is a height, so the line stays in the accessibility tree", () => {
    // `display: none` and `visibility: hidden` both take the line out of it. A screen
    // reader that cannot reach the reason is not reading a disclosed row, it is reading
    // a row whose provenance was dropped — which PRD:227 forbids.
    expect(listRow).toContain("grid-template-rows: 0fr");
    expect(/\.meta\s*\{[^}]*display:\s*none/.test(listRow)).toBe(false);
    expect(/\.meta\s*\{[^}]*visibility:\s*hidden/.test(listRow)).toBe(false);
  });

  test("all three ways of asking open it", () => {
    // Descendant selectors, deliberately, and this is the second assertion of the pair:
    // written first as `.row:hover > .body > .meta`, the compiler silently dropped the
    // whole media block — `svelte:element` defeats the child-combinator match — and
    // `svelte-check` reported zero warnings while the built CSS carried no rule at all.
    // Checked against `dashboard/dist` after the change, not just against the source.
    for (const opener of [
      ".row:hover .meta",
      ".row:focus-within .meta",
      ".row.current .meta",
    ]) {
      expect([opener, listRow.includes(opener)]).toEqual([opener, true]);
    }
  });

  test("a device with no hover is exempt", () => {
    // The whole collapse sits inside `pointer: fine`. Without the guard a touch reader
    // would have to guess that a reason exists at all.
    const guard = listRow.indexOf("@media (pointer: fine)");
    expect(guard).toBeGreaterThan(-1);
    expect(listRow.indexOf("grid-template-rows: 0fr")).toBeGreaterThan(guard);
  });

  test("the row that is spent is dimmed, not hidden", () => {
    // `dimmed` carries /feed's decided rows. Same rule as above: opacity, never removal.
    expect(listRow).toContain(".row.dimmed");
    expect(/\.row\.dimmed\s*\{[^}]*display:\s*none/.test(listRow)).toBe(false);
  });
});
