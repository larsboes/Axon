/**
 * The one basemap style URL, shared by every MapLibre surface in this shell.
 *
 * The host is a declared upstream (`upstreams.toml` `[openfreemap]`), already named
 * swappable to self-hosted PMTiles in `capabilities/places/ISA.md` ("Not yet
 * specified"). One constant, so a future swap is one edit.
 */
export const MAP_STYLE_URL = "https://tiles.openfreemap.org/styles/liberty";

/**
 * Phase colors, exactly the ones TripMap's paint expressions use: past travel is
 * recessive gray, upcoming is the cyan pair (darker for lines, lighter for points).
 * Fixed hex rather than app.css tokens on purpose — marks sit on the basemap,
 * which does not follow the app's light/dark theme.
 */
export const PHASE_PAST = "#71717a";
export const PHASE_UPCOMING_LINE = "#0891b2";
export const PHASE_UPCOMING_POINT = "#06b6d4";
