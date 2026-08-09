// Tests for tools/lib/demo-data.ts (#168).
//
// Two properties, and neither is about the code being correct in the ordinary sense.
//
// DETERMINISM is what makes a published demo reviewable: two builds of the same commit have to
// produce the same corpus, or every rebuild is an unreadable diff and nobody can tell a data
// change from a code change.
//
// THE VOCABULARY SWEEP is the first of the two guarantees behind "no real data on the public
// page". The second is tools/check-site-payload.sh, which scans the built site. This one runs
// at the source: if a generator can only draw from VOCABULARY, and nothing in VOCABULARY looks
// like a real person's contact details, then the corpus cannot contain any — checked here so
// the failure lands on whoever adds the string, not on a deploy six commits later.

import { describe, expect, test } from "bun:test";
import { addDays, at, daysBetween, monthStart, Rng, VOCABULARY, email, vocabularyStrings } from "./demo-data.ts";

describe("Rng", () => {
  test("the same seed replays the same sequence", () => {
    const a = new Rng("axon-demo-v1");
    const b = new Rng("axon-demo-v1");
    const draw = (r: Rng) => [r.next(), r.int(0, 1000), r.pick([1, 2, 3, 4, 5]), r.amountCents(100, 90_000)];
    expect(draw(a)).toEqual(draw(b));
  });

  test("different seeds diverge", () => {
    const a = Array.from({ length: 8 }, (_, i) => new Rng("one").int(0, 1e6) + i);
    const b = Array.from({ length: 8 }, (_, i) => new Rng("two").int(0, 1e6) + i);
    expect(a).not.toEqual(b);
  });

  test("stays inside its bounds", () => {
    const r = new Rng("bounds");
    for (let i = 0; i < 2000; i++) {
      const n = r.int(-5, 5);
      expect(n).toBeGreaterThanOrEqual(-5);
      expect(n).toBeLessThanOrEqual(5);
      const cents = r.amountCents(200, 5_000);
      expect(cents).toBeGreaterThanOrEqual(190);
      expect(cents).toBeLessThanOrEqual(5_000);
    }
  });

  test("sample() returns distinct members and never over-draws", () => {
    const r = new Rng("sample");
    const picked = r.sample(VOCABULARY.people, 4);
    expect(picked).toHaveLength(4);
    expect(new Set(picked).size).toBe(4);
    expect(r.sample([1, 2], 9)).toHaveLength(2);
  });
});

describe("dates", () => {
  // UTC throughout, so a demo rebuilt in another timezone produces identical bytes. A
  // local-time implementation passes in Europe and shifts a day in Auckland.
  test("addDays crosses a month and a year without drifting", () => {
    expect(addDays("2026-03-16", 20)).toBe("2026-04-05");
    expect(addDays("2026-01-01", -1)).toBe("2025-12-31");
    expect(addDays("2026-02-28", 1)).toBe("2026-03-01"); // 2026 is not a leap year
  });

  test("at() is an RFC3339 instant in UTC", () => {
    expect(at("2026-03-16", 9, 30)).toBe("2026-03-16T09:30:00Z");
  });

  test("monthStart walks back whole months", () => {
    expect(monthStart("2026-03-16")).toBe("2026-03-01");
    expect(monthStart("2026-03-16", 3)).toBe("2025-12-01");
  });

  test("daysBetween is signed and symmetric", () => {
    expect(daysBetween("2026-03-01", "2026-03-16")).toBe(15);
    expect(daysBetween("2026-03-16", "2026-03-01")).toBe(-15);
  });
});

describe("the vocabulary contains nothing real", () => {
  const strings = vocabularyStrings();

  test("is not empty, so the sweep below is actually sweeping something", () => {
    expect(strings.length).toBeGreaterThan(50);
  });

  test("holds no email address, real or otherwise", () => {
    // Addresses are built by email() at use time and always land on example.org. A literal
    // one sitting in the vocabulary is how a real address gets in.
    expect(strings.filter((s) => /@/.test(s))).toEqual([]);
  });

  test("holds nothing shaped like an IBAN, a card number or a phone number", () => {
    for (const s of strings) {
      expect(s).not.toMatch(/\b[A-Z]{2}[0-9]{2}[A-Z0-9]{10,}\b/);
      expect(s).not.toMatch(/\b(?:\d[ -]?){13,19}\b/);
      expect(s).not.toMatch(/\+\d{6,}/);
    }
  });

  test("holds no filesystem path, host or URL", () => {
    for (const s of strings) {
      expect(s).not.toMatch(/^(~|\/)|\/(Users|home)\//);
      expect(s).not.toMatch(/https?:\/\//);
      expect(s).not.toMatch(/\.(ts\.net|local|internal)\b/);
    }
  });

  test("names no deployment instance", () => {
    for (const s of strings) {
      expect(s.toLowerCase()).not.toMatch(/axon-personal|axon-family|axon-work/);
    }
  });

  // The home city is the one place the corpus could accidentally point somewhere. It is
  // asserted to be a member of the same public city list rather than a value of its own,
  // so adding a home means picking one of the six already reviewed.
  test("the demo's home is one of the declared cities", () => {
    expect(VOCABULARY.cities.map((c) => c.name)).toContain(VOCABULARY.home.name);
  });

  test("every city has coordinates a map can render", () => {
    for (const c of VOCABULARY.cities) {
      expect(Math.abs(c.latitude)).toBeLessThanOrEqual(90);
      expect(Math.abs(c.longitude)).toBeLessThanOrEqual(180);
      expect(c.latitude === 0 && c.longitude === 0).toBe(false);
    }
  });

  test("every trip names a city that exists in the list", () => {
    const known = new Set(VOCABULARY.cities.map((c) => c.name));
    for (const t of VOCABULARY.trips) expect(known.has(t.city)).toBe(true);
  });
});

describe("email()", () => {
  test("always lands on a reserved documentation domain", () => {
    for (const person of VOCABULARY.people) expect(email(person)).toMatch(/@example\.org$/);
  });

  test("produces something an address parser accepts", () => {
    expect(email("Mara Velten")).toBe("mara.velten@example.org");
  });
});
