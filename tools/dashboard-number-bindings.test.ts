// A numeric input's bound state is a number, not a string.
//
// `bind:value` on `<input type="number">` does not hand back what was typed. Svelte coerces
// it: `is_numberlike_input(input)` is true for `type="number"` and `type="range"`, and
// `to_number` returns `null` for an empty field and `+value` otherwise
// (svelte/src/internal/client/dom/elements/bindings/input.js). A state declared as `""` and
// bound to such an input therefore holds `number | null` the moment the field is touched,
// and the next `value.trim()` throws `TypeError: value.trim is not a function`.
//
// This is a source scan for the same reason `dashboard-nav-links.test.ts` is one: the defect
// is invisible to every check anybody runs. `svelte-check` types `bind:value` loosely and
// passes; the component renders; the form simply stops saving, silently when the submit is
// fired through `void submit()`. Two of this repository's forms shipped with it.
//
// It lives in tools/ because SvelteKit's generated tsconfig type-checks everything under
// `src/`, where `bun:test` and `import.meta.dir` are not in scope.

import { describe, expect, test } from "bun:test";
import { readdirSync, readFileSync, statSync } from "node:fs";
import { extname, join } from "node:path";

const SRC = join(import.meta.dir, "../dashboard/src");

function sources(dir: string): string[] {
  return readdirSync(dir).flatMap((name) => {
    const full = join(dir, name);
    if (statSync(full).isDirectory()) return sources(full);
    return extname(name) === ".svelte" ? [full] : [];
  });
}

/** Every `<input …>` tag, attributes included, across line breaks. */
const INPUT_TAG = /<input\b[^>]*>/gs;
const NUMBERLIKE = /type=["'](number|range)["']/;
const BOUND = /bind:value=\{([A-Za-z_$][\w$]*)\}/;

/** String methods a coerced value does not have. `.toFixed` is the number's own. */
const STRING_METHODS = ["trim", "toUpperCase", "toLowerCase", "padStart", "padEnd", "charAt"];

const relative = (file: string) => file.slice(SRC.length + 1);

describe("state bound to a numeric input is never a string", () => {
  const bindings = sources(SRC).flatMap((file) => {
    const text = readFileSync(file, "utf8");
    return [...text.matchAll(INPUT_TAG)].flatMap((tag) => {
      if (!NUMBERLIKE.test(tag[0])) return [];
      const bound = BOUND.exec(tag[0]);
      return bound ? [{ file, text, name: bound[1] }] : [];
    });
  });

  test("the scan finds the numeric inputs it is meant to guard", () => {
    // A regex that matches nothing passes every assertion below. This is the tripwire.
    expect(bindings.length).toBeGreaterThan(0);
  });

  test("no numeric binding is declared as a string", () => {
    for (const { file, text, name } of bindings) {
      // The declaration and its initializer in one pattern: reading forward from
      // the declaration would find the NEXT `$state("…")` in the file instead.
      const stringState = new RegExp(
        `\\b(?:let|const)\\s+${name}\\s*=\\s*\\$state\\s*(?:<[^>]*>)?\\s*\\(\\s*["'\`]`,
      );
      expect(
        stringState.test(text),
        `${relative(file)}: ${name} is bound to a numeric input and initialized with a string`,
      ).toBe(false);
    }
  });

  test("no numeric binding is used as a string", () => {
    for (const { file, text, name } of bindings) {
      for (const method of STRING_METHODS) {
        expect(
          text.includes(`${name}.${method}(`),
          `${relative(file)}: ${name}.${method}() runs on a value Svelte coerced to number | null`,
        ).toBe(false);
      }
    }
  });
});
