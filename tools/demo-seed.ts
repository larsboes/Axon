#!/usr/bin/env bun
// tools/demo-seed.ts — fill running capabilities with fictional data, through their own APIs (#168).
//
// No capability crate contains a line of demo code. Everything below is a POST to a route
// the dashboard also calls, which buys three things: the seeders cannot reach past a public
// contract into a store, a seeding run is an end-to-end exercise of the write paths, and a
// capability's demo data goes stale the same way its API does — loudly, with a 404.
//
// THE GUARD, and why it is not optional. These are real writes to whatever database the
// running capabilities are pointed at. Run this against a machine's own Axon and it would
// post invented transactions into a real ledger. So it refuses to start unless the resolved
// overlay is this repository's demo overlay, and refuses each capability that already holds
// data. Both checks are cheap; the failure they prevent is not recoverable by re-running.
//
//   tools/demo-seed              seed every capability with a `writes` in demo.toml
//   tools/demo-seed --only tasks seed one
//   tools/demo-seed --force      skip the not-empty refusal (never the overlay check)

import { execFileSync } from "node:child_process";
import { mkdirSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";

import {
  addDays,
  at,
  daysBetween,
  eventWindow,
  monthStart,
  Rng,
  VOCABULARY,
} from "./lib/demo-data.ts";
import {
  AXON_ROOT,
  loadManifest,
  resolvePath,
  routes,
  type DemoManifest,
} from "./lib/demo-endpoints.ts";

const DEMO_OVERLAY = join(AXON_ROOT, "demo/overlay");

interface Ctx {
  rng: Rng;
  anchor: string;
  manifest: DemoManifest;
  /** Resolve a browser path to a live URL, e.g. `/tasks/api/tasks`. */
  url: (browserPath: string) => string;
}

// ─── HTTP ─────────────────────────────────────────────────────────────────────

async function send<T>(method: string, url: string, body?: unknown): Promise<T> {
  const res = await fetch(url, {
    method,
    headers: body === undefined ? undefined : { "Content-Type": "application/json" },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const text = await res.text();
  if (!res.ok) {
    throw new Error(`${method} ${url} → ${res.status}: ${text.slice(0, 400)}`);
  }
  return (text ? JSON.parse(text) : undefined) as T;
}

const get = <T>(url: string) => send<T>("GET", url);
const post = <T>(url: string, body?: unknown) => send<T>("POST", url, body);
const patch = <T>(url: string, body: unknown) => send<T>("PATCH", url, body);

/** Poll a capability until it answers, so a seeder never races a server that is still
 *  opening its database. Bounded: a capability that is not coming up is a failure to
 *  report, not something to wait out. */
async function waitFor(url: string, seconds = 60): Promise<void> {
  const deadline = Date.now() + seconds * 1000;
  let last = "";
  while (Date.now() < deadline) {
    try {
      const res = await fetch(url);
      if (res.ok) return;
      last = `${res.status}`;
    } catch (err) {
      last = err instanceof Error ? err.message : String(err);
    }
    await Bun.sleep(500);
  }
  throw new Error(`demo-seed: ${url} never became ready (${seconds}s; last: ${last})`);
}

// ─── Seeders ──────────────────────────────────────────────────────────────────
//
// One function per `writes` value in demo.toml. Each returns a short line for the log —
// what it created, so a CI run says what the published demo actually contains.

const SEEDERS: Record<string, (ctx: Ctx) => Promise<string>> = {
  async tasks(ctx) {
    const url = ctx.url("/tasks/api/tasks");
    const titles = ctx.rng.sample(VOCABULARY.tasks, 9);
    let done = 0;
    for (const [index, title] of titles.entries()) {
      // A third of them dated, spread either side of the anchor: the demo has to show an
      // overdue row, because "what is late" is the question the counts endpoint exists for.
      const due = ctx.rng.bool(0.55) ? addDays(ctx.anchor, ctx.rng.int(-9, 21)) : null;
      const created = await post<{ task: { id: string } }>(url, {
        title,
        due,
        note: ctx.rng.bool(0.3) ? "Picked up from last week's review." : null,
      });
      // Close a couple, so the demo's done filter is not an empty list.
      if (index % 4 === 3) {
        await patch(`${url}/${created.task.id}`, { status: "done" });
        done++;
      }
    }
    return `${titles.length} tasks (${done} completed)`;
  },

  async calendar(ctx) {
    const entries = ctx.url("/calendar/api/entries");
    const rhythms = ctx.url("/calendar/api/rhythms");
    const contexts = ctx.url("/calendar/api/contexts");

    // Rhythms first: each materializes its own instances, and an entry posted afterwards
    // can then legitimately collide with one — which is what makes the feasibility view
    // show a conflict rather than an empty grid.
    for (const r of VOCABULARY.rhythms) {
      await post(rhythms, {
        kind: r.kind,
        title: r.title,
        location: VOCABULARY.home.name,
        byweekday: [...r.byweekday],
        start_time: r.start,
        end_time: r.end,
        valid_from: addDays(ctx.anchor, -28),
        valid_until: addDays(ctx.anchor, 45),
      });
    }

    let count = 0;
    for (let day = -26; day <= 44; day++) {
      if (!ctx.rng.bool(0.42)) continue;
      const event = ctx.rng.pick(VOCABULARY.events);
      const date = addDays(ctx.anchor, day);
      const window = eventWindow(ctx.rng, date, event.hours);
      await post(entries, {
        kind: event.kind,
        // Past entries are settled, near ones are planned, far ones are still ideas. A
        // calendar where everything is equally certain cannot demonstrate a tool whose
        // whole model is that they are not.
        commitment: day < 0 ? "committed" : day < 14 ? "planned" : "possible",
        title: event.title,
        starts_at: window.starts_at,
        ends_at: window.ends_at,
        all_day: false,
        location: ctx.rng.bool(0.6) ? VOCABULARY.home.name : null,
        notes: null,
        source: "manual",
      });
      count++;
    }

    for (const trip of VOCABULARY.trips.filter((t) => t.offsetDays > 0)) {
      await post(contexts, {
        kind: "travel",
        title: `Away — ${trip.city}`,
        details: "Reachable, but not for anything that needs the desk.",
        valid_from: addDays(ctx.anchor, trip.offsetDays),
        valid_until: addDays(ctx.anchor, trip.offsetDays + trip.nights),
      });
    }
    return `${VOCABULARY.rhythms.length} rhythms, ${count} entries, ${VOCABULARY.trips.filter((t) => t.offsetDays > 0).length} contexts`;
  },

  async trips(ctx) {
    const url = ctx.url("/trips/api/plans");
    const home = VOCABULARY.home;
    const place = (c: (typeof VOCABULARY.cities)[number]) => ({
      id: `demo-${c.name.toLowerCase().replace(/[^a-z]/g, "")}`,
      name: c.name,
      kind: "city" as const,
      address: null,
      latitude: c.latitude,
      longitude: c.longitude,
    });

    for (const trip of VOCABULARY.trips) {
      const city = VOCABULARY.cities.find((c) => c.name === trip.city)!;
      const start = addDays(ctx.anchor, trip.offsetDays);
      const plan = await post<{ id: string }>(url, {
        title: trip.title,
        origin: place(home),
        destinations: [place(city)],
        date_start: start,
        date_end: addDays(start, trip.nights),
        interests: "Walking the city, one museum, somewhere to work for a morning.",
        travelers: ctx.rng.sample(VOCABULARY.people, ctx.rng.int(1, 2)),
        transport_modes: ["train", "walk"],
      });
      // Two items per plan, so a detail page has an itinerary rather than a header.
      await post(`${url}/${plan.id}/items`, {
        item_type: "stay",
        day: start,
        external_id: `demo-stay-${plan.id}`,
        title: `Apartment near the centre, ${city.name}`,
        payload: { nights: trip.nights, currency: "EUR" },
      });
      await post(`${url}/${plan.id}/items`, {
        item_type: "note",
        day: addDays(start, 1),
        external_id: `demo-note-${plan.id}`,
        title: "Museum is closed on Mondays",
        payload: {},
      });
    }
    return `${VOCABULARY.trips.length} plans with itinerary items`;
  },

  async finance(ctx) {
    const base = ctx.url("/finance/api");

    // 1. A bank export, the way somebody actually starts: preview, then import, then
    //    review. Semicolons and comma decimals on purpose — that is the European CSV this
    //    importer exists for, and a demo built on the easy dialect would not show it working.
    const { csv, rows } = bankCsv(ctx);
    const mapping = bankMapping();
    const preview = await post<{ preview_id: string; candidate_count: number }>(
      `${base}/import/csv/preview`,
      { content: csv, mapping },
    );
    await post(`${base}/import/csv`, {
      content: csv,
      mapping,
      expected_preview_id: preview.preview_id,
    });

    // 2. Confirm most of them, at the account the importer proposed, and deliberately
    //    leave a handful pending — the Import Review screen is part of the product, and a
    //    demo with an empty review queue hides it.
    const candidates = await get<Array<{ id: string; proposed_account: string; state: string }>>(
      `${base}/import/candidates`,
    );
    const pending = candidates.filter((c) => c.state === "pending");
    const confirm = pending.slice(0, Math.max(0, pending.length - 6));
    if (confirm.length > 0) {
      await post(`${base}/import/candidates/confirm-batch`, {
        items: confirm.map((c) => ({ id: c.id, account: c.proposed_account })),
      });
    }

    // 3. Holdings, from a broker export in the same shape.
    const investments = investmentCsv(ctx);
    const invMapping = investmentMapping();
    const invPreview = await post<{ snapshot_id: string }>(
      `${base}/import/investments/preview`,
      { content: investments, mapping: invMapping },
    );
    await post(`${base}/import/investments/confirm`, {
      content: investments,
      mapping: invMapping,
      source_key: "demo-broker",
      expected_snapshot_id: invPreview.snapshot_id,
      coverage: "complete",
    });

    // 4. What a bank export cannot know: cash outside the account, and a loan.
    await post(`${base}/balance-snapshot`, {
      as_of: ctx.anchor,
      currency: "EUR",
      coverage: "complete",
      balances: VOCABULARY.balances.map((b, i) => ({
        id: `demo-balance-${i}`,
        label: b.label,
        kind: b.kind,
        amount_cents: b.cents,
      })),
    });

    // 5. Subscriptions, seeded from vault notes this writes first. Generated rather than
    //    committed: they are demo data like everything else, and a tracked markdown file
    //    that looks like somebody's real note is exactly the confusion to avoid.
    writeSubscriptionVault();
    const imported = await post<{ created: number }>(`${base}/import/obsidian`);

    return (
      `${rows} bank rows → ${preview.candidate_count} candidates ` +
      `(${confirm.length} confirmed, ${pending.length - confirm.length} left for review), ` +
      `${VOCABULARY.instruments.length} holdings, ${VOCABULARY.balances.length} balances, ` +
      `${imported.created} subscriptions`
    );
  },
};

// ─── Finance inputs ───────────────────────────────────────────────────────────

/** Fourteen months of transactions. Long enough that the trend chart has a trend and the
 *  month-over-month view has something to compare; short enough to import in seconds. */
function bankCsv(ctx: Ctx): { csv: string; rows: number } {
  const lines = ["date;description;amount;currency;reference;category"];
  const cents = (n: number) => (n / 100).toFixed(2).replace(".", ",");

  for (let monthsBack = 13; monthsBack >= 0; monthsBack--) {
    const first = monthStart(ctx.anchor, monthsBack);
    const span = monthsBack === 0 ? daysBetween(first, ctx.anchor) : 27;

    for (const income of VOCABULARY.income) {
      // Freelance work is not every month. Salary is.
      if (income.account.endsWith("freelance") && !ctx.rng.bool(0.35)) continue;
      const day = addDays(first, income.account.endsWith("salary") ? 0 : ctx.rng.int(4, 20));
      if (daysBetween(first, day) > span) continue;
      lines.push(
        [
          german(day),
          income.description,
          cents(income.cents + ctx.rng.int(-4_000, 4_000)),
          "EUR",
          "",
          "income",
        ].join(";"),
      );
    }

    for (let i = 0; i < ctx.rng.int(14, 26); i++) {
      const offset = ctx.rng.int(0, span);
      const merchant = ctx.rng.pick(VOCABULARY.merchants);
      lines.push(
        [
          german(addDays(first, offset)),
          merchant.name,
          `-${cents(ctx.rng.amountCents(merchant.min, merchant.max))}`,
          "EUR",
          "",
          merchant.account.split(":")[1] ?? "",
        ].join(";"),
      );
    }
  }
  return { csv: `${lines.join("\n")}\n`, rows: lines.length - 1 };
}

const german = (iso: string): string => {
  const [y, m, d] = iso.split("-");
  return `${d}.${m}.${y}`;
};

function bankMapping() {
  return {
    delimiter: ";",
    decimal_separator: ",",
    date_column: "date",
    amount_column: "amount",
    description_column: "description",
    categorization_columns: ["category"],
    reference_column: "reference",
    currency_column: "currency",
    default_currency: "EUR",
    source_account: "assets:bank:everyday",
    default_outflow_account: "expenses:uncategorized",
    default_inflow_account: "income:uncategorized",
    // One rule per invented merchant. Written from the same vocabulary the CSV is written
    // from, so the demo's categorization rate is high but not perfect — `expenses:uncategorized`
    // still catches the rows no rule names, which is the honest picture of a real import.
    categorization_rules: VOCABULARY.merchants.map((m) => ({
      description_contains_any: [m.name],
      description_starts_with_any: [],
      direction: "outflow" as const,
      account: m.account,
      confidence_basis_points: 9_200,
    })).concat(
      VOCABULARY.income.map((i) => ({
        description_contains_any: [i.description],
        description_starts_with_any: [],
        direction: "inflow" as const,
        account: i.account,
        confidence_basis_points: 9_800,
      })),
    ),
    row_filter: null,
    amount_sign: "as_provided" as const,
    amount_rounding: "half_away_from_zero" as const,
    date_formats: ["day_month_year_dots" as const],
    row_policy: "required_fields" as const,
  };
}

function investmentCsv(ctx: Ctx): string {
  const lines = ["date;instrument;quantity;price;currency;activity"];
  for (const instrument of VOCABULARY.instruments) {
    // A position built over several buys, which is what makes the holdings view show an
    // aggregate rather than echoing one row back.
    for (let i = 0; i < ctx.rng.int(3, 6); i++) {
      lines.push(
        [
          german(addDays(ctx.anchor, -ctx.rng.int(30, 900))),
          instrument.symbol,
          String(ctx.rng.int(1, 12)),
          (instrument.unitCents / 100).toFixed(2).replace(".", ","),
          "EUR",
          "BUY",
        ].join(";"),
      );
    }
    lines.push(
      [german(addDays(ctx.anchor, -ctx.rng.int(10, 120))), instrument.symbol, "0", "0,00", "EUR", "DIVIDEND"].join(";"),
    );
  }
  return `${lines.join("\n")}\n`;
}

function investmentMapping() {
  return {
    delimiter: ";",
    decimal_separator: ",",
    date_column: "date",
    instrument_column: "instrument",
    quantity_column: "quantity",
    activity_type_column: "activity",
    position_activity_values: ["BUY", "SELL"],
    non_position_activity_values: ["DIVIDEND", "FEE"],
    reference_column: null,
    price_column: "price",
    currency_column: "currency",
    default_currency: "EUR",
    instrument_aliases: Object.fromEntries(
      VOCABULARY.instruments.map((i) => [i.symbol, i.canonical]),
    ),
  };
}

/** The vault the finance capability imports subscriptions from. Written fresh each run into
 *  demo/vault, which is untracked — see the comment at the call site. */
function writeSubscriptionVault(): void {
  const dir = join(AXON_ROOT, "demo/vault/subscriptions");
  rmSync(dir, { recursive: true, force: true });
  mkdirSync(dir, { recursive: true });
  for (const sub of VOCABULARY.subscriptions) {
    const note = [
      "---",
      `cost: ${(sub.cents / 100).toFixed(2)}`,
      "currency: EUR",
      "billing_cycle: monthly",
      `category: ${sub.category}`,
      "status: active",
      `value_rating: ${(sub.cents % 5) + 1}`,
      "---",
      "",
      `# ${sub.name}`,
      "",
      "A fictional service, in a fictional vault, for a demonstration.",
      "",
    ].join("\n");
    writeFileSync(join(dir, `${sub.name.replace(/[^A-Za-z0-9]+/g, "-")}.md`), note);
  }
}

// ─── Guards ───────────────────────────────────────────────────────────────────

/** Which overlay the running capabilities were configured from. Asked of paths.sh rather
 *  than reimplemented, because the answer has to be the one the service runner got. */
function activeOverlay(): string {
  const out = execFileSync(
    "bash",
    ["-c", `source "${AXON_ROOT}/tools/lib/paths.sh" && printf '%s' "$AXON_OVERLAY_ROOT"`],
    { encoding: "utf8" },
  );
  return out.trim();
}

function requireDemoOverlay(): void {
  const active = activeOverlay();
  if (active === DEMO_OVERLAY) return;
  console.error(
    [
      "demo-seed: refusing to write — the active overlay is not the demo overlay.",
      "",
      `  active: ${active}`,
      `  demo:   ${DEMO_OVERLAY}`,
      "",
      "These are real POSTs to whatever store the running capabilities use. Against a",
      "personal overlay that means inventing rows in a real ledger and a real calendar.",
      "",
      "Start the demo stack instead, which points every capability at the demo overlay:",
      "",
      "  tools/demo-up",
      "",
    ].join("\n"),
  );
  process.exit(1);
}

/** One list endpoint per seeded capability, used only to prove it is empty before writing.
 *  A capability whose first declared path is not a list is fine — a non-array answer is
 *  not evidence of data, so it does not block. */
async function refuseIfPopulated(cap: string, url: string, force: boolean): Promise<void> {
  let body: unknown;
  try {
    body = await get(url);
  } catch {
    return; // Unreachable is the runner's problem, reported by waitFor, not this check's.
  }
  const rows = Array.isArray(body)
    ? body.length
    : Array.isArray((body as any)?.tasks)
      ? (body as any).tasks.length
      : 0;
  if (rows === 0) return;
  if (force) {
    console.warn(`demo-seed: ${cap} already holds ${rows} rows — seeding anyway (--force)`);
    return;
  }
  console.error(
    `demo-seed: ${cap} already holds ${rows} rows. The demo expects an empty store, and\n` +
      `seeding on top of one produces a corpus nobody can reproduce. Reset it, or pass --force.`,
  );
  process.exit(1);
}

// ─── Main ─────────────────────────────────────────────────────────────────────

async function main(): Promise<void> {
  const args = process.argv.slice(2);
  if (args.includes("-h") || args.includes("--help")) {
    console.log("tools/demo-seed [--only <capability>] [--force]");
    return;
  }
  const onlyIdx = args.indexOf("--only");
  const only = onlyIdx >= 0 ? args[onlyIdx + 1] : null;
  const force = args.includes("--force");

  requireDemoOverlay();

  const manifest = loadManifest();
  const table = routes();
  const url = (browserPath: string) => resolvePath(browserPath, table).url;
  const seeded = manifest.capabilities.filter((c) => c.writes && (!only || c.name === only));
  if (seeded.length === 0) {
    console.error(`demo-seed: nothing to seed${only ? ` for '${only}'` : ""}`);
    process.exit(1);
  }

  for (const cap of seeded) {
    const seeder = SEEDERS[cap.writes!];
    if (!seeder) {
      throw new Error(`demo.toml: [capability.${cap.name}] writes = "${cap.writes}", which no seeder implements`);
    }
    await waitFor(url(cap.paths[0]));
    await refuseIfPopulated(cap.name, url(cap.paths[0]), force);
  }

  for (const cap of seeded) {
    // A fresh Rng per capability, seeded from the manifest seed plus the capability name.
    // Seeding only tasks then has to produce the same tasks as seeding everything — with one
    // shared generator it would not, and a `--only` run would silently differ from CI's.
    const ctx: Ctx = {
      rng: new Rng(`${manifest.seed}:${cap.name}`),
      anchor: manifest.anchor,
      manifest,
      url,
    };
    const summary = await SEEDERS[cap.writes!](ctx);
    console.log(`seeded ${cap.name}: ${summary}`);
  }
}

if (import.meta.main) {
  main().catch((err) => {
    console.error(`demo-seed: ${err instanceof Error ? err.message : String(err)}`);
    process.exit(1);
  });
}
