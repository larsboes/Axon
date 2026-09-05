<script lang="ts">
  import type { PlanCost } from "$lib/travel/api";

  /**
   * What the trip was meant to cost, what was committed to, and what was paid.
   *
   * It reports and does not reconcile: money is a human-confirm domain, and
   * three numbers from three sources that nearly agree is information, while one
   * number that hides which source it came from is not.
   *
   * The rule this component exists to keep: it never renders a 0 for an unknown.
   * Every figure the server sent as null renders as "unknown", with the reason
   * beside it — `sources[]` is printed verbatim in the footer, including
   * "finance is not reachable".
   */
  let { cost }: { cost: PlanCost } = $props();

  const money = (cents: number | null, currency: string | null): string =>
    cents === null
      ? "unknown"
      : (cents / 100).toLocaleString("en-GB", {
          style: "currency",
          currency: currency ?? "EUR",
        });

  const actualFigures = $derived([
    { label: "Paid", value: cost.actuals.personal_cents },
    { label: "Gross out", value: cost.actuals.gross_cash_outflow_cents },
    { label: "Reimbursed", value: cost.actuals.reimbursed_cents },
    { label: "Still owed", value: cost.actuals.outstanding_cents },
  ]);
</script>

<section class="cost card" aria-labelledby="cost-title">
  <header>
    <span class="eyebrow">Cost</span>
    <h3 id="cost-title">Planned, committed, paid</h3>
  </header>

  <dl class="headline">
    <div>
      <dt>Budget</dt>
      <dd>{money(cost.planned_cents, cost.currency)}</dd>
    </div>
    <div>
      <dt>Booked</dt>
      <dd>{money(cost.booked_cents, cost.currency)}</dd>
      {#if cost.booked_reason}
        <small>{cost.booked_reason}</small>
      {/if}
    </div>
  </dl>

  {#if cost.by_currency.length > 1}
    <ul class="currencies">
      {#each cost.by_currency as row (row.currency)}
        <li>{money(row.booked_cents, row.currency)} · {row.item_count} items</li>
      {/each}
    </ul>
  {/if}

  <div class="actuals">
    <h4>Actually paid</h4>
    {#if cost.actuals.ok && cost.actuals.posting_count === 0}
      <p class="note">No transactions are tagged to this trip yet.</p>
    {:else}
      <dl>
        {#each actualFigures as figure (figure.label)}
          <div>
            <dt>{figure.label}</dt>
            <dd class:unknown={figure.value === null}>
              {money(figure.value, cost.currency)}
            </dd>
          </div>
        {/each}
      </dl>
      {#if !cost.actuals.ok && cost.actuals.reason}
        <p class="note">{cost.actuals.reason}</p>
      {/if}
    {/if}
  </div>

  {#if cost.by_stage.some((stage) => stage.item_count > 0)}
    <ul class="stages">
      {#each cost.by_stage.filter((stage) => stage.item_count > 0) as stage (stage.stage_id)}
        <li>
          <span>{stage.origin} → {stage.destination}</span>
          <span>{money(stage.booked_cents, cost.currency)}</span>
        </li>
      {/each}
      {#if cost.unattributed.item_count > 0}
        <li class="unattributed">
          <span>Not bound to a stage · {cost.unattributed.item_count} items</span>
          <span>{money(cost.unattributed.booked_cents, cost.currency)}</span>
        </li>
      {/if}
    </ul>
  {/if}

  {#if cost.selected_options.priced_items > 0}
    <p class="offered">
      Chosen options: {cost.selected_options.total?.toFixed(2)} ·
      <span class="note">{cost.selected_options.note}</span>
    </p>
  {/if}

  <footer>
    {#each cost.sources as source (source.source)}
      <span class:failed={!source.ok}>
        {source.source}{source.reason ? `: ${source.reason}` : ""}
      </span>
    {/each}
  </footer>
</section>

<style>
  .cost {
    display: grid;
    gap: 0.75rem;
    padding: 1rem;
  }

  header h3 {
    margin: 0.15rem 0 0;
    font-size: 1rem;
  }

  dl {
    display: flex;
    flex-wrap: wrap;
    gap: 1.25rem;
    margin: 0;
  }

  dt {
    font-size: 0.7rem;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-tertiary);
  }

  dd {
    margin: 0.1rem 0 0;
    font-size: 1rem;
    color: var(--text-primary);
    font-variant-numeric: tabular-nums;
  }

  .headline dd {
    font-size: 1.25rem;
  }

  dd.unknown {
    font-size: 0.9rem;
    color: var(--text-tertiary);
  }

  h4 {
    margin: 0 0 0.35rem;
    font-size: 0.8rem;
    color: var(--text-secondary);
  }

  .currencies,
  .stages {
    display: grid;
    gap: 0.25rem;
    margin: 0;
    padding: 0;
    list-style: none;
    font-size: 0.8rem;
    color: var(--text-secondary);
  }

  .stages li {
    display: flex;
    justify-content: space-between;
    gap: 0.75rem;
  }

  .stages li.unattributed {
    color: var(--text-tertiary);
  }

  .note,
  .offered,
  footer {
    margin: 0;
    font-size: 0.72rem;
    color: var(--text-tertiary);
  }

  footer {
    display: flex;
    flex-wrap: wrap;
    gap: 0.75rem;
    border-top: 1px solid var(--card-border);
    padding-top: 0.5rem;
  }

  footer .failed {
    color: var(--warning);
  }
</style>
