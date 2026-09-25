import { describe, expect, test } from 'bun:test';
import { groupByPlace, peopleSources } from '../src/lib/people/people-layers';
import type { LocatedPerson } from '../src/lib/api';

const at = (name: string, lat: number | null, lon: number | null, over: Partial<LocatedPerson> = {}): LocatedPerson => ({
  entity_id: name,
  name,
  predicate: 'home_base',
  place: 'Bonn',
  latitude: lat,
  longitude: lon,
  sleeping_option: null,
  sleeping_note: null,
  ...over,
});

describe('groupByPlace', () => {
  const located = [
    at('A', 50.7352, 7.1024),
    at('B', 50.7349, 7.1021, { sleeping_option: 'ask' }),
    at('C', 38.72, -9.14, { place: 'Lisbon', predicate: 'away' }),
    at('D', null, null),
  ];

  test('one group per place, biggest first, visitors and hosts counted', () => {
    const groups = groupByPlace(located);
    expect(groups.map((g) => [g.place, g.people.length, g.hosts, g.visiting])).toEqual([
      ['Bonn', 2, 1, false],
      ['Lisbon', 1, 0, true],
    ]);
  });

  test('only hosts leaves out everyone who cannot host', () => {
    expect(groupByPlace(located, true).map((g) => g.people.map((p) => p.name))).toEqual([['B']]);
  });

  test('a selected place is marked in the map source', () => {
    const groups = groupByPlace(located);
    const features = peopleSources(groups, groups[1].key)['people-points'].features;
    expect(features.map((f) => (f.properties as { selected: number }).selected)).toEqual([0, 1]);
  });
});
