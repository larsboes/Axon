<script lang="ts">
  import type { FinanceDashboard } from "$lib/api";

  let { data, throughDate }: { data: FinanceDashboard; throughDate: string } = $props();

  type Step = { label: string; start: number; end: number; delta: number; total: boolean };

  const comparison = $derived.by(() => {
    const currentMonth = throughDate.slice(0, 7);
    const cutoffDay = Number(throughDate.slice(8, 10));
    if (!/^\d{4}-\d{2}$/.test(currentMonth) || !Number.isInteger(cutoffDay)) return null;
    const baselineMonths = [3, 2, 1].map((offset) => shiftMonth(currentMonth, -offset));

    const expenseRows = data.transactions.filter((row) => row.kind === "expense");
    const categories = new Set(expenseRows.map((row) => row.category));
    const amount = (month: string, category: string) => expenseRows
      .filter((row) => row.date.startsWith(month)
        && Number(row.date.slice(8, 10)) <= cutoffDay
        && row.category === category)
      .reduce((sum, row) => sum + row.amount_cents, 0);
    const median = (values: number[]) => {
      const sorted = [...values].sort((left, right) => left - right);
      const middle = Math.floor(sorted.length / 2);
      return sorted.length % 2 ? sorted[middle] : Math.round((sorted[middle - 1] + sorted[middle]) / 2);
    };
    const drivers = [...categories].map((category) => {
      const baseline = median(baselineMonths.map((month) => amount(month, category)));
      const current = amount(currentMonth, category);
      return { category, baseline, current, delta: current - baseline };
    });
    const baselineTotal = drivers.reduce((sum, driver) => sum + driver.baseline, 0);
    const currentTotal = drivers.reduce((sum, driver) => sum + driver.current, 0);
    const ranked = drivers.sort((left, right) => Math.abs(right.delta) - Math.abs(left.delta));
    const visible = ranked.slice(0, 5);
    const otherDelta = ranked.slice(5).reduce((sum, driver) => sum + driver.delta, 0);
    const deltas = otherDelta === 0 ? visible : [...visible, { category: "other", baseline: 0, current: 0, delta: otherDelta }];
    let running = baselineTotal;
    const steps: Step[] = [{ label: "3-month median", start: 0, end: baselineTotal, delta: baselineTotal, total: true }];
    for (const driver of deltas) {
      const start = running;
      running += driver.delta;
      steps.push({ label: short(driver.category), start, end: running, delta: driver.delta, total: false });
    }
    steps.push({ label: currentMonth, start: 0, end: currentTotal, delta: currentTotal, total: true });
    const bounds = steps.flatMap((step) => [step.start, step.end, 0]);
    return {
      currentMonth,
      cutoffDay,
      baselineMonths,
      baselineTotal,
      currentTotal,
      steps,
      minimum: Math.min(...bounds),
      maximum: Math.max(1, ...bounds),
    };
  });

  function shiftMonth(month: string, offset: number) {
    const [year, oneBasedMonth] = month.split("-").map(Number);
    const shifted = new Date(Date.UTC(year, oneBasedMonth - 1 + offset, 1));
    return shifted.toISOString().slice(0, 7);
  }

  function short(account: string) {
    if (account === "other") return "Other";
    return account.replace(/^expenses:/, "").split(":").join(" · ");
  }

  function money(cents: number) {
    return new Intl.NumberFormat("de-DE", { style: "currency", currency: data.summary.currency }).format(cents / 100);
  }

  function top(step: Step) {
    if (!comparison) return 0;
    const range = Math.max(1, comparison.maximum - comparison.minimum);
    return (comparison.maximum - Math.max(step.start, step.end)) / range * 100;
  }

  function height(step: Step) {
    if (!comparison) return 0;
    return Math.max(1.5, Math.abs(step.end - step.start) / Math.max(1, comparison.maximum - comparison.minimum) * 100);
  }
</script>

<section class="panel drivers">
  <div class="heading">
    <div>
      <h2>What changed this month</h2>
      <p>Personal spending month to date versus the same day range in the preceding three calendar months.</p>
    </div>
    {#if comparison}
      <strong class:negative={comparison.currentTotal > comparison.baselineTotal}>
        {comparison.currentTotal >= comparison.baselineTotal ? "+" : ""}{money(comparison.currentTotal - comparison.baselineTotal)}
      </strong>
    {/if}
  </div>

  {#if comparison}
    <div class="waterfall-shell">
      <div class="waterfall" style={`grid-template-columns: repeat(${comparison.steps.length}, minmax(5.5rem, 1fr))`}>
        {#each comparison.steps as step, index (`${step.label}-${index}`)}
          <article>
            <div class="plot">
              <span
                class:total={step.total}
                class:increase={!step.total && step.delta > 0}
                class:decrease={!step.total && step.delta < 0}
                style:top={`${top(step)}%`}
                style:height={`${height(step)}%`}
              ></span>
            </div>
            <strong>{step.total ? money(step.end) : `${step.delta > 0 ? "+" : ""}${money(step.delta)}`}</strong>
            <small>{step.label}</small>
          </article>
        {/each}
      </div>
    </div>
    <p class="note">Through day {comparison.cutoffDay}. Baseline months: {comparison.baselineMonths.join(", ")}. Positive drivers increased spending; negative drivers reduced it.</p>
  {:else}
    <p class="muted">At least two months of reviewed expense history are needed for a driver comparison.</p>
  {/if}
</section>

<style>
  .panel { border: 1px solid var(--border, #333); border-radius: 8px; background: var(--card-bg, transparent); padding: .9rem; margin-bottom: .75rem; min-width: 0; }
  .heading { display: flex; align-items: start; justify-content: space-between; gap: 1rem; }
  h2 { margin: 0; font-size: .85rem; }
  p { margin: .2rem 0 0; color: var(--muted, #888); font-size: .67rem; }
  .heading > strong { font-size: 1rem; font-variant-numeric: tabular-nums; }
  .negative { color: var(--danger, #b44); }
  .waterfall-shell { overflow-x: auto; margin-top: .8rem; }
  .waterfall { display: grid; gap: .3rem; min-width: 42rem; height: 13rem; border-block: 1px solid color-mix(in srgb, currentColor 14%, transparent); }
  article { display: grid; grid-template-rows: 1fr auto auto; gap: .18rem; min-width: 0; text-align: center; font-size: .65rem; }
  .plot { position: relative; min-height: 9.5rem; background: linear-gradient(to bottom, transparent 49.7%, color-mix(in srgb, currentColor 10%, transparent) 50%, transparent 50.3%); }
  .plot span { position: absolute; left: 16%; width: 68%; min-height: 2px; border-radius: 2px; background: #777; }
  .plot span.total { background: #4f7fa8; }
  .plot span.increase { background: #a65f4c; }
  .plot span.decrease { background: #708273; }
  article > strong { font-variant-numeric: tabular-nums; }
  article > small { color: var(--muted, #888); line-height: 1.2; overflow-wrap: anywhere; }
  .note { border-top: 1px solid var(--border, #333); padding-top: .6rem; margin-top: .7rem; }
  .muted { margin-top: .8rem; }
</style>
