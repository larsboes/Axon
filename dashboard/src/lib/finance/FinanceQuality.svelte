<script lang="ts">
  import type { FinanceDashboard } from "$lib/api";

  let { data }: { data: FinanceDashboard } = $props();

  function percent(value: number | null) {
    return value === null ? "—" : `${value.toFixed(1)}%`;
  }

  function date(value: string | null) {
    return value?.slice(0, 10) ?? "No data";
  }
</script>

<section class="quality" aria-labelledby="finance-quality-title">
  <div class="heading">
    <div>
      <h2 id="finance-quality-title">Analysis readiness</h2>
      <p>Coverage is shown beside the results so provisional comparisons stay visible.</p>
    </div>
    <strong class:warning={(data.quality.categorization_value_percent ?? 0) < 80}>
      {percent(data.quality.categorization_value_percent)} categorized by value
    </strong>
  </div>

  <div class="metrics">
    <article>
      <span>Expense postings</span>
      <strong>{data.quality.categorized_expense_posting_count} / {data.quality.expense_posting_count}</strong>
      <small>{percent(data.quality.categorization_count_percent)} categorized</small>
    </article>
    <article>
      <span>Selected period</span>
      <strong>{data.quality.observed_months} / {data.quality.expected_months} months</strong>
      <small>{data.quality.first_transaction_date ?? "No transactions"} to {data.quality.latest_transaction_date ?? "—"}</small>
    </article>
    {#each data.source_freshness as source (source.source)}
      <article>
        <span>{source.label}</span>
        <strong>{date(source.as_of)}</strong>
        <small class:warning={source.freshness !== "current"} class={source.coverage}>{source.freshness}{source.age_days === null ? "" : ` · ${source.age_days}d old`} · {source.source === "journal" ? "projected transactions" : `${source.coverage} coverage`}</small>
      </article>
    {/each}
  </div>
</section>

<style>
  .quality { border-block: 1px solid var(--border, #333); padding: .8rem 0; margin-bottom: .9rem; }
  .heading { display: flex; align-items: baseline; justify-content: space-between; gap: 1rem; margin-bottom: .65rem; }
  h2 { margin: 0; font-size: .82rem; }
  p, small, span { color: var(--muted, #888); }
  p { margin: .15rem 0 0; font-size: .68rem; }
  .heading > strong { font-size: .72rem; font-variant-numeric: tabular-nums; }
  .heading > strong.warning, small.warning, small.partial, small.missing { color: #a65f4c; }
  .metrics { display: grid; grid-template-columns: repeat(auto-fit, minmax(9rem, 1fr)); gap: .5rem 1rem; }
  article { display: grid; gap: .1rem; min-width: 0; }
  article span, article small { font-size: .63rem; }
  article strong { font-size: .78rem; font-variant-numeric: tabular-nums; overflow-wrap: anywhere; }
  small.complete { color: #4f7fa8; }
  @media (max-width: 650px) { .heading { align-items: start; flex-direction: column; } }
</style>
