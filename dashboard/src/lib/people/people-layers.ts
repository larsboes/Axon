/**
 * What people look like on the map: one marker per place, with how many people are there
 * on the chosen day.
 *
 * One marker per place rather than per person, because people cluster: most of them share a
 * home city, and 36 markers on one coordinate would be one marker with 35 hidden under it.
 * The data is entities' GET /api/located (PRD Q117); C2, joined here on this machine.
 */

import { MARK_ACCENT, MARK_HALO, MARK_INK, PHASE_UPCOMING_POINT } from "$lib/map/style";
import type { MapFeatureCollection, MapLayerSpec } from "$lib/map/surface";
import type { LocatedPerson } from "$lib/api";

export const PEOPLE_POINTS = "people-points";

export interface PlaceGroup {
  /** The first member's coordinate, which names the group on the map. */
  key: string;
  place: string;
  latitude: number;
  longitude: number;
  people: LocatedPerson[];
  /** Someone here is away from home, not living here. */
  visiting: boolean;
  hosts: number;
}

/** How close two people must be to share a marker: a city, not a street. */
export const SAME_PLACE_KM = 3;

function km(a: [number, number], b: [number, number]): number {
  const rad = (d: number) => (d * Math.PI) / 180;
  const h =
    Math.sin(rad(b[0] - a[0]) / 2) ** 2 +
    Math.cos(rad(a[0])) * Math.cos(rad(b[0])) * Math.sin(rad(b[1] - a[1]) / 2) ** 2;
  return 2 * 6371 * Math.asin(Math.sqrt(h));
}

/** Groups located people by place: a person joins the first group within SAME_PLACE_KM.
 *  Rounding to a grid was tried and split two points 40 m apart across a grid line.
 *  People without a coordinate are left out; the caller counts them from the difference. */
export function groupByPlace(located: LocatedPerson[], onlyHosts = false): PlaceGroup[] {
  const groups: PlaceGroup[] = [];
  for (const person of located) {
    if (person.latitude == null || person.longitude == null) continue;
    const host = person.sleeping_option === "ask" || person.sleeping_option === "yes";
    if (onlyHosts && !host) continue;
    const here: [number, number] = [person.latitude, person.longitude];
    let group = groups.find((g) => km([g.latitude, g.longitude], here) <= SAME_PLACE_KM);
    if (!group) {
      group = {
        key: `${person.latitude.toFixed(4)},${person.longitude.toFixed(4)}`,
        place: person.place,
        latitude: person.latitude,
        longitude: person.longitude,
        people: [],
        visiting: false,
        hosts: 0,
      };
      groups.push(group);
    }
    group.people.push(person);
    group.visiting ||= person.predicate === "away";
    if (host) group.hosts += 1;
  }
  return groups.sort((a, b) => b.people.length - a.people.length);
}

export function peopleSources(groups: PlaceGroup[], selectedKey: string | null): Record<string, MapFeatureCollection> {
  return {
    [PEOPLE_POINTS]: {
      type: "FeatureCollection",
      features: groups.map((group) => ({
        type: "Feature",
        properties: {
          key: group.key,
          label: `${group.place} · ${group.people.length}`,
          count: group.people.length,
          visiting: group.visiting ? 1 : 0,
          selected: group.key === selectedKey ? 1 : 0,
        },
        geometry: { type: "Point", coordinates: [group.longitude, group.latitude] as [number, number] },
      })),
    },
  };
}

/** Refit when the set of places changes, not when a selection does. */
export const peopleFitKey = (groups: PlaceGroup[]): string => groups.map((g) => g.key).join("|");

export const PEOPLE_LAYERS: MapLayerSpec[] = [
  {
    id: PEOPLE_POINTS,
    type: "circle",
    source: PEOPLE_POINTS,
    paint: {
      // Bigger with more people, capped so a home city does not cover its neighbours.
      "circle-radius": ["+", ["case", ["==", ["get", "selected"], 1], 3, 0], ["min", 16, ["+", 6, ["*", 2, ["get", "count"]]]]],
      "circle-color": [
        "case",
        ["==", ["get", "selected"], 1], MARK_ACCENT,
        ["==", ["get", "visiting"], 1], MARK_INK,
        PHASE_UPCOMING_POINT,
      ],
      "circle-opacity": 0.9,
      "circle-stroke-color": MARK_HALO,
      "circle-stroke-width": 2,
    },
  },
  {
    id: "people-labels",
    type: "symbol",
    source: PEOPLE_POINTS,
    layout: {
      "text-field": ["get", "label"],
      "text-size": 12,
      "text-offset": [0, 1.6],
      "text-anchor": "top",
    },
    paint: {
      "text-color": MARK_INK,
      "text-halo-color": MARK_HALO,
      "text-halo-width": 1.5,
    },
  },
];
