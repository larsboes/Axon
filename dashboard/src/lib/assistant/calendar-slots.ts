import type { CalendarEntry } from '$lib/api';

/**
 * Free-slot math for the drawer, in the calendar capability's own time model: every
 * instant is a naive local wall time, an all-day entry covers its dates from midnight, and
 * every end is exclusive. The minute scalar mirrors
 * `capabilities/calendar/src/date.rs::instant_minutes`, so an all-day entry ending on a
 * date and a timed entry starting at that date's 00:00 do not overlap here either.
 *
 * No `Date`/`toISOString` round trip anywhere below `localDate`: `toISOString` is UTC,
 * and between 00:00 and 02:00 in Berlin it names yesterday.
 */

/** The machine's local calendar date, "YYYY-MM-DD". */
export function localDate(at: Date): string {
  return [
    at.getFullYear(),
    String(at.getMonth() + 1).padStart(2, '0'),
    String(at.getDate()).padStart(2, '0'),
  ].join('-');
}

function daysFromCivil(year: number, month: number, day: number): number {
  return Math.floor(Date.UTC(year, month - 1, day) / 86_400_000);
}

function civilFromDays(days: number): string {
  return new Date(days * 86_400_000).toISOString().slice(0, 10);
}

function parseDate(text: string): number | null {
  const match = /^(\d{4})-(\d{2})-(\d{2})$/.exec(text);
  if (!match) return null;
  const [year, month, day] = [Number(match[1]), Number(match[2]), Number(match[3])];
  const days = daysFromCivil(year, month, day);
  return civilFromDays(days) === text ? days : null;
}

/** `date` plus `n` days, pure date arithmetic (no zone, no DST). */
export function addDays(date: string, n: number): string {
  const days = parseDate(date);
  if (days === null) throw new Error(`not a date: ${date}`);
  return civilFromDays(days + n);
}

/** Minutes since 1970-01-01T00:00 local for "YYYY-MM-DD" or "YYYY-MM-DDTHH:MM[:SS]".
 *  Null for anything else, including a string with an offset or `Z`: the calendar does
 *  not store those, and guessing a zone for one would be inventing it. */
export function instantMinutes(text: string): number | null {
  const dateOnly = parseDate(text);
  if (dateOnly !== null) return dateOnly * 1440;
  const match = /^(\d{4}-\d{2}-\d{2})T(\d{2}):(\d{2})(?::(\d{2}))?$/.exec(text);
  if (!match) return null;
  const days = parseDate(match[1]);
  const [hours, minutes, seconds] = [Number(match[2]), Number(match[3]), Number(match[4] ?? 0)];
  if (days === null || hours > 23 || minutes > 59 || seconds > 59) return null;
  return days * 1440 + hours * 60 + minutes;
}

export interface Interval {
  start: number;
  end: number;
}

/** Half-open overlap: touching intervals do not overlap. */
export function overlaps(a: Interval, b: Interval): boolean {
  return a.start < b.end && b.start < a.end;
}

export function entryInterval(entry: Pick<CalendarEntry, 'starts_at' | 'ends_at'>): Interval | null {
  const start = instantMinutes(entry.starts_at);
  const end = instantMinutes(entry.ends_at);
  return start === null || end === null || end <= start ? null : { start, end };
}

/** "YYYY-MM-DDTHH:MM:00" for a minute scalar: what the calendar create accepts. */
export function wallTime(minutes: number): string {
  const days = Math.floor(minutes / 1440);
  const rest = minutes - days * 1440;
  const hh = String(Math.floor(rest / 60)).padStart(2, '0');
  const mm = String(rest % 60).padStart(2, '0');
  return `${civilFromDays(days)}T${hh}:${mm}:00`;
}

export interface SlotCheck {
  startsAt: string;
  endsAt: string;
  conflicts: CalendarEntry[];
}

/** Every entry that overlaps the slot. Entries whose instants cannot be read are
 *  returned separately by `unreadableEntries`, never silently treated as free. */
export function checkSlot(startsAt: string, endsAt: string, entries: CalendarEntry[]): SlotCheck {
  const start = instantMinutes(startsAt);
  const end = instantMinutes(endsAt);
  if (start === null || end === null || end <= start) {
    throw new Error(`not a slot: ${startsAt} – ${endsAt}`);
  }
  const conflicts = entries.filter((entry) => {
    const interval = entryInterval(entry);
    return interval !== null && overlaps({ start, end }, interval);
  });
  return { startsAt, endsAt, conflicts };
}

export function unreadableEntries(entries: CalendarEntry[]): CalendarEntry[] {
  return entries.filter((entry) => entryInterval(entry) === null);
}

export interface FreeSlotSearch {
  date: string;
  /** "HH:MM", inclusive start of the searched window. */
  windowStart: string;
  /** "HH:MM", exclusive end of the searched window. */
  windowEnd: string;
  durationMinutes: number;
  stepMinutes?: number;
  limit?: number;
}

/**
 * Walks the window in steps and returns the first `limit` slots with no overlapping
 * entry. Each returned slot has been checked against every entry by `checkSlot`; a slot
 * is never offered on the strength of a neighbouring slot's check.
 */
export function findFreeSlots(search: FreeSlotSearch, entries: CalendarEntry[]): SlotCheck[] {
  const day = instantMinutes(search.date);
  const [startH, startM] = search.windowStart.split(':').map(Number);
  const [endH, endM] = search.windowEnd.split(':').map(Number);
  if (day === null) throw new Error(`not a date: ${search.date}`);
  const windowStart = day + startH * 60 + startM;
  let windowEnd = day + endH * 60 + endM;
  if (windowEnd <= windowStart) windowEnd += 1440; // window runs past midnight
  const step = search.stepMinutes ?? 30;
  const limit = search.limit ?? 2;

  const free: SlotCheck[] = [];
  for (let start = windowStart; start + search.durationMinutes <= windowEnd; start += step) {
    const check = checkSlot(wallTime(start), wallTime(start + search.durationMinutes), entries);
    if (check.conflicts.length === 0) {
      free.push(check);
      if (free.length >= limit) break;
      start += search.durationMinutes - step; // next proposal starts after this one ends
    }
  }
  return free;
}
