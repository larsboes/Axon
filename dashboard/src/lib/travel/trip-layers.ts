/**
 * What a trip looks like on the map: the two feature collections and the three layers.
 *
 * This is the half of the old `TripMap.svelte` that was actually about travel. The other
 * half — the lazy import, the observers, the resize dance, the deferred/loading/failed
 * states — was a second copy of `MapView.svelte`'s and is now `$lib/map/surface`.
 * Splitting them is what lets `/travel` keep its own paint expressions while sharing one
 * MapLibre instance with `/map`.
 */

import {
  MARK_ACCENT,
  MARK_HALO,
  MARK_INK,
  PHASE_PAST,
  PHASE_UPCOMING_LINE,
  PHASE_UPCOMING_POINT,
} from "$lib/map/style";
import type { MapFeatureCollection, MapLayerSpec } from "$lib/map/surface";

export interface MapPoint {
  id: string;
  groupId?: string;
  routeId?: string;
  label: string;
  latitude: number;
  longitude: number;
  kind: "origin" | "destination";
  phase?: "upcoming" | "past";
  selected: boolean;
}

export const TRIP_POINTS = "trip-points";
export const TRIP_ROUTE = "trip-route";

function pointCollection(points: MapPoint[]): MapFeatureCollection {
  return {
    type: "FeatureCollection",
    features: points.map((point) => ({
      type: "Feature",
      properties: {
        id: point.id,
        groupId: point.groupId ?? point.id,
        label: point.label,
        kind: point.kind,
        phase: point.phase ?? "upcoming",
        selected: point.selected ? 1 : 0,
      },
      geometry: { type: "Point", coordinates: [point.longitude, point.latitude] as [number, number] },
    })),
  };
}

function routeCollection(points: MapPoint[]): MapFeatureCollection {
  const routes = new Map<string, MapPoint[]>();
  for (const point of points) {
    const routeId = point.routeId ?? "active";
    routes.set(routeId, [...(routes.get(routeId) ?? []), point]);
  }
  return {
    type: "FeatureCollection",
    features: [...routes.entries()]
      .filter(([, routePoints]) => routePoints.length > 1)
      .map(([routeId, routePoints]) => ({
        type: "Feature",
        properties: { routeId, phase: routePoints[0].phase ?? "upcoming" },
        geometry: {
          type: "LineString",
          coordinates: routePoints.map((point): [number, number] => [point.longitude, point.latitude]),
        },
      })),
  };
}

/** Both sources, in the shape `MapSurface` takes. */
export function tripSources(points: MapPoint[]): Record<string, MapFeatureCollection> {
  return { [TRIP_ROUTE]: routeCollection(points), [TRIP_POINTS]: pointCollection(points) };
}

/**
 * The frame key. `MapSurface` refits when this changes and not otherwise, because a trip's
 * point array is rebuilt whenever a row is hovered — same places, different `selected` flag —
 * and refitting on that flew the camera to the bounds it was already at on every mouse move.
 */
export const tripFitKey = (points: MapPoint[]): string => points.map((point) => point.id).join("|");

/** Route first, then points, then labels: MapLibre draws in list order. */
export const TRIP_LAYERS: MapLayerSpec[] = [
  {
    id: TRIP_ROUTE,
    type: "line",
    source: TRIP_ROUTE,
    paint: {
      "line-color": ["match", ["get", "phase"], "past", PHASE_PAST, PHASE_UPCOMING_LINE],
      "line-width": 3,
      "line-opacity": 0.65,
      "line-dasharray": [1.5, 1.5],
    },
  },
  {
    id: TRIP_POINTS,
    type: "circle",
    source: TRIP_POINTS,
    paint: {
      "circle-radius": ["case", ["==", ["get", "selected"], 1], 9, 6],
      "circle-color": [
        "case",
        ["==", ["get", "selected"], 1], MARK_ACCENT,
        ["==", ["get", "phase"], "past"], PHASE_PAST,
        ["==", ["get", "kind"], "origin"], MARK_INK,
        PHASE_UPCOMING_POINT,
      ],
      "circle-stroke-color": MARK_HALO,
      "circle-stroke-width": 2,
    },
  },
  {
    id: "trip-labels",
    type: "symbol",
    source: TRIP_POINTS,
    layout: {
      "text-field": ["get", "label"],
      "text-size": 12,
      "text-offset": [0, 1.4],
      "text-anchor": "top",
    },
    paint: {
      "text-color": MARK_INK,
      "text-halo-color": MARK_HALO,
      "text-halo-width": 1.5,
    },
  },
];
