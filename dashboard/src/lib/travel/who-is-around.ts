// Who you know near each leg of a plan, and the meetups the plan holds.
//
// The people come from places' companion register, confirmed rows only
// (GET /places/api/layers/people). Names never leave this machine: the join runs
// in the dashboard, and nothing here is sent to a model (PRD §6.1, C2).

import type { PeopleLayer, PersonFacts, PlanItem, TripStage } from '../api';

/** places' own disclosure radius (capabilities/places/README.md, D4). */
export const AROUND_RADIUS_KM = 50;

export interface AroundPerson {
  person: string;
  placeName: string;
  distanceKm: number;
}

export interface Meetup {
  itemId: string;
  title: string;
  people: string[];
  status: string;
  day: string | null;
  place: string | null;
}

export function haversineKm(a: [number, number], b: [number, number]): number {
  const rad = (d: number) => (d * Math.PI) / 180;
  const dLat = rad(b[0] - a[0]);
  const dLon = rad(b[1] - a[1]);
  const h = Math.sin(dLat / 2) ** 2 + Math.cos(rad(a[0])) * Math.cos(rad(b[0])) * Math.sin(dLon / 2) ** 2;
  return 2 * 6371 * Math.asin(Math.sqrt(h));
}

/** Whether a register row covers `day`. An open start or end is open; no day means any. */
export function presentOn(since: string | null | undefined, until: string | null | undefined, day: string | null): boolean {
  if (!day) return true;
  if (since && since.slice(0, 10) > day) return false;
  if (until && until.slice(0, 10) < day) return false;
  return true;
}

/**
 * People near one leg's destination on its date, nearest first, one row per person.
 * `destination` is `[latitude, longitude]`, or null when the leg has no coordinate
 * and none could be resolved: then the answer is empty, never a guess from the name.
 */
export function whoIsAround(
  destination: [number, number] | null,
  day: string | null,
  layer: PeopleLayer,
): AroundPerson[] {
  if (!destination) return [];
  const nearest = new Map<string, AroundPerson>();
  for (const feature of layer.features) {
    const { person, place_name, since, until } = feature.properties;
    if (!presentOn(since, until, day)) continue;
    const [lon, lat] = feature.geometry.coordinates;
    const distanceKm = haversineKm(destination, [lat, lon]);
    if (distanceKm > AROUND_RADIUS_KM) continue;
    const seen = nearest.get(person);
    if (!seen || distanceKm < seen.distanceKm) nearest.set(person, { person, placeName: place_name, distanceKm });
  }
  return [...nearest.values()].sort((a, b) => a.distanceKm - b.distanceKm);
}

/** The meetups on a plan: `activity` items whose payload names people in `with`. */
export function meetupsOf(items: PlanItem[]): Meetup[] {
  const meetups: Meetup[] = [];
  for (const item of items) {
    if (item.item_type !== 'activity') continue;
    const payload = (item.payload ?? {}) as { with?: Array<{ person?: string }>; status?: string; place?: string };
    const people = (payload.with ?? []).map((entry) => entry.person?.trim() ?? '').filter(Boolean);
    if (!people.length) continue;
    meetups.push({
      itemId: item.id,
      title: item.title,
      people,
      status: payload.status ?? 'idea',
      day: item.day,
      place: payload.place ?? null,
    });
  }
  return meetups.sort((a, b) => (a.day ?? '9999').localeCompare(b.day ?? '9999'));
}

/** The coordinate a leg's destination carries, if any. */
export function stageCoordinate(stage: TripStage): [number, number] | null {
  const { latitude, longitude } = stage.destination;
  return latitude != null && longitude != null ? [latitude, longitude] : null;
}

export interface Host {
  person: string;
  placeName: string;
  distanceKm: number;
  note: string | null;
}

/**
 * People near a leg who said they can host (`host: yes` in their note), nearest first.
 * Where they are comes from the register, so a friend who is away that week drops out
 * and one visiting the city shows up, as long as the row is confirmed.
 */
export function hostsAround(
  destination: [number, number] | null,
  day: string | null,
  layer: PeopleLayer,
  people: PersonFacts[],
): Host[] {
  const hosting = new Map(people.filter((p) => p.host).map((p) => [p.name, p]));
  return whoIsAround(destination, day, layer)
    .filter((around) => hosting.has(around.person))
    .map((around) => ({ ...around, note: hosting.get(around.person)?.host_note ?? null }));
}

/** The stays already on the plan, for the leg's view of where to sleep. */
export function staysOf(items: PlanItem[]): { title: string; checkIn: string | null; checkOut: string | null }[] {
  return items
    .filter((item) => item.item_type === 'stay')
    .map((item) => {
      const payload = (item.payload ?? {}) as { check_in?: string; check_out?: string };
      return { title: item.title, checkIn: payload.check_in ?? null, checkOut: payload.check_out ?? null };
    });
}
