import { describe, expect, test } from 'bun:test';
import { unseenLows, upcoming } from '../src/lib/travel/fare-alerts';

describe('unseenLows', () => {
  const items = [
    { item_type: 'note', external_id: 'sparpreis-low:Bonn:Stuttgart:2026-10-07T07:00:00:2026-09-26', title: 'Sparpreis new low Bonn Hbf → Stuttgart Hbf' },
    // The pre-2026-09-25 notes compared against the last check, not the lowest.
    { item_type: 'note', external_id: 'sparpreis-drop:8000044:8011160:2026-10-07T08:00:00:2026-09-23', title: 'old' },
    { item_type: 'option_set', external_id: 'sparpreis-low:not-a-note', title: 'x' },
  ];

  test('only unseen new-low notes become alerts', () => {
    expect(unseenLows(items, new Set())).toEqual([
      { id: 'sparpreis-low:Bonn:Stuttgart:2026-10-07T07:00:00:2026-09-26', title: 'Sparpreis new low Bonn Hbf → Stuttgart Hbf' },
    ]);
  });

  test('an announced note is not announced again', () => {
    expect(unseenLows(items, new Set(['sparpreis-low:Bonn:Stuttgart:2026-10-07T07:00:00:2026-09-26']))).toEqual([]);
  });
});

describe('upcoming', () => {
  test('a plan counts until its last day has passed', () => {
    expect(upcoming({ date_start: '2026-10-07', date_end: '2026-10-13' }, '2026-10-13')).toBe(true);
    expect(upcoming({ date_start: '2026-10-07', date_end: '2026-10-13' }, '2026-10-14')).toBe(false);
    expect(upcoming({ date_start: '2026-10-16', date_end: null }, '2026-10-16')).toBe(true);
  });
});
