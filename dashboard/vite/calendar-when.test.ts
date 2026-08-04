import { describe, expect, test } from "bun:test";

import {
  shiftDayKey,
  whenError,
  whenOf,
  whenPatch,
  type EntryWhen,
} from "../src/lib/calendar/types";
import type { CalendarEntry } from "../src/lib/api";

function entry(overrides: Partial<CalendarEntry>): CalendarEntry {
  return {
    id: "cal:entry:x",
    kind: "event",
    commitment: "planned",
    title: "Something",
    starts_at: "2026-08-15T16:00:00",
    ends_at: "2026-08-15T22:00:00",
    all_day: false,
    location: null,
    notes: null,
    source: "manual",
    external_id: null,
    rhythm_id: null,
    payload: null,
    created_at: "0",
    updated_at: "0",
    ...overrides,
  } as CalendarEntry;
}

describe("when an entry happens", () => {
  test("a timed entry round-trips unchanged", () => {
    const original = entry({});
    const patch = whenPatch(whenOf(original));
    expect(patch.starts_at).toBe(original.starts_at);
    expect(patch.ends_at).toBe(original.ends_at);
    expect(patch.all_day).toBe(false);
  });

  // The bug this whole module exists to prevent: the store's end is exclusive,
  // the form's is inclusive, and a surface that forgets moves the entry a day.
  test("an all-day entry round-trips without drifting a day", () => {
    const original = entry({
      all_day: true,
      starts_at: "2026-09-15",
      ends_at: "2026-09-16",
    });
    const when = whenOf(original);
    expect(when.endDate).toBe("2026-09-15");
    const patch = whenPatch(when);
    expect(patch.starts_at).toBe("2026-09-15");
    expect(patch.ends_at).toBe("2026-09-16");
  });

  test("a multi-day all-day entry keeps both ends", () => {
    const original = entry({
      all_day: true,
      starts_at: "2026-10-07",
      ends_at: "2026-10-14",
    });
    const when = whenOf(original);
    expect(when.startDate).toBe("2026-10-07");
    expect(when.endDate).toBe("2026-10-13");
    expect(whenPatch(when).ends_at).toBe("2026-10-14");
  });

  test("switching a timed entry to all day drops the times", () => {
    const when: EntryWhen = { ...whenOf(entry({})), allDay: true };
    const patch = whenPatch(when);
    expect(patch.starts_at).toBe("2026-08-15");
    expect(patch.ends_at).toBe("2026-08-16");
  });

  test("switching an all-day entry to timed gets a working default day", () => {
    const original = entry({ all_day: true, starts_at: "2026-09-15", ends_at: "2026-09-16" });
    const when: EntryWhen = { ...whenOf(original), allDay: false };
    const patch = whenPatch(when);
    expect(patch.starts_at).toBe("2026-09-15T09:00:00");
    expect(patch.ends_at).toBe("2026-09-15T10:00:00");
  });

  // A contributed draft may omit all_day. The timestamp already says which it
  // is, and the old blanket default turned a timed draft into an all-day one.
  test("a draft without all_day is read from its timestamp shape", () => {
    expect(whenOf({ starts_at: "2026-08-15T16:00:00", ends_at: "2026-08-15T22:00:00" })).toEqual({
      allDay: false,
      startDate: "2026-08-15",
      endDate: "2026-08-15",
      startTime: "16:00",
      endTime: "22:00",
    });
    expect(whenOf({ starts_at: "2026-09-15", ends_at: "2026-09-16" })).toEqual({
      allDay: true,
      startDate: "2026-09-15",
      endDate: "2026-09-15",
      startTime: "",
      endTime: "",
    });
  });

  test("a day shift crosses a month and a DST boundary correctly", () => {
    expect(shiftDayKey("2026-08-31", 1)).toBe("2026-09-01");
    expect(shiftDayKey("2026-01-01", -1)).toBe("2025-12-31");
    // Europe/Berlin springs forward on 2026-03-29; noon anchoring keeps this exact.
    expect(shiftDayKey("2026-03-29", 1)).toBe("2026-03-30");
    expect(shiftDayKey("2026-10-25", 1)).toBe("2026-10-26");
  });
});

describe("what the store would reject", () => {
  test("a valid entry has no error", () => {
    expect(whenError(whenOf(entry({})))).toBeNull();
  });

  test("an end before the start is refused", () => {
    const when = { ...whenOf(entry({})), endDate: "2026-08-14" };
    expect(whenError(when)).toBe("The end must be on or after the start");
  });

  // Ends are exclusive, so an identical instant is an empty range.
  test("a zero-length timed entry is refused", () => {
    const when = { ...whenOf(entry({})), endTime: "16:00" };
    expect(whenError(when)).toBe("The end must be after the start");
  });

  test("a timed entry missing a time is refused", () => {
    const when = { ...whenOf(entry({})), endTime: "" };
    expect(whenError(when)).toBe("A timed entry needs a start and end time");
  });

  // An all-day entry has no times to check, so it must not inherit that rule.
  test("an all-day entry on one day is fine", () => {
    const when = whenOf(entry({ all_day: true, starts_at: "2026-09-15", ends_at: "2026-09-16" }));
    expect(whenError(when)).toBeNull();
  });
});
