<script lang="ts">
  import type { FinanceTransaction } from "$lib/api";
  let { rows }: { rows: FinanceTransaction[] } = $props();
  const money = (cents: number, currency: string) =>
    new Intl.NumberFormat("de-DE", { style: "currency", currency }).format(cents / 100);
  const short = (account: string) => account.split(":").slice(1).join(" · ") || account;
</script>

<div class="table-wrap"><table>
  <thead><tr><th>Date</th><th>Description</th><th>Account</th><th>Category</th><th class="num">Amount</th></tr></thead>
  <tbody>
    {#each rows as row (row.id)}
      <tr><td>{row.date}</td><td>{row.description}</td><td>{short(row.account)}</td><td>{short(row.category)}</td><td class="num {row.kind}">{row.kind === "expense" ? "−" : row.kind === "income" ? "+" : ""}{money(row.amount_cents, row.currency)}</td></tr>
    {/each}
    {#if rows.length === 0}<tr><td colspan="5" class="muted">No matching transactions.</td></tr>{/if}
  </tbody>
</table></div>

<style>
  .table-wrap { overflow-x: auto; margin-top: .65rem; }
  table { width: 100%; border-collapse: collapse; font-size: .75rem; }
  th, td { text-align: left; padding: .48rem .5rem; border-bottom: 1px solid var(--border, #333); white-space: nowrap; }
  th { color: var(--muted, #888); font-size: .62rem; text-transform: uppercase; letter-spacing: .04em; }
  td:nth-child(2) { white-space: normal; min-width: 11rem; }
  .num { text-align: right; font-variant-numeric: tabular-nums; }
  td.income { color: var(--primary); }
  td.transfer, .muted { color: var(--muted, #888); }
</style>
