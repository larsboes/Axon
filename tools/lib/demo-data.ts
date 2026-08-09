// tools/lib/demo-data.ts — a deterministic source of fictional values (#168).
//
// This module knows nothing about capabilities or HTTP. It answers "give me a plausible
// merchant name" and "give me a date eleven days after the anchor", always with the same
// answer for the same seed. tools/demo-seed decides what to do with them.
//
// The split is the point. Every shape in the demo — what a transaction looks like, which
// endpoint accepts it — lives next to the API it targets, in demo-seed.ts. Everything a
// reviewer has to eyeball for accidental realism lives here, in one file that imports
// nothing and can be read top to bottom in a minute.
//
// TWO RULES GOVERN THE VOCABULARY, and tools/lib/demo-data.test.ts enforces both:
//
//   1. Nothing here names a real person, company, or address. Merchants are invented
//      rather than borrowed — "Nordlicht Kaffee" instead of a chain that exists — because
//      a real brand in a screenshot invites a question nobody wants to answer, and because
//      an invented one is unmistakably a fixture the moment you read it.
//   2. Cities are real, and are chosen to be somewhere the principal is not. A map needs
//      coordinates that exist to render at all, and where a public demo says it is going
//      on holiday is not a private fact — but where its author actually lives might be, so
//      the set deliberately excludes it. The published-payload gate (tools/check-site-payload)
//      independently rejects the overlay's own home city if one ever appeared.

/** Deterministic 32-bit PRNG (mulberry32). Chosen for being eight lines and exactly
 *  reproducible across runtimes — a demo whose corpus depends on V8's internals would
 *  regenerate differently in CI than on the machine that reviewed it. */
export class Rng {
  private state: number;

  constructor(seed: string) {
    // FNV-1a over the seed string, so a human-readable seed becomes a 32-bit state.
    let h = 0x811c9dc5;
    for (let i = 0; i < seed.length; i++) {
      h ^= seed.charCodeAt(i);
      h = Math.imul(h, 0x01000193);
    }
    this.state = h >>> 0;
  }

  /** Uniform in [0, 1). */
  next(): number {
    this.state = (this.state + 0x6d2b79f5) >>> 0;
    let t = this.state;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  }

  /** Uniform integer in [min, max]. */
  int(min: number, max: number): number {
    return min + Math.floor(this.next() * (max - min + 1));
  }

  pick<T>(items: readonly T[]): T {
    if (items.length === 0) throw new Error("demo-data: pick() from an empty list");
    return items[Math.floor(this.next() * items.length)];
  }

  /** `count` distinct members, or the whole list when it is shorter. */
  sample<T>(items: readonly T[], count: number): T[] {
    const pool = [...items];
    const out: T[] = [];
    while (out.length < count && pool.length > 0) {
      out.push(pool.splice(Math.floor(this.next() * pool.length), 1)[0]);
    }
    return out;
  }

  bool(trueProbability = 0.5): boolean {
    return this.next() < trueProbability;
  }

  /** A skewed amount in cents. Real spending is not uniform — a few large rents among
   *  many small coffees — and a uniform distribution makes every chart in the demo look
   *  like noise. Cubing the sample pushes the mass toward the floor. */
  amountCents(minCents: number, maxCents: number): number {
    const skew = this.next() ** 3;
    return Math.round((minCents + skew * (maxCents - minCents)) / 10) * 10;
  }
}

// ─── Dates ────────────────────────────────────────────────────────────────────

/** Calendar arithmetic in UTC. The capabilities store instants and the demo's anchor is a
 *  plain date, so every conversion here is deliberately timezone-free — a demo rebuilt in
 *  a different TZ must produce the same bytes, and local-time arithmetic would not. */
export function addDays(iso: string, days: number): string {
  const d = new Date(`${iso}T00:00:00Z`);
  d.setUTCDate(d.getUTCDate() + days);
  return d.toISOString().slice(0, 10);
}

/** An RFC3339 instant at a given hour on a date, always UTC. */
export function at(iso: string, hour: number, minute = 0): string {
  const d = new Date(`${iso}T00:00:00Z`);
  d.setUTCHours(hour, minute, 0, 0);
  return d.toISOString().replace(".000Z", "Z");
}

/** First day of the month `back` months before `iso`. */
export function monthStart(iso: string, back = 0): string {
  const d = new Date(`${iso}T00:00:00Z`);
  d.setUTCDate(1);
  d.setUTCMonth(d.getUTCMonth() - back);
  return d.toISOString().slice(0, 10);
}

export function daysBetween(from: string, to: string): number {
  const a = Date.parse(`${from}T00:00:00Z`);
  const b = Date.parse(`${to}T00:00:00Z`);
  return Math.round((b - a) / 86_400_000);
}

// ─── Vocabulary ───────────────────────────────────────────────────────────────
//
// Exported as one object so demo-data.test.ts can sweep every string in it against the
// same forbidden-pattern set the published-payload gate uses. A generator that reaches
// past this object for a literal defeats that sweep, so they do not.

export const VOCABULARY = {
  /** Invented people. Used as travellers and as the counterparty on a shared expense. */
  people: [
    "Mara Velten",
    "Tomas Iversen",
    "Juno Halvorsen",
    "Priya Rasmussen",
    "Emil Sandoval",
    "Noor Lindqvist",
  ],

  /** Real cities, none of them the principal's, with coordinates so a map renders.
   *  `region` is what the trips UI shows next to the name. */
  cities: [
    { name: "Lisbon", region: "Portugal", latitude: 38.7223, longitude: -9.1393 },
    { name: "Copenhagen", region: "Denmark", latitude: 55.6761, longitude: 12.5683 },
    { name: "Kraków", region: "Poland", latitude: 50.0647, longitude: 19.945 },
    { name: "Turin", region: "Italy", latitude: 45.0703, longitude: 7.6869 },
    { name: "Ghent", region: "Belgium", latitude: 51.0543, longitude: 3.7174 },
    { name: "Tallinn", region: "Estonia", latitude: 59.437, longitude: 24.7536 },
  ],

  /** The demo's home city. One of the above rather than a special case, so nothing in the
   *  corpus is anchored anywhere the real installation is. */
  home: { name: "Ghent", region: "Belgium", latitude: 51.0543, longitude: 3.7174 },

  /** Invented merchants, grouped by the ledger account a transaction from them posts to.
   *  The grouping is what makes the demo's category chart show structure rather than a
   *  flat bar — spending has a shape, and a fixture that does not have one demonstrates
   *  nothing about a tool built to find it. */
  merchants: [
    { name: "Nordlicht Kaffee", account: "expenses:food:cafe", min: 280, max: 950 },
    { name: "Markthal Grocers", account: "expenses:food:groceries", min: 1_200, max: 8_400 },
    { name: "Blau & Sohn Baumarkt", account: "expenses:home:supplies", min: 900, max: 14_000 },
    { name: "Velo Kollektiv", account: "expenses:transport:bike", min: 1_500, max: 22_000 },
    { name: "Stadtwerke Ghent", account: "expenses:home:utilities", min: 4_200, max: 11_800 },
    { name: "Regiobahn Tickets", account: "expenses:transport:rail", min: 480, max: 6_900 },
    { name: "Apotheke Zum Anker", account: "expenses:health:pharmacy", min: 620, max: 4_300 },
    { name: "Buchhandlung Silbe", account: "expenses:leisure:books", min: 890, max: 5_600 },
    { name: "Kantine Werk 4", account: "expenses:food:lunch", min: 640, max: 1_800 },
    { name: "Hafenbad Schwimmhalle", account: "expenses:health:sport", min: 450, max: 3_200 },
  ],

  /** Invented services, with a monthly price in cents. Drives the subscriptions page. */
  subscriptions: [
    { name: "Halden Notes", category: "tools", cents: 800 },
    { name: "Prisma Code Host", category: "tools", cents: 2_100 },
    { name: "Ferrit Object Storage", category: "infrastructure", cents: 1_150 },
    { name: "Kanto Music", category: "media", cents: 1_099 },
    { name: "Weidkamp Fitness", category: "health", cents: 3_490 },
    { name: "Loam Language Tutor", category: "learning", cents: 1_500 },
  ],

  /** Invented instruments. Deliberately not real tickers: a demo portfolio quoting real
   *  securities at invented prices is the one kind of fixture that could mislead somebody
   *  who skims it. */
  instruments: [
    { symbol: "NDLX", label: "Nordlys Broad Index", unitCents: 8_420 },
    { symbol: "TRVA", label: "Terravia Sustainable", unitCents: 3_190 },
    { symbol: "KPST", label: "Keystone Short Bond", unitCents: 10_240 },
  ],

  /** Task titles. Written to read like somebody's actual list — a demo whose tasks are
   *  "Task 1, Task 2" shows the layout and hides the product. */
  tasks: [
    "Return the bike light before the warranty lapses",
    "Book the dentist follow-up",
    "Renew the residence registration",
    "Compare electricity tariffs before the fixed term ends",
    "Send Mara the trip photos",
    "Cancel the unused storage plan",
    "Reply to the housing association about the bike shed",
    "Order replacement filters for the kettle",
    "Read the two saved papers on retrieval evaluation",
    "Fix the wobbling shelf in the hallway",
  ],

  /** Calendar entries as {title, kind, hours}. `kind` matches the calendar capability's
   *  own vocabulary, so the demo exercises its real colour and feasibility rules. */
  events: [
    { title: "Standup", kind: "work_onsite", hours: 0.5 },
    { title: "Design review — ingest pipeline", kind: "work_onsite", hours: 1 },
    { title: "Bouldering", kind: "sport", hours: 2 },
    { title: "Dinner with Tomas", kind: "nightlife", hours: 3 },
    { title: "Language class", kind: "learning", hours: 1.5 },
    { title: "Dentist", kind: "appointment", hours: 1 },
    { title: "Long run", kind: "sport", hours: 1.5 },
    { title: "Team retro", kind: "work_onsite", hours: 1 },
    { title: "Parents visiting", kind: "family", hours: 6 },
    { title: "Concert — Kanto Live", kind: "nightlife", hours: 4 },
  ],

  /** Weekly rhythms, in the calendar capability's byweekday form. */
  rhythms: [
    { title: "Bouldering", kind: "sport", byweekday: ["TU", "TH"], start: "18:30", end: "20:30" },
    { title: "Language class", kind: "learning", byweekday: ["WE"], start: "19:00", end: "20:30" },
    { title: "Standup", kind: "work_onsite", byweekday: ["MO", "TU", "WE", "TH", "FR"], start: "09:30", end: "09:45" },
  ],

  /** Trip titles, paired with how far ahead of the anchor they start. A demo with every
   *  trip in the future has no history and no completed state to show. */
  trips: [
    { title: "Long weekend in Lisbon", city: "Lisbon", offsetDays: 24, nights: 4 },
    { title: "Conference — Copenhagen", city: "Copenhagen", offsetDays: 61, nights: 3 },
    { title: "Kraków with Mara", city: "Kraków", offsetDays: -38, nights: 5 },
  ],

  /** Manual balances: accounts a bank CSV never covers. `kind` drives the net-worth sign. */
  balances: [
    { label: "Everyday account", kind: "asset" as const, cents: 412_300 },
    { label: "Savings", kind: "asset" as const, cents: 1_845_000 },
    { label: "Bike loan", kind: "liability" as const, cents: 78_400 },
  ],

  /** Employers and income lines. Invented, same rule as merchants. */
  income: [
    { description: "Salary — Halden Systems", account: "income:salary", cents: 289_000 },
    { description: "Invoice — Velten Studio", account: "income:freelance", cents: 62_500 },
  ],
} as const;

/** Every string anywhere in VOCABULARY, flattened. The test sweeps this; the payload gate
 *  sweeps the built site. Two passes over the same rule, at the two moments it can fail. */
export function vocabularyStrings(): string[] {
  const out: string[] = [];
  const walk = (value: unknown): void => {
    if (typeof value === "string") out.push(value);
    else if (Array.isArray(value)) value.forEach(walk);
    else if (value && typeof value === "object") Object.values(value).forEach(walk);
  };
  walk(VOCABULARY);
  return out;
}

/** An address that is syntactically an email and semantically nobody. RFC 2606 reserves
 *  example.org for exactly this, so a scanner can allow it by name without allowing a
 *  pattern that a real address could slip through. */
export function email(name: string): string {
  return `${name.toLowerCase().replace(/[^a-z]+/g, ".")}@example.org`;
}
