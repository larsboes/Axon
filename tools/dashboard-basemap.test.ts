// tools/dashboard-basemap.test.ts — the vendored basemap is complete, same-origin, and still
// carries its attribution.
//
// What these guard is not style. It is the three ways this optimisation can silently become a
// pessimisation or a licence breach, none of which show up in a screenshot:
//
//   1. A refresh that leaves `glyphs` or `sprite` pointing back at tiles.openfreemap.org puts
//      ~780 KB and six transatlantic round trips back on the critical path, and the map still
//      looks perfect.
//   2. Inlining the TileJSON is what removes the second hop, and the TileJSON is also the
//      document the OpenStreetMap and OpenMapTiles attribution travelled in. Dropping it is a
//      licence failure that renders as a slightly cleaner map.
//   3. A manifest that names a glyph range no file backs sends the browser to a 404 instead of
//      the upstream fallback, and the label just does not draw.

import { existsSync, readFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { describe, expect, test } from "bun:test";

import { basemapUrl } from "../dashboard/src/lib/map/surface";

const BASEMAP = resolve(import.meta.dir, "..", "dashboard", "static", "basemap");
const read = (name: string) => JSON.parse(readFileSync(join(BASEMAP, name), "utf8"));

interface Manifest {
  fetched_at: string;
  planet_snapshot: string;
  glyphs: string[];
}

describe("the vendored basemap", () => {
  const style = read("style.json") as Record<string, any>;
  const manifest = read("manifest.json") as Manifest;

  test("serves its style, sprite and glyphs from this origin", () => {
    expect(style.glyphs).toBe("/basemap/fonts/{fontstack}/{range}.pbf");
    expect(style.sprite).toBe("/basemap/sprite/ofm");
  });

  test("declares its vector tiles inline, so no TileJSON hop remains", () => {
    const source = style.sources.openmaptiles;
    expect(source.url).toBeUndefined();
    expect(source.tiles?.[0]).toContain("https://tiles.openfreemap.org/planet/");
    expect(source.maxzoom).toBeGreaterThan(0);
  });

  test("keeps the attribution the inlined TileJSON carried", () => {
    const attribution: string = style.sources.openmaptiles.attribution;
    expect(attribution).toContain("openstreetmap.org/copyright");
    expect(attribution).toContain("openmaptiles.org");
  });

  test("has a file behind every glyph range the manifest names", () => {
    const missing = manifest.glyphs.filter((key) => !existsSync(join(BASEMAP, "fonts", `${key}.pbf`)));
    expect(missing).toEqual([]);
    expect(manifest.glyphs.length).toBeGreaterThan(0);
  });

  test("vendors at least one range for every fontstack the style names", () => {
    const named = new Set<string>();
    for (const layer of style.layers as Array<Record<string, any>>) {
      for (const font of layer.layout?.["text-font"] ?? []) named.add(font as string);
    }
    const vendored = new Set(manifest.glyphs.map((key) => key.slice(0, key.lastIndexOf("/"))));
    expect([...named].filter((stack) => !vendored.has(stack))).toEqual([]);
  });

  test("ships every sprite variant MapLibre asks for", () => {
    for (const file of ["ofm.json", "ofm.png", "ofm@2x.json", "ofm@2x.png"]) {
      expect(existsSync(join(BASEMAP, "sprite", file))).toBe(true);
    }
  });
});

describe("basemapUrl", () => {
  const vendored = new Set(["Noto Sans Regular/0-255"]);

  test("leaves a URL that is not a basemap asset alone", () => {
    const tile = "https://tiles.openfreemap.org/planet/x/1/2/3.pbf";
    expect(basemapUrl(tile, vendored)).toBe(tile);
  });

  test("serves a vendored range from this origin", () => {
    expect(basemapUrl("/basemap/fonts/Noto%20Sans%20Regular/0-255.pbf", vendored)).toBe(
      "/basemap/fonts/Noto%20Sans%20Regular/0-255.pbf",
    );
  });

  // The one that keeps a Cyrillic or CJK label rendering at all.
  test("sends a range nobody vendored to the upstream host", () => {
    expect(basemapUrl("/basemap/fonts/Noto%20Sans%20Regular/1024-1279.pbf", vendored)).toBe(
      "https://tiles.openfreemap.org/fonts/Noto%20Sans%20Regular/1024-1279.pbf",
    );
  });

  // MapLibre v6 refuses a root-relative sprite, so `boot()` resolves the style's own URLs
  // against location.href before any Map is constructed. By the time a request reaches here
  // it is absolute, and the fallback still has to recognise it.
  test("recognises a vendored range through an absolute URL", () => {
    const absolute = "http://127.0.0.1:8082/basemap/fonts/Noto%20Sans%20Regular/0-255.pbf";
    expect(basemapUrl(absolute, vendored)).toBe(absolute);
  });

  test("sends an unvendored range upstream through an absolute URL", () => {
    expect(
      basemapUrl("https://lars.example.ts.net/Axon/basemap/fonts/Noto%20Sans%20Bold/1024-1279.pbf", vendored),
    ).toBe("https://tiles.openfreemap.org/fonts/Noto%20Sans%20Bold/1024-1279.pbf");
  });

  test("leaves the style and sprite alone", () => {
    for (const url of ["/basemap/style.json", "http://127.0.0.1:8082/basemap/sprite/ofm@2x.png"]) {
      expect(basemapUrl(url, vendored)).toBe(url);
    }
  });
});
