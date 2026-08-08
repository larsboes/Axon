<script lang="ts">
  import { onMount } from "svelte";
  import Icon from "$lib/Icon.svelte";
  import PageHeader from "$lib/PageHeader.svelte";
  import {
    finance,
    type Burn,
    type Subscription,
    type SubscriptionState,
    type WritebackResult,
  } from "$lib/api";

  // The date picker is the point of this page rather than a convenience on it. A
  // subscription's price is an append-only series, so "what am I paying" and "what
  // will I be paying in October" are different questions with different answers,
  // and every other subscription tool can only answer the first.
  let at = $state(new Date().toISOString().slice(0, 10));

  let subs = $state<Subscription[]>([]);
  let burn = $state<Burn | null>(null);
  let error = $state<string | null>(null);
  let busy = $state(false);
  let notice = $state<string | null>(null);
  let loaded = $state(false);

  async function load(date: string) {
    try {
      const [list, totals] = await Promise.all([finance.subscriptions(), finance.burn(date)]);
      subs = list;
      burn = totals;
      error = null;
    } catch (cause) {
      error = cause instanceof Error ? cause.message : String(cause);
    } finally {
      loaded = true;
    }
  }

  // A native date input can emit a partial or malformed value mid-edit, and it does
  // not have to be plausible: typing eight digits into the year segment produced
  // `202610-01-08`, which every comparison here happily accepted. Dates are compared
  // lexicographically, so a malformed one does not throw — it sorts somewhere
  // arbitrary and returns a confident wrong number, which is the failure mode this
  // whole capability exists to avoid.
  const WELL_FORMED = /^\d{4}-\d{2}-\d{2}$/;
  const validAt = $derived(WELL_FORMED.test(at) ? at : null);

  onMount(() => void load(at));

  $effect(() => {
    if (loaded && validAt) void load(validAt);
  });

  const ORDER: Record<SubscriptionState, number> = {
    active: 0,
    trial: 1,
    paused: 2,
    considering: 3,
    cancelled: 4,
  };

  interface Row {
    sub: Subscription;
    state: SubscriptionState;
    monthlyCents: number;
    currency: string;
    scheduled: { valid_from: string; amount_cents: number; currency: string } | null;
  }

  /** The same arithmetic the server does, so a row and the total agree. The server
   *  owns the authoritative figure; this exists because rendering per-row costs
   *  would otherwise need one request each. */
  function at_or_before<T extends { d: string }>(items: T[], date: string): T | null {
    let best: T | null = null;
    for (const item of items) if (item.d <= date && (!best || item.d > best.d)) best = item;
    return best;
  }

  const MONTHLY: Record<string, (cents: number) => number> = {
    weekly: (c) => Math.round((c * 52) / 12),
    monthly: (c) => c,
    quarterly: (c) => Math.round(c / 3),
    yearly: (c) => Math.round(c / 12),
    one_off: () => 0,
  };

  const rows = $derived.by<Row[]>(() =>
    subs
      .map((sub) => {
        const price = at_or_before(
          sub.prices.map((p) => ({ d: p.valid_from, p })),
          at,
        )?.p;
        const state =
          at_or_before(
            sub.states.map((s) => ({ d: s.effective, s })),
            at,
          )?.s.state ?? "considering";
        const billing = state === "active" || state === "trial";
        const scheduled =
          sub.prices
            .filter((p) => p.valid_from > at)
            .sort((a, b) => a.valid_from.localeCompare(b.valid_from))[0] ?? null;
        return {
          sub,
          state,
          monthlyCents: billing && price ? (MONTHLY[price.cycle] ?? ((c: number) => c))(price.amount_cents) : 0,
          currency: price?.currency ?? "EUR",
          scheduled,
        };
      })
      // Billing first, then most expensive. A cut candidate is a big number next to
      // a low value rating, and this puts both in the first rows.
      .sort(
        (a, b) => ORDER[a.state] - ORDER[b.state] || b.monthlyCents - a.monthlyCents,
      ),
  );

  interface Upcoming {
    id: string;
    name: string;
    valid_from: string;
    amount_cents: number;
    currency: string;
  }

  // Narrowed here rather than asserted in the markup: the template is not parsed as
  // TypeScript, so `row.scheduled!` is a syntax error rather than a type hint.
  const upcoming = $derived.by<Upcoming[]>(() =>
    rows.flatMap((row) =>
      row.scheduled
        ? [
            {
              id: row.sub.id,
              name: row.sub.name,
              valid_from: row.scheduled.valid_from,
              amount_cents: row.scheduled.amount_cents,
              currency: row.scheduled.currency,
            },
          ]
        : [],
    ),
  );

  function money(cents: number, currency = "EUR"): string {
    const sign = cents < 0 ? "-" : "";
    const abs = Math.abs(cents);
    return `${sign}${Math.floor(abs / 100)}.${String(abs % 100).padStart(2, "0")} ${currency}`;
  }

  async function run(label: string, action: () => Promise<unknown>) {
    busy = true;
    notice = null;
    try {
      const result = await action();
      notice = describe(label, result);
      await load(at);
    } catch (cause) {
      error = cause instanceof Error ? cause.message : String(cause);
    } finally {
      busy = false;
    }
  }

  /** A writeback that hit a conflict is not a failure to report as an error, and
   *  not a success to report as done. It names the notes and waits for a human. */
  function describe(label: string, result: unknown): string {
    if (label === "writeback") {
      const w = result as WritebackResult;
      if (w.conflicts.length > 0) {
        return `${w.written} written, ${w.unchanged} unchanged. Left alone because you edited inside the block: ${w.conflicts.join(", ")}`;
      }
      return `${w.written} written, ${w.unchanged} already correct.`;
    }
    const i = result as { created: number; already_present: number };
    return `${i.created} imported, ${i.already_present} already known.`;
  }
</script>

<PageHeader
  badge="Finance"
  title="Subscriptions"
  desc="What recurring spending costs, on a date you choose. Price is a series, so the answer moves."
/>

{#if error}
  <p class="err"><Icon name="alert" size={14} /> {error}</p>
{:else if !loaded}
  <p class="muted">Loading…</p>
{:else}
  <section class="bar">
    <label class="date">
      <span>As of</span>
      <input type="date" bind:value={at} />
    </label>

    {#if !validAt}
      <p class="muted totals">Waiting for a complete date…</p>
    {:else if burn}
      <div class="totals">
        <div class="total">
          <span class="figure">{burn.monthly}</span>
          <span class="unit">EUR / month</span>
        </div>
        <div class="total secondary">
          <span class="figure">{burn.annual}</span>
          <span class="unit">EUR / year</span>
        </div>
        <p class="muted count">
          {burn.billing_count} billing of {burn.total_count} tracked
        </p>
      </div>
    {/if}

    <div class="actions">
      <button disabled={busy} onclick={() => run("import", finance.importVault)}>
        <Icon name="refresh" size={14} /> Import vault
      </button>
      <button disabled={busy} onclick={() => run("writeback", finance.writeback)}>
        <Icon name="arrow-right" size={14} /> Write back
      </button>
    </div>
  </section>

  {#if notice}<p class="notice">{notice}</p>{/if}

  {#if upcoming.length > 0}
    <section class="upcoming">
      <h2>Price changes ahead</h2>
      <ul>
        {#each upcoming as item (item.id)}
          <li>
            <strong>{item.name}</strong> goes to
            {money(item.amount_cents, item.currency)} on {item.valid_from}
          </li>
        {/each}
      </ul>
    </section>
  {/if}

  <table>
    <thead>
      <tr>
        <th>Subscription</th>
        <th>State</th>
        <th class="num">Monthly</th>
        <th class="num">Value</th>
        <th>Category</th>
      </tr>
    </thead>
    <tbody>
      {#each rows as row (row.sub.id)}
        <tr class:dim={row.monthlyCents === 0}>
          <td>{row.sub.name}</td>
          <td><span class="state {row.state}">{row.state}</span></td>
          <td class="num">{row.monthlyCents > 0 ? money(row.monthlyCents, row.currency) : "—"}</td>
          <td class="num">{row.sub.value_rating ?? "—"}</td>
          <td class="muted">{row.sub.category ?? "—"}</td>
        </tr>
      {/each}
      {#if rows.length === 0}
        <tr><td colspan="5" class="muted">Nothing imported yet. Import vault reads the notes.</td></tr>
      {/if}
    </tbody>
  </table>
{/if}

<style>
  .bar {
    display: flex;
    flex-wrap: wrap;
    align-items: flex-end;
    gap: 1.5rem;
    margin-bottom: 1.25rem;
  }

  .date {
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
    font-size: 0.75rem;
    color: var(--muted, #888);
  }

  .date input {
    font: inherit;
    font-size: 0.875rem;
    padding: 0.35rem 0.5rem;
    border: 1px solid var(--border, #333);
    border-radius: 6px;
    background: transparent;
    color: inherit;
  }

  .totals {
    display: flex;
    align-items: baseline;
    gap: 1.25rem;
    flex-wrap: wrap;
  }

  .total {
    display: flex;
    align-items: baseline;
    gap: 0.35rem;
  }

  .figure {
    font-size: 1.75rem;
    font-weight: 600;
    font-variant-numeric: tabular-nums;
  }

  .total.secondary .figure {
    font-size: 1.125rem;
    opacity: 0.7;
  }

  .unit {
    font-size: 0.75rem;
    color: var(--muted, #888);
  }

  .count {
    margin: 0;
    font-size: 0.75rem;
  }

  .actions {
    display: flex;
    gap: 0.5rem;
    margin-left: auto;
  }

  .actions button {
    display: inline-flex;
    align-items: center;
    gap: 0.35rem;
    font: inherit;
    font-size: 0.8125rem;
    padding: 0.4rem 0.7rem;
    border: 1px solid var(--border, #333);
    border-radius: 6px;
    background: transparent;
    color: inherit;
    cursor: pointer;
  }

  .actions button:disabled {
    opacity: 0.5;
    cursor: default;
  }

  .upcoming {
    margin-bottom: 1.25rem;
    padding: 0.75rem 1rem;
    border: 1px solid var(--border, #333);
    border-radius: 8px;
  }

  .upcoming h2 {
    margin: 0 0 0.5rem;
    font-size: 0.6875rem;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--primary);
  }

  .upcoming ul {
    margin: 0;
    padding-left: 1.1rem;
    font-size: 0.875rem;
  }

  table {
    width: 100%;
    border-collapse: collapse;
    font-size: 0.875rem;
  }

  th,
  td {
    text-align: left;
    padding: 0.5rem 0.6rem;
    border-bottom: 1px solid var(--border, #2a2a2a);
  }

  th {
    font-size: 0.6875rem;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--muted, #888);
  }

  .num {
    text-align: right;
    font-variant-numeric: tabular-nums;
  }

  tr.dim td {
    opacity: 0.55;
  }

  .state {
    font-size: 0.6875rem;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    padding: 0.1rem 0.4rem;
    border-radius: 4px;
    border: 1px solid var(--border, #333);
  }

  .state.active {
    border-color: var(--primary);
    color: var(--primary);
  }

  .state.paused,
  .state.cancelled {
    opacity: 0.6;
  }

  .muted {
    color: var(--muted, #888);
  }

  .notice {
    font-size: 0.8125rem;
    margin: 0 0 1rem;
  }

  .err {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    font-size: 0.875rem;
  }
</style>
