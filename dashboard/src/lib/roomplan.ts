import { addPluginListener, invoke, type PluginListener } from "@tauri-apps/api/core";

export type RoomPlanConfidence = "high" | "medium" | "low";
export type RoomPlanCaptureMode = "refine_existing" | "new_room";

export interface RoomPlanElement {
  id: string;
  category: string;
  dimensions_m: [number, number, number];
  transform: number[];
  confidence: RoomPlanConfidence;
  source_id: string;
}

export interface RoomPlanReference {
  schema_version: "roomplan-reference/v1";
  revision_id: string;
  room_id: string;
  imported_at: string;
  status: "raw-only";
  asset: {
    format: "usdz";
    byte_length: number;
    sha256: string;
    storage_token: string;
  };
  observation: {
    format: "usdz";
    byte_length: number;
    sha256: string;
    meters_per_unit: number;
    up_axis: "Y";
    export_observation: {
      room_groups: number;
      mesh_assets: number;
      category_counts: Record<string, number>;
    };
    status: string;
    source_contract: string;
  };
}

export interface RoomPlanDraft {
  schema_version: "roomplan-capture/v1";
  draft_id: string;
  created_at: string;
  source: {
    platform: "ios";
    roomplan_version?: string;
    app_version?: string;
  };
  coordinate_system: {
    units: "meters";
    up_axis: "Y";
    handedness?: string;
  };
  room: {
    id: string;
    surfaces: RoomPlanElement[];
    openings: RoomPlanElement[];
    objects: RoomPlanElement[];
  };
  assets: Array<{
    asset_id: string;
    role: "parametric" | "mesh" | "model";
    format: "usdz";
    byte_length: number;
    sha256: string;
    storage_token: string;
  }>;
  provenance: {
    capture_mode?: RoomPlanCaptureMode;
    parent_revision_id?: string;
    capture_started_at?: string;
    capture_finished_at?: string;
  };
}

export type RoomPlanEvent =
  | { phase: "capturing" | "building"; draft_id?: string }
  | { phase: "cancelled"; draft_id?: string }
  | { draft: RoomPlanDraft };

export function isTauriMobile(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

export function isRoomPlanPhone(): boolean {
  if (!isTauriMobile()) return false;
  return /iPhone|iPad|iPod/.test(window.navigator.userAgent);
}

export async function isRoomPlanAvailable(): Promise<boolean> {
  if (!isTauriMobile()) return false;
  const result = await invoke<{ available: boolean }>("plugin:roomplan|isAvailable");
  return result.available;
}

export async function currentRoomPlanState(): Promise<{
  draft: RoomPlanDraft | null;
  pending: RoomPlanDraft | null;
  reference: RoomPlanReference | null;
}> {
  if (!isTauriMobile()) return { draft: null, pending: null, reference: null };
  return invoke("plugin:roomplan|currentCapture");
}

export async function importLegacyRoomPlanReference(): Promise<RoomPlanReference | null> {
  if (!isTauriMobile()) return null;
  const result = await invoke<{ reference: RoomPlanReference | null }>(
    "plugin:roomplan|importLegacyReference",
  );
  return result.reference;
}

export async function acceptPendingRoomPlan(): Promise<void> {
  if (!isTauriMobile()) return;
  await invoke("plugin:roomplan|acceptPending");
}

export async function rejectPendingRoomPlan(): Promise<void> {
  if (!isTauriMobile()) return;
  await invoke("plugin:roomplan|rejectPending");
}

export async function previewImportedRoomPlanReference(): Promise<void> {
  if (!isTauriMobile()) return;
  await invoke("plugin:roomplan|previewReference");
}

export type RoomPlanDiffChange = {
  kind: "added" | "removed" | "changed";
  collection: "surfaces" | "openings" | "objects";
  category: string;
  before?: RoomPlanElement;
  after?: RoomPlanElement;
};

export type RoomPlanDiff = {
  added: RoomPlanDiffChange[];
  removed: RoomPlanDiffChange[];
  changed: RoomPlanDiffChange[];
};

function elementPosition(element: RoomPlanElement): [number, number, number] {
  return [element.transform[12] ?? 0, element.transform[13] ?? 0, element.transform[14] ?? 0];
}

function elementDistance(a: RoomPlanElement, b: RoomPlanElement): number {
  const pa = elementPosition(a);
  const pb = elementPosition(b);
  return Math.hypot(pa[0] - pb[0], pa[1] - pb[1], pa[2] - pb[2]);
}

function dimensionDistance(a: RoomPlanElement, b: RoomPlanElement): number {
  return Math.max(...a.dimensions_m.map((value, index) => Math.abs(value - b.dimensions_m[index])));
}

export function compareRoomPlanDrafts(base: RoomPlanDraft, next: RoomPlanDraft): RoomPlanDiff {
  const diff: RoomPlanDiff = { added: [], removed: [], changed: [] };
  const collections: Array<"surfaces" | "openings" | "objects"> = [
    "surfaces",
    "openings",
    "objects",
  ];

  for (const collection of collections) {
    const oldElements = [...base.room[collection]];
    const newElements = [...next.room[collection]];
    const used = new Set<number>();

    for (const after of newElements) {
      let match = oldElements.findIndex(
        (before, index) =>
          !used.has(index) &&
          before.source_id === after.source_id &&
          before.category === after.category,
      );
      if (match < 0) {
        let score = Infinity;
        oldElements.forEach((before, index) => {
          if (used.has(index) || before.category !== after.category) return;
          const candidate = elementDistance(before, after) + dimensionDistance(before, after);
          if (candidate < score && elementDistance(before, after) <= 0.4 && dimensionDistance(before, after) <= 0.4) {
            score = candidate;
            match = index;
          }
        });
      }

      if (match < 0) {
        diff.added.push({ kind: "added", collection, category: after.category, after });
        continue;
      }
      used.add(match);
      const before = oldElements[match];
      if (elementDistance(before, after) > 0.05 || dimensionDistance(before, after) > 0.05 || before.confidence !== after.confidence) {
        diff.changed.push({ kind: "changed", collection, category: after.category, before, after });
      }
    }

    oldElements.forEach((before, index) => {
      if (!used.has(index)) diff.removed.push({ kind: "removed", collection, category: before.category, before });
    });
  }
  return diff;
}

export async function startRoomPlanCapture(
  includeMesh = false,
  mode: RoomPlanCaptureMode = "new_room",
): Promise<{ status: string; draft_id: string }> {
  return invoke("plugin:roomplan|startCapture", { includeMesh, mode });
}

export async function stopRoomPlanCapture(): Promise<{ status: string }> {
  return invoke("plugin:roomplan|stopCapture");
}

export async function cancelRoomPlanCapture(): Promise<{ status: string }> {
  return invoke("plugin:roomplan|cancelCapture");
}

export function listenRoomPlan<T>(
  event: "capture-progress" | "capture-completed" | "capture-cancelled" | "capture-failed",
  handler: (payload: T) => void,
): Promise<PluginListener> {
  return addPluginListener<T>("roomplan", event, handler);
}
