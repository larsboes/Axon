// Text tokens clear WCAG AA against the surfaces they are actually used on.
//
// Measured before the change: --text-tertiary was 2.56:1 on a light card, 2.33:1 on the
// light page and 3.67:1 on a dark card, while carrying the meta line on every row of the
// Home ladder. Nothing in the repository stated the requirement, so nothing could catch it.
//
// --warning is deliberately NOT asserted. It is a fill and icon token, held to the 3:1
// non-text bar, and it is used as a text colour in 14 files this pass may not touch. That
// is a real AA failure, recorded here rather than hidden by loosening the rule.

import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";

const APP_CSS = join(import.meta.dir, "../dashboard/src/app.css");

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

const LIGHT = theme(":root");
const DARK = { ...LIGHT, ...theme(":root.dark") };

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
