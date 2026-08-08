<script lang="ts">
  import type { ReviewedHoldingsSnapshot } from "$lib/api";

  let { snapshot }: { snapshot: ReviewedHoldingsSnapshot | null } = $props();

  function decimal(mantissa: string, scale: number): string {
    const sign = mantissa.startsWith("-") ? "-" : "";
    const digits = (sign ? mantissa.slice(1) : mantissa).padStart(scale + 1, "0");
    if (scale === 0) return `${sign}${digits}`;
    return `${sign}${digits.slice(0, -scale)}.${digits.slice(-scale)}`;
  }
</script>

<section class="portfolio">
  <div class="heading">
    <div>
      <h2>Portfolio holdings</h2>
      {#if snapshot}<p>Reviewed {snapshot.reviewed_at}; prices are from the latest activity, not a live quote.</p>{/if}
    </div>
    {#if snapshot}<strong>{snapshot.holdings.length} open</strong>{/if}
  </div>
  {#if snapshot === null}
    <p>No reviewed holdings snapshot yet.</p>
  {:else if snapshot.holdings.length === 0}
    <p>The reviewed snapshot contains no open positions.</p>
  {:else}
    <div class="table-wrap">
      <table>
        <thead><tr><th>Instrument</th><th>Quantity</th><th>Latest activity price</th></tr></thead>
        <tbody>
          {#each snapshot.holdings as holding (holding.instrument)}
            <tr>
              <td>{holding.instrument}</td>
              <td>{decimal(holding.quantity.mantissa, holding.quantity.scale)}</td>
              <td>{holding.latest_unit_price === null ? "—" : `${decimal(holding.latest_unit_price.mantissa, holding.latest_unit_price.scale)} ${holding.currency}`}</td>
            </tr>
          {/each}
        </tbody>
      </table>
    </div>
  {/if}
</section>

<style>
  .portfolio { margin-top: .75rem; padding: .9rem; border: 1px solid var(--border, #333); border-radius: 8px; background: var(--card-bg, transparent); }
  .heading { display: flex; align-items: start; justify-content: space-between; gap: 1rem; }
  h2 { margin: 0; font-size: .85rem; }
  p { margin: .2rem 0 0; color: var(--muted, #888); font-size: .7rem; }
  .heading > strong { font-size: .78rem; font-variant-numeric: tabular-nums; }
  .table-wrap { margin-top: .75rem; overflow-x: auto; }
  table { width: 100%; border-collapse: collapse; font-size: .78rem; }
  th, td { padding: .5rem .4rem; border-top: 1px solid var(--border, #333); text-align: left; }
  th:nth-child(n+2), td:nth-child(n+2) { text-align: right; font-variant-numeric: tabular-nums; }
</style>
