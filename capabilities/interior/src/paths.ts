/**
 * Every path this capability reads, resolved in one place.
 *
 * The capability is generic; the flat is not. No room, no dimension and no photo lives in
 * this repository — the model tree is hand-maintained data captured on site and belongs to
 * whoever owns the flat, so it sits in the active private overlay and is resolved at runtime
 * through `libs/overlay`, the same rule `tools/lib/paths.sh` implements for the shell tools.
 *
 * `INTERIOR_MODEL_DIR` overrides the lot. Tests use it to point at a scratch model written
 * into a temp directory, which is what lets the rule engine be tested with no overlay present.
 */

import { join, resolve } from "node:path";
import { overlayRoot } from "../../../libs/overlay/overlay.ts";

/** Where a deployment keeps its flats. One overlay may hold more than one. */
const OVERLAY_DATA_SUBDIR = ["data", "wohnung"];

function overlayDataDir(): string {
  const root = overlayRoot();
  if (!root) {
    throw new Error(
      "no private overlay resolved, so there is no room model to read. " +
        "Set INTERIOR_MODEL_DIR to a model directory, or configure axon.local.toml.",
    );
  }
  return join(root, ...OVERLAY_DATA_SUBDIR);
}

export const MODEL_DIR = process.env.INTERIOR_MODEL_DIR
  ? resolve(process.env.INTERIOR_MODEL_DIR)
  : join(overlayDataDir(), "model");

export const OUT_DIR = process.env.INTERIOR_OUT_DIR
  ? resolve(process.env.INTERIOR_OUT_DIR)
  : join(overlayDataDir(), "out");

export const ROOM_YAML = join(MODEL_DIR, "room.yaml");
export const FURNITURE_YAML = join(MODEL_DIR, "furniture.yaml");
export const CONSTRAINTS_YAML = join(MODEL_DIR, "constraints.yaml");
export const LAYOUTS_DIR = join(MODEL_DIR, "layouts");

/** Pinned in package.json so the geometry engine cannot drift under a verified room model. */
export const PASCAL_PKG = "@pascal-app/mcp@0.3.2";
