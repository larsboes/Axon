<script lang="ts">
  import { exactMoney } from "$lib/finance/money";
  import type { Portfolio, PriceStatus, Position } from "$lib/finance/invest-api";

  let { portfolio, prices }: { portfolio: Portfolio; prices: PriceStatus | null } = $props();

  // Formatting only. Every figure below arrives computed: the share, the drift,
  // the freshness tag and the value all come from capabilities/finance/src/portfolio.rs,
  // and this component divides nothing and sums nothing.
  const percent = (bp: number) => `${(bp / 100).toFixed(2)}%`;
  const signedPercent = (bp: number) => `${bp > 0 ? "+" : ""}${(bp / 100).toFixed(2)}%`;

  // The bar width. A percentage of the widest share in the table, so a portfolio
  // of small positions still reads as a shape rather than as eight slivers.
  const widest = $derived(
    Math.max(100, ...portfolio.positions.map((position) => position.share_bp)),
  );
  const barWidth = (position: Position) => `${(position.share_bp / widest) * 100}%`;
  const targetOffset = (position: Position) =>
    position.target_bp === null ? "0%" : `${(position.target_bp / widest) * 100}%`;

  const freshnessLabel: Record<string, string> = {
    fresh: "fresh",
    stale: "stale",
    none: "no quote",
  };
</script>

<section class="investments">
  <div class="heading">
    <div>
      <h2>Positions</h2>
      <p>
        Valued at the market price where one was observed, and at the reviewed broker
        activity price otherwise — each row says which. Change is measured against that
        reviewed price, so it is not a return and not P&amp;L: this capability holds no lot
        and no cost basis.
      </p>
    </div>
    <strong>{exactMoney(portfolio.total.mantissa, portfolio.total.scale, portfolio.currency)}</strong>
  </div>

  {#if !portfolio.targets_configured}
    <p class="no-policy">
      <strong>No target allocation is declared yet.</strong>
      Add a <code>targets</code> block to <code>&lt;overlay&gt;/config/finance.json</code>
      — see <code>schemas/finance.json.example</code> — and drift, rebalance proposals and
      the Home band all begin working. Until then these are positions and shares, with
      nothing to be off target from.
    </p>
  {/if}

  {#if portfolio.positions.length === 0}
    <p>The reviewed snapshot holds no open positions.</p>
  {:else}
    <div class="table-wrap">
      <table>
        <thead>
          <tr>
            <th scope="col">Position</th>
            <th scope="col">Price</th>
            <th scope="col">Value</th>
            <th scope="col">Share</th>
            <th scope="col">Drift</th>
          </tr>
        </thead>
        <tbody>
          {#each portfolio.positions as position (position.instrument)}
            <tr>
              <td class="name">
                <span class="label">{position.label}</span>
                <span class="meta">{position.instrument} · {position.asset_class}</span>
              </td>
              <td>
                {#if position.market_price}
                  {exactMoney(position.market_price.mantissa, position.market_price.scale, position.currency)}
                  <span class="meta">
                    {position.market_price_source} · {position.market_price_observed_on}
                    <span class="tag {position.price_freshness}">{freshnessLabel[position.price_freshness]}</span>
                  </span>
                {:else if position.review_price}
                  {exactMoney(position.review_price.mantissa, position.review_price.scale, position.currency)}
                  <span class="meta">reviewed import <span class="tag none">no quote</span></span>
                {:else}
                  —
                {/if}
              </td>
              <td>
                {exactMoney(position.value.mantissa, position.value.scale, position.currency)}
                {#if position.change_since_review_bp !== null}
                  <span class="meta">{signedPercent(position.change_since_review_bp)} since review</span>
                {/if}
              </td>
              <td>
                {percent(position.share_bp)}
                <span class="bar" aria-hidden="true">
                  <span class="fill" class:outside={position.outside_band} style:width={barWidth(position)}></span>
                  {#if position.target_bp !== null}
                    <span class="target" style:left={targetOffset(position)}></span>
                  {/if}
                </span>
              </td>
              <td>
                {#if position.drift_bp === null}
                  <span class="meta">no target</span>
                {:else}
                  <span class:outside={position.outside_band}>{signedPercent(position.drift_bp)}</span>
                  <span class="meta">target {percent(position.target_bp ?? 0)} ± {percent(position.band_bp ?? 0)}</span>
                {/if}
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
    </div>
  {/if}

  {#if portfolio.asset_classes.some((row) => row.target_bp !== null)}
    <div class="table-wrap">
      <table>
        <thead>
          <tr><th scope="col">Asset class</th><th scope="col">Value</th><th scope="col">Share</th><th scope="col">Drift</th></tr>
        </thead>
        <tbody>
          {#each portfolio.asset_classes as row (row.asset_class)}
            <tr>
              <td class="name">{row.asset_class}</td>
              <td>{exactMoney(row.value.mantissa, row.value.scale, portfolio.currency)}</td>
              <td>{percent(row.share_bp)}</td>
              <td>
                {#if row.drift_bp === null}
                  <span class="meta">no target</span>
                {:else}
                  <span class:outside={row.outside_band}>{signedPercent(row.drift_bp)}</span>
                  <span class="meta">target {percent(row.target_bp ?? 0)} ± {percent(row.band_bp ?? 0)}</span>
                {/if}
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
    </div>
  {/if}

  {#if portfolio.caveats.length > 0}
    <ul class="caveats">
      {#each portfolio.caveats as caveat (caveat)}<li>{caveat}</li>{/each}
    </ul>
  {/if}

  {#if prices}
    <div class="prices">
      <h3>Price sources</h3>
      <ul>
        {#each prices.providers as provider (provider.name)}
          <li>
            <span>{provider.name}</span>
            <span class="meta">
              {#if provider.last_status === null}
                never run
              {:else}
                {provider.last_status} · {provider.last_fetched_at}{provider.last_detail ? ` · ${provider.last_detail}` : ""}
              {/if}
            </span>
          </li>
        {/each}
      </ul>
      <p class="meta">
        A price older than {prices.freshness_days} day{prices.freshness_days === 1 ? "" : "s"}
        reads as stale. Refresh by hand with <code>finance-cli prices fetch</code>; the
        <code>finance-prices</code> job does it nightly.
      </p>
    </div>
  {/if}
</section>

<style>
  .investments { margin-top: .75rem; padding: .9rem; border: 1px solid var(--card-border); border-radius: var(--radius-md); background: var(--card-bg); }
  .heading { display: flex; align-items: start; justify-content: space-between; gap: 1rem; }
  h2 { margin: 0; font-size: .85rem; }
  h3 { margin: 0 0 .35rem; font-size: .75rem; }
  p { margin: .2rem 0 0; color: var(--text-secondary); font-size: .7rem; }
  .heading > strong { font-size: .95rem; font-variant-numeric: tabular-nums; white-space: nowrap; }
  .no-policy { margin-top: .65rem; padding: .55rem .65rem; border-left: 3px solid var(--warning); background: var(--warning-soft); color: var(--text-primary); font-size: .72rem; }
  .no-policy code { font-family: var(--font-mono); font-size: .95em; }
  .table-wrap { margin-top: .75rem; overflow-x: auto; }
  table { width: 100%; border-collapse: collapse; font-size: .78rem; }
  th, td { padding: .5rem .4rem; border-top: 1px solid var(--card-border); text-align: left; vertical-align: top; }
  th:nth-child(n+2), td:nth-child(n+2) { text-align: right; font-variant-numeric: tabular-nums; }
  td.name { text-align: left; }
  .label { display: block; }
  .meta { display: block; color: var(--text-tertiary); font-size: .66rem; font-variant-numeric: tabular-nums; }
  .tag { display: inline-block; padding: 0 .3rem; border-radius: var(--radius-sm); font-size: .95em; }
  .tag.fresh { background: var(--success-soft); color: var(--success); }
  .tag.stale { background: var(--warning-soft); color: var(--warning-ink); }
  .tag.none { background: var(--accent-soft); color: var(--text-secondary); }
  .outside { color: var(--danger); }
  /* Two CSS tokens and a percentage width. No charting dependency: the build
     fails on any statically reachable chunk over 500 KB, and a drift bar is a
     div. */
  .bar { position: relative; display: block; height: 4px; margin-top: .3rem; border-radius: 2px; background: var(--accent-soft); overflow: hidden; }
  .bar .fill { position: absolute; inset: 0 auto 0 0; background: var(--primary); }
  .bar .fill.outside { background: var(--danger); }
  .bar .target { position: absolute; top: 0; bottom: 0; width: 2px; background: var(--text-primary); }
  .caveats { margin: .65rem 0 0; padding-left: 1rem; color: var(--text-secondary); font-size: .7rem; }
  .prices { margin-top: .9rem; padding-top: .65rem; border-top: 1px solid var(--card-border); }
  .prices ul { margin: 0; padding: 0; list-style: none; display: flex; flex-wrap: wrap; gap: .5rem; }
  .prices li { padding: .3rem .45rem; border: 1px solid var(--card-border); border-radius: var(--radius-sm); font-size: .72rem; }
  .prices code { font-family: var(--font-mono); }
</style>
