/**
 * The mark palette every MapLibre surface in this shell paints with.
 *
 * Fixed hex rather than app.css tokens, on purpose: marks sit on the basemap, which does not
 * follow the app's light/dark theme. A token here would flip a pin to near-white over a
 * near-white raster the moment the shell went dark.
 *
 * The basemap style URL used to live here as one constant. It moved into
 * `./surface.svelte.ts`, which is now the only thing that constructs a Map — the style is
 * vendored at `static/basemap/style.json` (see `tools/fetch-basemap` and that directory's
 * LICENSE.md), so it is no longer an upstream URL a caller could reasonably want to override.
 */

/** Travel phase. Past is recessive gray; upcoming is the cyan pair, darker for lines. */
export const PHASE_PAST = "#71717a";
export const PHASE_UPCOMING_LINE = "#0891b2";
export const PHASE_UPCOMING_POINT = "#06b6d4";

/** Spend. The light-theme primary, because a venue dot is a quantity, not a status. */
export const SPEND_COLOR = "#0e7490";

/** The amber that means "this one". A people pin on /map, the highlighted trip on /travel —
 *  one hue for the thing the panel beside the map is currently about. */
export const MARK_ACCENT = "#d97706";

/** Structure rather than subject: a rail station, and a place inferred from spend alone. */
export const STATION_COLOR = "#3f3f46";
export const PRESENCE_COLOR = "#a1a1aa";

/** Near-black for a trip origin and for label text; white for every stroke and halo. */
export const MARK_INK = "#18181b";
export const MARK_HALO = "#ffffff";
