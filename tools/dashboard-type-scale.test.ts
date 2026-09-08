// A number that is already a scale step, written as a number.
//
// `app.css` declares eight named type steps and the README rules that the token layer is
// the whole design system. Measured in `dashboard/src/` on 2026-09-08: 883 `font-size`
// declarations, 851 of them carrying a literal, and 333 of THOSE were EXACTLY one of the
// eight steps — `0.75rem` where `var(--text-xs)` was meant, 332 of those in files this
// sweep could reach. That is not a typography decision anybody made; it is the same
// decision made 333 times without the name, and the cost is that changing the scale
// changes nothing.
//
// The other 518 are left alone on purpose. `0.72rem` is not `--text-xs` rounded, it is a
// value somebody chose, and substituting the nearest step would be a silent redesign of
// surfaces this test cannot see. They are counted instead, and the count is a ratchet:
// tokenising one is welcome, adding a 519th is not.
//
// Two rules, and they answer different questions:
//
//   exact steps    zero tolerance. A literal that equals a step is a token that was not
//                  written, and it is mechanical to fix.
//   off-scale      a ceiling, not zero. Every one of the 518 is a judgement call that
//                  belongs to whoever made it, and this file cannot make it for them.

import { describe, expect, test } from "bun:test";
import { readdirSync, readFileSync, statSync } from "node:fs";
import { extname, join } from "node:path";

const SRC = join(import.meta.dir, "../dashboard/src");
const APP_CSS = join(SRC, "app.css");

function sources(dir: string): string[] {
  return readdirSync(dir).flatMap((name) => {
    const full = join(dir, name);
    if (statSync(full).isDirectory()) return sources(full);
    return [".svelte", ".css"].includes(extname(name)) ? [full] : [];
  });
}

const relative = (file: string) => file.slice(SRC.length + 1);
const lineOf = (text: string, index: number) => text.slice(0, index).split("\n").length;

/**
 * The escape list, one entry, with the stream that owes the fix.
 *
 * A second entry fails the last test in this file. That is the point: this list is for a
 * file another branch holds open right now, not for a defect somebody would rather keep.
 */
const KNOWN: Record<string, string> = {
  "lib/home/rows/FinanceRow.svelte": "0.75rem (owner: the Home rows stream, 2026-09-08)",
};

/** The eight steps, read from app.css rather than copied, keyed by their exact value. */
function scaleSteps(): Map<string, string> {
  const css = readFileSync(APP_CSS, "utf8");
  const steps = new Map<string, string>();
  for (const match of css.matchAll(/(--text-[a-z0-9]+):\s*([\d.]+rem)\s*;/g)) {
    steps.set(match[2], match[1]);
  }
  return steps;
}

/** `.75rem` and `0.75rem` are the same length written two ways. */
const normalise = (value: string) => (value.startsWith(".") ? `0${value}` : value);

/** Every `font-size: <value>;` in a file, with the offset it was found at. */
function declarations(text: string): { value: string; at: number }[] {
  return [...text.matchAll(/font-size:\s*([^;]+);/g)].map((match) => ({
    value: match[1].trim(),
    at: match.index,
  }));
}

/**
 * True when the value still names a length after every `var()` is taken out of it.
 *
 * `clamp(var(--text-xl), 2.6vw, var(--text-2xl))` is fully tokenised — the `2.6vw` is the
 * fluid middle, not a size — and must not be counted as a literal. `clamp(2rem, 3.25vw,
 * 3.35rem)` is not, and must.
 */
function holdsLiteral(value: string): boolean {
  return /\d*\.?\d+\s*(rem|em|px|pt)\b/.test(value.replace(/var\([^)]*\)/g, ""));
}

const STEPS = scaleSteps();
const FILES = sources(SRC);

describe("the scale itself", () => {
  test("app.css declares eight steps and this file can read them", () => {
    expect([...STEPS.values()].sort()).toEqual([
      "--text-2xl", "--text-2xs", "--text-base", "--text-lg",
      "--text-md", "--text-sm", "--text-xl", "--text-xs",
    ]);
  });

  test("the two spellings of the same length agree", () => {
    expect(normalise(".75rem")).toBe("0.75rem");
    expect(STEPS.get(normalise(".75rem"))).toBe("--text-xs");
  });

  test("a fully tokenised clamp is not a literal, and a bare one is", () => {
    expect(holdsLiteral("clamp(var(--text-xl), 2.6vw, var(--text-2xl))")).toBe(false);
    expect(holdsLiteral("var(--text-sm)")).toBe(false);
    expect(holdsLiteral("inherit")).toBe(false);
    expect(holdsLiteral("clamp(2rem, 3.25vw, 3.35rem)")).toBe(true);
    expect(holdsLiteral("11px")).toBe(true);
    expect(holdsLiteral(".72rem !important")).toBe(true);
  });

  test("there are sources to scan, so a passing run means something", () => {
    expect(FILES.length).toBeGreaterThan(80);
    const total = FILES.reduce(
      (sum, file) => sum + declarations(readFileSync(file, "utf8")).length,
      0,
    );
    expect(total).toBeGreaterThan(800);
  });
});

describe("a value that is already a step is written as the step", () => {
  test("no font-size literal equals one of the eight named sizes", () => {
    const offenders: string[] = [];
    for (const file of FILES) {
      const rel = relative(file);
      const text = readFileSync(file, "utf8");
      for (const { value, at } of declarations(text)) {
        const token = STEPS.get(normalise(value));
        if (!token) continue;
        const note = KNOWN[rel];
        offenders.push(
          `${rel}:${lineOf(text, at)} ${value} is ${token}${note ? ` [known: ${note}]` : ""}`,
        );
      }
    }
    expect(offenders.filter((line) => !line.includes("[known:")).sort()).toEqual([]);
  });

  test("the escape list stays exactly one entry, so it cannot grow quietly", () => {
    expect(Object.keys(KNOWN)).toHaveLength(1);
  });
});

describe("the off-scale literals are a ceiling, not a target", () => {
  /**
   * 518, counted 2026-09-08 after the substitution above.
   *
   * Lower it when you tokenise one. Raising it is a decision, and it should read as one in
   * the diff — which is the whole reason the number is here rather than derived.
   */
  const BASELINE = 518;

  function offScale(): string[] {
    const found: string[] = [];
    for (const file of FILES) {
      const text = readFileSync(file, "utf8");
      for (const { value, at } of declarations(text)) {
        if (STEPS.has(normalise(value)) || !holdsLiteral(value)) continue;
        found.push(`${relative(file)}:${lineOf(text, at)} ${value}`);
      }
    }
    return found.sort();
  }

  test("the count has not grown", () => {
    const found = offScale();
    // The count is the assertion; the sample is so a failure says WHICH file grew.
    expect([found.length, found.slice(-3)]).toEqual([
      Math.min(found.length, BASELINE),
      found.slice(-3),
    ]);
    expect(found.length).toBeLessThanOrEqual(BASELINE);
  });

  test("the baseline is still roughly where it was, so the counter did not break", () => {
    // A counter that silently stops matching passes the ceiling above with a zero. This
    // is the floor that catches that.
    expect(offScale().length).toBeGreaterThan(BASELINE - 60);
  });
});
