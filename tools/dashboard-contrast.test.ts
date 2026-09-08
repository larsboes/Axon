// Text tokens clear WCAG AA against the surfaces they are actually used on.
//
// Measured before the change: --text-tertiary was 2.56:1 on a light card, 2.33:1 on the
// light page and 3.67:1 on a dark card, while carrying the meta line on every row of the
// Home ladder. Nothing in the repository stated the requirement, so nothing could catch it.
//
// --warning is deliberately NOT asserted against the 4.5:1 text bar. It is a fill, stroke
// and border token, held to the 3:1 non-text bar. The text half is --warning-ink, and the
// split used to be a comment: 48 `color: var(--warning)` declarations across 19 files
// carried words at 3.19:1 on white. They were swapped on 2026-09-07 and the last test in
// this file is what keeps them swapped, because a rule nothing checks is how the first
// batch got there.

import { describe, expect, test } from "bun:test";
import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";

const APP_CSS = join(import.meta.dir, "../dashboard/src/app.css");
const SRC = join(import.meta.dir, "../dashboard/src");

/** Every .svelte and .css file under dashboard/src, path-relative to it. */
function styleSources(dir: string = SRC, prefix = ""): string[] {
  const found: string[] = [];
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const rel = prefix ? `${prefix}/${entry.name}` : entry.name;
    if (entry.isDirectory()) found.push(...styleSources(join(dir, entry.name), rel));
    else if (/\.(svelte|css)$/.test(entry.name)) found.push(rel);
  }
  return found;
}

const channel = (value: number): number => {
  const s = value / 255;
  return s <= 0.03928 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4;
};

function luminance(hex: string): number {
  const h = hex.replace("#", "");
  const full = h.length === 3 ? [...h].map((c) => c + c).join("") : h;
  const [r, g, b] = [0, 2, 4].map((i) => parseInt(full.slice(i, i + 2), 16));
  return 0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b);
}

function contrast(a: string, b: string): number {
  const [x, y] = [luminance(a), luminance(b)].sort((p, q) => q - p);
  return (x + 0.05) / (y + 0.05);
}

/** `a` painted at `alpha` over `b`, as the browser composites it. */
function composite(a: string, alpha: number, b: string): string {
  const parse = (hex: string) => {
    const h = hex.replace("#", "");
    const full = h.length === 3 ? [...h].map((c) => c + c).join("") : h;
    return [0, 2, 4].map((i) => parseInt(full.slice(i, i + 2), 16));
  };
  const [fg, bg] = [parse(a), parse(b)];
  return `#${[0, 1, 2]
    .map((i) => Math.round(fg[i] * alpha + bg[i] * (1 - alpha)).toString(16).padStart(2, "0"))
    .join("")}`;
}

/** Reads one selector's block out of app.css and returns its hex-valued properties. */
function theme(selector: string): Record<string, string> {
  const css = readFileSync(APP_CSS, "utf8");
  const start = css.indexOf(`${selector} {`);
  if (start < 0) throw new Error(`app.css has no ${selector} block`);
  const block = css.slice(start, css.indexOf("\n}", start));
  const values: Record<string, string> = {};
  for (const match of block.matchAll(/(--[a-z0-9-]+):\s*(#[0-9a-f]{3,8})\s*;/gi)) {
    values[match[1]] = match[2];
  }
  return values;
}

/** The alpha `--primary-soft` is declared with, e.g. `rgb(14 116 144 / 8%)` -> 0.08. */
function softAlpha(selector: string): number {
  const css = readFileSync(APP_CSS, "utf8");
  const start = css.indexOf(`${selector} {`);
  const block = css.slice(start, css.indexOf("\n}", start));
  const match = /--primary-soft:\s*rgb\([^)]*\/\s*([\d.]+)%\s*\)/.exec(block);
  if (!match) throw new Error(`${selector} declares no rgb() --primary-soft`);
  return Number(match[1]) / 100;
}

const LIGHT = theme(":root");
const DARK = { ...LIGHT, ...theme(":root.dark") };
const SOFT_ALPHA = { light: softAlpha(":root"), dark: softAlpha(":root.dark") };

describe("contrast helper", () => {
  test("agrees with the two reference ratios everyone knows", () => {
    expect(contrast("#ffffff", "#000000")).toBeCloseTo(21, 1);
    expect(contrast("#ffffff", "#ffffff")).toBeCloseTo(1, 5);
  });
});

describe.each([
  ["light", LIGHT],
  ["dark", DARK],
])("%s theme", (name, tokens) => {
  const surfaces = [
    ["card", "--card-bg"],
    ["page", "--page-bg"],
  ] as const;

  test("the block parsed and carries the tokens under test", () => {
    for (const key of [
      "--card-bg", "--page-bg", "--text-primary", "--text-secondary",
      "--text-tertiary", "--warning-ink", "--rule",
    ]) {
      expect([name, key, typeof tokens[key]]).toEqual([name, key, "string"]);
    }
  });

  for (const [surfaceName, surfaceKey] of surfaces) {
    test(`text tokens clear 4.5:1 on the ${surfaceName}`, () => {
      const failures = ["--text-primary", "--text-secondary", "--text-tertiary", "--warning-ink"]
        .map((key) => [key, contrast(tokens[surfaceKey], tokens[key])] as const)
        .filter(([, ratio]) => ratio < 4.5)
        .map(([key, ratio]) => `${key} ${ratio.toFixed(2)}:1`);
      expect(failures).toEqual([]);
    });
  }

  test("--primary clears 4.5:1 on the card, since links and headings wear it", () => {
    expect(contrast(tokens["--card-bg"], tokens["--primary"])).toBeGreaterThanOrEqual(4.5);
  });

  test("--rule is visible as a line without reading as text", () => {
    // A structural line: the 3:1 non-text bar does not apply to a hairline separator,
    // but it has to be distinguishable from the surface at all.
    expect(contrast(tokens["--card-bg"], tokens["--rule"])).toBeGreaterThanOrEqual(1.4);
  });

  test("the band spine's two loud tones clear the 3:1 non-text bar", () => {
    // alarm and now carry the top of the ladder. owed and offer are deliberately
    // recessive, and no band is ever identified by colour alone — the break above a
    // band renders its name in words.
    for (const key of ["--band-alarm", "--band-now"]) {
      expect([key, contrast(tokens["--card-bg"], tokens[key]) >= 3]).toEqual([key, true]);
    }
  });
});

describe("the warning token split is enforced, not described", () => {
  // The negative lookbehind is what separates `color:` from `background-color:` and
  // `border-color:`, which are the non-text uses --warning is correct for.
  const asText = /(?<!-)color:\s*var\(--warning\)/;

  test("nothing paints text with --warning; --warning-ink is the text half", () => {
    const offenders = styleSources()
      .filter((rel) => asText.test(readFileSync(join(SRC, rel), "utf8")))
      .sort();
    expect(offenders).toEqual([]);
  });

  test("--warning itself still fails the text bar, which is why the rule exists", () => {
    // If this ever passes, --warning was changed and the rule above can be reconsidered
    // rather than silently kept. Recorded as a fact about the token, not a wish.
    expect(contrast(LIGHT["--card-bg"], LIGHT["--warning"])).toBeLessThan(4.5);
  });

  test("--warning clears the 3:1 non-text bar it is actually held to", () => {
    for (const [name, tokens] of [["light", LIGHT], ["dark", DARK]] as const) {
      expect([name, contrast(tokens["--card-bg"], tokens["--warning"]) >= 3]).toEqual([name, true]);
    }
  });
});

// A focus indicator a keyboard reader cannot see is the same defect as body text at
// 2.5:1, and it hides in the same way: the rule looks deliberate, and nothing measures it.
//
// Three components drew their focus state as `background: var(--primary-soft)` with
// `outline: none` beside it, sharing the rule with `:hover`. Measured 2026-09-08: the tint
// is 1.11:1 against a light card and 1.21:1 against a dark one, where WCAG 1.4.11 asks a
// non-text indicator for 3:1 — and the `outline: none` removed the browser's own ring,
// while two of the three sit inside a scrolling container whose overflow clips the shared
// `--focus-ring` box-shadow away. There was nothing left to see.
//
// They now draw `outline: 2px solid var(--primary)` inset by 2px, the idiom
// `$lib/rail/RailSection.svelte` already used. Both halves are asserted below: the ring's
// ratio, and the source rule that stops the tint-only form coming back.

describe("focus indicators are visible", () => {
  const SURFACES = ["--card-bg", "--page-bg", "--surface"] as const;

  describe.each([
    ["light", LIGHT, SOFT_ALPHA.light],
    ["dark", DARK, SOFT_ALPHA.dark],
  ])("%s theme", (name, tokens, alpha) => {
    test("--primary, which every ring is drawn in, clears 3:1 on each surface", () => {
      const failures = SURFACES.map(
        (key) => [key, contrast(tokens[key], tokens["--primary"])] as const,
      )
        .filter(([, ratio]) => ratio < 3)
        .map(([key, ratio]) => `${key} ${ratio.toFixed(2)}:1`);
      expect([name, failures]).toEqual([name, []]);
    });

    test("the ring stays visible on a control the same state has tinted", () => {
      // A focused control wears --primary-soft AND the ring, so the ring's real
      // background is the tint, not the bare surface.
      const failures = SURFACES.map((key) => {
        const tint = composite(tokens["--primary"], alpha, tokens[key]);
        return [key, contrast(tint, tokens["--primary"])] as const;
      })
        .filter(([, ratio]) => ratio < 3)
        .map(([key, ratio]) => `${key} ${ratio.toFixed(2)}:1`);
      expect([name, failures]).toEqual([name, []]);
    });

    test("--primary-soft on its own is nowhere near an indicator", () => {
      // Recorded as a fact about the token, not a wish. If this ever fails, the tint was
      // changed and the source rule below can be reconsidered rather than silently kept.
      for (const key of SURFACES) {
        const tint = composite(tokens["--primary"], alpha, tokens[key]);
        expect([name, key, contrast(tint, tokens[key]) < 3]).toEqual([name, key, true]);
      }
    });
  });

  /** Every `selector { … }` rule in a file's stylesheet, comments removed. */
  function rules(rel: string): { selector: string; body: string }[] {
    const text = readFileSync(join(SRC, rel), "utf8");
    const style = rel.endsWith(".css")
      ? text
      : (text.match(/<style[^>]*>([\s\S]*?)<\/style>/g) ?? []).join("\n");
    const bare = style.replace(/\/\*[\s\S]*?\*\//g, "");
    return [...bare.matchAll(/([^{}]+)\{([^{}]*)\}/g)].map((match) => ({
      selector: match[1].trim(),
      body: match[2],
    }));
  }

  test("the rule reader finds the shared ring, so a passing run means something", () => {
    const shared = rules("app.css").filter((rule) => rule.selector.includes(":focus-visible"));
    expect(shared).toHaveLength(1);
    expect(shared[0].body).toContain("var(--focus-ring)");
  });

  /**
   * True when a block that says `outline: none` still leaves something to see.
   *
   * A replacement has to be something a reader can see: the shared ring, or a
   * component's own box-shadow declared in the same block. `box-shadow: none` is not
   * one — it cancels the shared ring too, so a block holding both `none`s draws no
   * indicator at all.
   */
  function drawsSomethingInstead(body: string): boolean {
    return /box-shadow:(?!\s*none\b)/.test(body);
  }

  test("a block that cancels both the outline and the ring is not a replacement", () => {
    // The known-bad input this rule exists to reject. Without it the rule below passed
    // `outline: none; box-shadow: none;` — verified 2026-09-08 by planting exactly that
    // on `$lib/travel/PlaceField.svelte`'s `button:focus-visible` and watching it go green.
    expect(drawsSomethingInstead("outline: none; box-shadow: none;")).toBe(false);
    expect(drawsSomethingInstead("outline: none;")).toBe(false);
    expect(drawsSomethingInstead("outline: none; box-shadow: var(--focus-ring);")).toBe(true);
    expect(drawsSomethingInstead("outline: none; box-shadow: 0 0 0 2px var(--primary);")).toBe(true);
  });

  test("no :focus-visible rule removes the outline without putting one back", () => {
    const offenders: string[] = [];
    for (const rel of styleSources()) {
      for (const { selector, body } of rules(rel)) {
        if (!selector.includes(":focus-visible")) continue;
        if (!/outline:\s*none/.test(body)) continue;
        if (drawsSomethingInstead(body)) continue;
        offenders.push(`${rel}: ${selector.replace(/\s+/g, " ")}`);
      }
    }
    expect(offenders.sort()).toEqual([]);
  });
});
