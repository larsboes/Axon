import { describe, expect, test } from 'bun:test';
import { keepSide, mergedName } from '../src/lib/assistant/executor';
import { routeByKeywords } from '../src/lib/assistant/keyword-router';
import type { DuplicateCandidate, DuplicateProfile } from '../src/lib/api';

const profile = (name: string, over: Partial<DuplicateProfile> = {}): DuplicateProfile => ({
  name,
  lives_in: null,
  company: null,
  role: null,
  relation: null,
  emails: null,
  phones: null,
  birthday: null,
  sources: [],
  has_note: false,
  ...over,
});

const candidate = (a: DuplicateProfile, b: DuplicateProfile): DuplicateCandidate => ({
  a: { id: 'a', profile: a },
  b: { id: 'b', profile: b },
  strength: 'partial',
  reasons: [],
  evidence: { same: [], different: [] },
  verdict: null,
});

describe('merge choices', () => {
  test('the side with a note is kept even with fewer details, and the longer name wins', () => {
    const c = candidate(
      profile('Ron Mustermann', { phones: ['+49 1'], emails: ['r@example.org'], birthday: '1990-01-01' }),
      profile('Ron', { has_note: true }),
    );
    expect(keepSide(c)).toBe('b');
    expect(mergedName(c)).toBe('Ron Mustermann');
  });

  test('without a note, the side with more details is kept; a tie keeps a', () => {
    expect(keepSide(candidate(profile('A'), profile('B', { phones: ['1'] })))).toBe('b');
    expect(keepSide(candidate(profile('A'), profile('B')))).toBe('a');
  });
});

describe('routing', () => {
  const context = { pathname: '/', domain: 'general' as const, label: 'Home', contextSummary: '', quickPrompts: [] };
  test('"find duplicates" goes to people', () => {
    expect(routeByKeywords('Find duplicates', context).domain).toBe('people');
    expect(routeByKeywords('merge these contacts', context).domain).toBe('people');
  });
});
