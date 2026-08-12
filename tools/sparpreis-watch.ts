// One pass of the Sparpreis price watch, then exit.
//
// The research verdict this implements (travel PRD R4, 2026-08-12): Sparpreis prices
// for a specific train DO fall, but rarely and unpredictably — later-released cheap
// contingents and DB promo windows are the two real events. Watching a booked train is
// dead weight; watching a not-yet-booked trip is one cheap cron. So this job re-prices
// exactly the rail searches that already live in upcoming plans as `option_set` items
// (the solver and the agent surface write those), appends today's observation as a new
// option_set beside them, and writes a `note` item when the cheapest fare DROPPED below
// the last observation. Durable plan state is the alert surface — the dashboard shows
// the note; no new notification machinery.
//
// It talks to trips and transit over HTTP and never to their databases — the documented
// composition edge (README.md#schemas-and-dependency-direction). Fare context (bc,
// d_ticket, first_class) is replayed from the watched query, so a drop is a drop in the
// price the traveller would actually pay.

import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";

import { axonRoot } from "./lib/overlay.ts";

function fail(message: string): never {
  console.error(`sparpreis-watch: ${message}`);
  process.exit(1);
}

const AXON_ROOT = axonRoot();

/** A capability's port, from the one file that declares it. */
function portOf(capability: string): string {
  const manifest = join(AXON_ROOT, "capabilities", capability, "service.toml");
  if (!existsSync(manifest)) fail(`no ${manifest}`);
  const line = readFileSync(manifest, "utf8")
    .split("\n")
    .find((l) => /^port\s*=/.test(l));
  const port = line?.match(/"([^"]*)"/)?.[1] ?? "";
  if (!port) fail(`no port in ${manifest}`);
  return port;
}

export interface RailWatch {
  planId: string;
  from: string;
  to: string;
  time: string;
  bc?: number;
  dTicket?: boolean;
  firstClass?: boolean;
}

/** A stable identity for one watched search, so observations upsert per day. */
export function watchKey(watch: RailWatch): string {
  const fare = [
    watch.bc ? `bc${watch.bc}` : "",
    watch.dTicket ? "dt" : "",
    watch.firstClass ? "k1" : "",
  ]
    .filter(Boolean)
    .join("-");
  return `${watch.from}:${watch.to}:${watch.time}${fare ? `:${fare}` : ""}`;
}

/** The rail searches a plan already records: option_set items whose query names two
 *  numeric station ids and a departure time. Accommodation option_sets (coordinate
 *  queries) and anything else fall through the numeric test. */
export function railWatchesOf(planId: string, items: unknown[]): RailWatch[] {
  const watches: RailWatch[] = [];
  for (const raw of items) {
    const item = raw as {
      item_type?: string;
      external_id?: string;
      payload?: {
        query?: {
          from?: unknown;
          to?: unknown;
          time?: unknown;
          bc?: unknown;
          d_ticket?: unknown;
          first_class?: unknown;
        };
      };
    };
    if (item.item_type !== "option_set") continue;
    // This job's own observations are option_sets too; re-watching them would
    // multiply the watch list every run.
    if (item.external_id?.startsWith("sparpreis-watch:")) continue;
    const q = item.payload?.query;
    if (typeof q?.from !== "string" || !/^\d+$/.test(q.from)) continue;
    if (typeof q?.to !== "string" || !/^\d+$/.test(q.to)) continue;
    if (typeof q?.time !== "string" || !q.time.includes("T")) continue;
    watches.push({
      planId,
      from: q.from,
      to: q.to,
      time: q.time,
      bc: typeof q.bc === "number" ? q.bc : undefined,
      dTicket: q.d_ticket === true,
      firstClass: q.first_class === true,
    });
  }
  return watches;
}

/** The last observed cheapest fare for a watch, from this job's own prior
 *  observations in the same plan. `null` on the first run for a watch. */
export function previousCheapest(items: unknown[], key: string): number | null {
  let latest: { day: string; price: number } | null = null;
  for (const raw of items) {
    const item = raw as {
      item_type?: string;
      external_id?: string;
      payload?: { options?: Array<{ total_price?: unknown }> };
    };
    if (item.item_type !== "option_set") continue;
    const prefix = `sparpreis-watch:${key}:`;
    if (!item.external_id?.startsWith(prefix)) continue;
    const day = item.external_id.slice(prefix.length);
    const prices = (item.payload?.options ?? [])
      .map((o) => o.total_price)
      .filter((p): p is number => typeof p === "number");
    if (!prices.length) continue;
    const price = Math.min(...prices);
    if (!latest || day > latest.day) latest = { day, price };
  }
  return latest?.price ?? null;
}

/** A drop is a real drop, not float noise. */
export function dropped(previous: number | null, current: number): boolean {
  return previous !== null && current < previous - 0.01;
}

async function main(): Promise<void> {
  const trips = `http://127.0.0.1:${portOf("trips")}`;
  const transit = `http://127.0.0.1:${portOf("transit")}`;
  const today = new Date().toISOString().slice(0, 10);

  const plans = (await (await fetch(`${trips}/api/plans`)).json()) as Array<{
    id: string;
    date_start: string;
    title: string;
  }>;
  const upcoming = plans.filter((p) => p.date_start >= today);
  console.log(`sparpreis-watch: ${upcoming.length}/${plans.length} plans upcoming`);

  let watched = 0;
  let dropCount = 0;
  const CAP = 10;
  for (const plan of upcoming) {
    const details = (await (await fetch(`${trips}/api/plans/${plan.id}`)).json()) as {
      items?: unknown[];
    };
    const items = details.items ?? [];
    for (const watch of railWatchesOf(plan.id, items)) {
      if (watched >= CAP) {
        console.log(`sparpreis-watch: cap of ${CAP} reached, remaining watches skipped this run`);
        return;
      }
      watched += 1;
      // The endpoint under this is bahn.de via transit; transit paces itself,
      // and this pause keeps a multi-watch run from bursting anyway.
      await new Promise((resolve) => setTimeout(resolve, 1000));
      const params = new URLSearchParams({ from: watch.from, to: watch.to, time: watch.time });
      if (watch.bc) params.set("bc", String(watch.bc));
      if (watch.dTicket) params.set("d_ticket", "true");
      if (watch.firstClass) params.set("first_class", "true");
      const search = await fetch(`${transit}/api/search?${params}`);
      if (!search.ok) {
        console.error(`sparpreis-watch: search ${watch.from}->${watch.to} HTTP ${search.status}`);
        continue;
      }
      const journeys = (await search.json()) as Array<{ total_price?: number | null }>;
      const prices = journeys
        .map((j) => j.total_price)
        .filter((p): p is number => typeof p === "number");
      if (!prices.length) {
        console.log(`sparpreis-watch: ${watch.from}->${watch.to} returned no priced journey`);
        continue;
      }
      const cheapest = Math.min(...prices);
      const key = watchKey(watch);
      const previous = previousCheapest(items, key);

      const wrote = await fetch(`${trips}/api/plans/${plan.id}/items`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({
          item_type: "option_set",
          external_id: `sparpreis-watch:${key}:${today}`,
          title: `Sparpreis watch ${watch.from} → ${watch.to}`,
          payload: {
            query: { from: watch.from, to: watch.to, time: watch.time, bc: watch.bc ?? null },
            observed_at: new Date().toISOString(),
            options: journeys.slice(0, 5).map((j) => ({ total_price: j.total_price ?? null })),
          },
        }),
      });
      if (!wrote.ok) {
        console.error(`sparpreis-watch: observation write HTTP ${wrote.status}`);
        continue;
      }
      if (dropped(previous, cheapest)) {
        dropCount += 1;
        await fetch(`${trips}/api/plans/${plan.id}/items`, {
          method: "POST",
          headers: { "content-type": "application/json" },
          body: JSON.stringify({
            item_type: "note",
            external_id: `sparpreis-drop:${key}:${today}`,
            title: `Sparpreis drop ${watch.from} → ${watch.to}: €${previous} → €${cheapest}`,
            payload: { previous, current: cheapest, watched_time: watch.time },
          }),
        });
        console.log(
          `sparpreis-watch: DROP ${watch.from}->${watch.to} €${previous} -> €${cheapest} (${plan.title})`,
        );
      } else {
        console.log(
          `sparpreis-watch: ${watch.from}->${watch.to} cheapest €${cheapest}` +
            (previous !== null ? ` (was €${previous})` : " (first observation)"),
        );
      }
    }
  }
  console.log(`sparpreis-watch: ${watched} watched, ${dropCount} drops`);
}

if (import.meta.main) {
  await main();
}
