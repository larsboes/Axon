import { describe, expect, test } from 'bun:test';
import { meetupsOf, presentOn, whoIsAround } from '../src/lib/travel/who-is-around';
import type { PeopleLayer, PlanItem } from '../src/lib/api';

const BONN: [number, number] = [50.7374, 7.0982];
const pin = (person: string, lat: number, lon: number, since: string | null = null, until: string | null = null) => ({
  type: 'Feature' as const,
  geometry: { type: 'Point' as const, coordinates: [lon, lat] as [number, number] },
  properties: { id: person, person, place_name: 'P', since, until, confidence_bp: 5000, source: 'vault-home' },
});

describe('whoIsAround', () => {
  const layer = {
    type: 'FeatureCollection',
    features: [
      pin('Köln friend', 50.9375, 6.9603), // ~24 km
      pin('Berlin friend', 52.52, 13.405), // ~480 km
      pin('Moved away', 50.74, 7.1, null, '2026-09-01'),
      pin('Bonn friend', 50.735, 7.1),
      pin('Bonn friend', 50.9, 7.0), // second, farther row for the same person
    ],
  } as unknown as PeopleLayer;

  test('people within 50 km on the day, nearest first, one row each', () => {
    const around = whoIsAround(BONN, '2026-10-14', layer).map((p) => p.person);
    expect(around).toEqual(['Bonn friend', 'Köln friend']);
  });

  test('a leg with no coordinate answers nobody rather than guessing', () => {
    expect(whoIsAround(null, '2026-10-14', layer)).toEqual([]);
  });
});

describe('presentOn', () => {
  test('open ends are open, and no day means any day', () => {
    expect(presentOn(null, null, '2026-10-14')).toBe(true);
    expect(presentOn('2026-10-15', null, '2026-10-14')).toBe(false);
    expect(presentOn(null, '2026-10-13', '2026-10-14')).toBe(false);
    expect(presentOn('2026-10-01', '2026-10-31', null)).toBe(true);
  });
});

describe('meetupsOf', () => {
  const item = (id: string, payload: unknown, day: string | null = null) =>
    ({ id, plan_id: 'p', item_type: 'activity', day, external_id: id, title: id, payload, created_at: '0' }) as PlanItem;

  test('only activities that name people are meetups, dated ones first', () => {
    const meetups = meetupsOf([
      item('museum', { status: 'proposed' }),
      item('later', { with: [{ person: 'B' }] }),
      item('boulder', { with: [{ person: 'A' }], status: 'asked', place: 'Bonn' }, '2026-10-14'),
    ]);
    expect(meetups.map((m) => [m.itemId, m.status, m.people])).toEqual([
      ['boulder', 'asked', ['A']],
      ['later', 'idea', ['B']],
    ]);
  });
});

describe('hostsAround', () => {
  test('only nearby people whose note says host: yes, with their note', async () => {
    const { hostsAround } = await import('../src/lib/travel/who-is-around');
    const layer = {
      type: 'FeatureCollection',
      features: [pin('Ron', 50.735, 7.1), pin('Anna', 50.74, 7.09), pin('Away', 50.74, 7.1, null, '2026-10-01')],
    } as unknown as PeopleLayer;
    const people = [
      { id: 'a', name: 'Ron', mention_count: 0, last_contact: null, met_at: null, home: 'Bonn', host: true, host_note: 'sofa' },
      { id: 'b', name: 'Anna', mention_count: 0, last_contact: null, met_at: null, home: 'Bonn', host: false, host_note: null },
      { id: 'c', name: 'Away', mention_count: 0, last_contact: null, met_at: null, home: 'Bonn', host: true, host_note: null },
    ];
    expect(hostsAround(BONN, '2026-10-14', layer, people).map((h) => [h.person, h.note])).toEqual([['Ron', 'sofa']]);
  });
});
