<script lang="ts">
  import type { FinanceDashboard } from "$lib/api";

  let { data }: { data: FinanceDashboard } = $props();

  const groups = [
    { id: "groceries", label: "Groceries", matches: ["groceries"] },
    { id: "canteen", label: "Work canteen", matches: ["canteen"] },
    { id: "eating-out", label: "Eating out", matches: ["dining-out", "fast-food", "bakery"] },
    { id: "going-out", label: "Going out", matches: ["going-out", "bars"] },
  ] as const;

  const months = $derived(data.trend.map((point) => point.month).slice(-12));
  const series = $derived(groups.map((group) => ({
    ...group,
    values: months.map((month) => data.category_trend
      .filter((point) => point.month === month && group.matches.some((match) => point.category.includes(match)))
      .reduce((sum, point) => sum + point.amount_cents, 0)),
  })));
  const maximum = $derived(Math.max(1, ...series.flatMap((item) => item.values)));
  const expenseTotal = $derived(data.category_trend.reduce((sum, point) => sum + point.amount_cents, 0));
  const uncategorized = $derived(data.category_trend
    .filter((point) => point.category.includes("uncategorized"))
    .reduce((sum, point) => sum + point.amount_cents, 0));

  function x(index: number) {
    return 58 + index * (650 / Math.max(1, months.length - 1));
  }

  function y(cents: number) {
    return 220 - cents / maximum * 180;
  }

  function points(values: number[]) {
    return values.map((value, index) => `${x(index)},${y(value)}`).join(" ");
  }

  function money(cents: number) {
    return new Intl.NumberFormat("de-DE", { style: "currency", currency: data.summary.currency }).format(cents / 100);
  }
</script>

<section class="panel category-trend">
  <div class="heading">
    <div>
      <h2>Food and going out by month</h2>
      <p>Reviewed spending in {data.summary.currency}; up to the latest 12 months in the selected period.</p>
    </div>
  </div>

  {#if months.length}
    <div class="legend" aria-label="Series legend">
      {#each series as item (item.id)}
        <span class={item.id}><i></i>{item.label} <strong>{money(item.values.reduce((sum, value) => sum + value, 0))}</strong></span>
      {/each}
    </div>
    <svg viewBox="0 0 740 255" role="img" aria-label="Monthly food and going-out spending line chart">
      <line class="axis" x1="58" y1="220" x2="708" y2="220"></line>
      <line class="guide" x1="58" y1="130" x2="708" y2="130"></line>
      <line class="guide" x1="58" y1="40" x2="708" y2="40"></line>
      <text x="50" y="44" text-anchor="end">{money(maximum)}</text>
      <text x="50" y="134" text-anchor="end">{money(Math.round(maximum / 2))}</text>
      {#each months as month, index (month)}
        {#if index % 2 === 0 || index === months.length - 1}
          <text x={x(index)} y="242" text-anchor="middle">{month}</text>
        {/if}
      {/each}
      {#each series as item (item.id)}
        <polyline class={item.id} points={points(item.values)}></polyline>
        {#each item.values as value, index (`${item.id}-${months[index]}`)}
          <circle class={item.id} cx={x(index)} cy={y(value)} r="3"><title>{item.label}, {months[index]}: {money(value)}</title></circle>
        {/each}
      {/each}
    </svg>
    {#if uncategorized > 0}
      <p class="coverage"><strong>{money(uncategorized)}</strong> remains uncategorized ({(uncategorized / Math.max(1, expenseTotal) * 100).toFixed(1)}% of reviewed spending in this period). Category comparisons are provisional until that share is reduced.</p>
    {/if}
  {:else}
    <p class="muted">No reviewed expense history in this period.</p>
  {/if}
</section>

<style>
  .panel { border: 1px solid var(--border, #333); border-radius: 8px; background: var(--card-bg, transparent); padding: .9rem; margin-bottom: .75rem; min-width: 0; }
  h2 { margin: 0; font-size: .85rem; }
  p { margin: .2rem 0 0; color: var(--muted, #888); font-size: .7rem; }
  .legend { display: flex; flex-wrap: wrap; gap: .55rem 1rem; margin: .8rem 0 .2rem; font-size: .68rem; }
  .legend span { display: inline-flex; align-items: center; gap: .3rem; }
  .legend i { width: .8rem; height: 2px; background: currentColor; }
  .legend strong { margin-left: .15rem; font-variant-numeric: tabular-nums; }
  svg { width: 100%; min-height: 14rem; overflow: visible; }
  svg text { fill: var(--muted, #888); font-size: 11px; font-variant-numeric: tabular-nums; }
  .axis, .guide { stroke: color-mix(in srgb, currentColor 22%, transparent); stroke-width: 1; }
  .guide { stroke-dasharray: 3 5; }
  polyline { fill: none; stroke: currentColor; stroke-width: 2; stroke-linejoin: round; stroke-linecap: round; }
  circle { fill: var(--card-bg, #111); stroke: currentColor; stroke-width: 2; }
  .groceries { color: #c58b32; }
  .canteen { color: #4f7fa8; }
  .eating-out { color: #a65f4c; }
  .going-out { color: #777; }
  .coverage { border-top: 1px solid var(--border, #333); padding-top: .65rem; line-height: 1.45; }
  .muted { color: var(--muted, #888); }
</style>
