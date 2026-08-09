<script lang="ts">
  import { onMount } from "svelte";
  import { trips, type FinanceDashboard, type SpendingPurpose, type TripPlan } from "$lib/api";

  let { data }: { data: FinanceDashboard } = $props();
  let plans = $state<TripPlan[]>([]);

  const labels: Record<SpendingPurpose, string> = {
    day_to_day: "Day to day",
    trip: "Trips",
    work: "Work",
    housing: "Housing",
    other: "Other",
  };
  const purposeRows = $derived([...data.purpose_spending].sort((left, right) => right.personal_spending_cents - left.personal_spending_cents));
  const purposeMaximum = $derived(Math.max(1, ...purposeRows.map((row) => row.personal_spending_cents)));
  const assignedCents = $derived(purposeRows.filter((row) => row.purpose !== null).reduce((sum, row) => sum + row.personal_spending_cents, 0));
  const purposeCoverage = $derived(data.summary.personal_spending_cents > 0
    ? assignedCents / data.summary.personal_spending_cents * 100
    : null);

  function money(cents: number) {
    return new Intl.NumberFormat("de-DE", { style: "currency", currency: data.summary.currency }).format(cents / 100);
  }

  function purposeLabel(purpose: SpendingPurpose | null) {
    return purpose === null ? "Purpose not reviewed" : labels[purpose];
  }

  function tripLabel(id: string) {
    return plans.find((plan) => plan.id === id)?.title ?? "Trip plan unavailable";
  }

  onMount(async () => {
    plans = await trips.list().catch(() => []);
  });
</script>

<section class="panel purpose-overview">
  <div class="heading">
    <div>
      <h2>Why the spending happened</h2>
      <p>Purpose is independent of category: a restaurant remains food whether it was day-to-day or part of a trip.</p>
    </div>
    <strong>{purposeCoverage === null ? "—" : `${purposeCoverage.toFixed(1)}%`} purpose-reviewed</strong>
  </div>

  <div class="purpose-grid">
    <div class="purpose-bars">
      {#each purposeRows as row (`${row.purpose ?? "unreviewed"}`)}
        <article class:unreviewed={row.purpose === null}>
          <span>{purposeLabel(row.purpose)}</span>
          <div><i style:width={`${Math.max(1, row.personal_spending_cents / purposeMaximum * 100)}%`}></i></div>
          <strong>{money(row.personal_spending_cents)}</strong>
          <small>{row.expense_posting_count} posting{row.expense_posting_count === 1 ? "" : "s"}</small>
        </article>
      {/each}
    </div>

    <div class="trip-summary">
      <h3>Reviewed trips in this period</h3>
      {#if data.trip_spending.length}
        {#each data.trip_spending as trip (trip.trip_id)}
          <article>
            <div><strong>{tripLabel(trip.trip_id)}</strong><small>{trip.expense_posting_count} reviewed posting{trip.expense_posting_count === 1 ? "" : "s"}</small></div>
            <span><small>Your cost</small><strong>{money(trip.personal_spending_cents)}</strong></span>
            <span><small>Gross paid</small><strong>{money(trip.gross_cash_outflow_cents)}</strong></span>
            <span><small>Repaid</small><strong>{money(trip.reimbursed_cents)}</strong></span>
            <span><small>Still owed</small><strong>{money(trip.outstanding_cents)}</strong></span>
          </article>
        {/each}
      {:else}
        <p>No expenses in this period have been assigned to a Trips plan yet.</p>
      {/if}
    </div>
  </div>
</section>

<style>
  .panel { border: 1px solid var(--border, #333); border-radius: 8px; background: var(--card-bg, transparent); padding: .9rem; margin-bottom: .75rem; min-width: 0; }
  .heading { display: flex; align-items: start; justify-content: space-between; gap: 1rem; }
  h2, h3 { margin: 0; font-size: .85rem; }
  h3 { font-size: .76rem; }
  p, small { margin: .2rem 0 0; color: var(--muted, #888); font-size: .67rem; }
  .heading > strong { font-size: .72rem; font-variant-numeric: tabular-nums; }
  .purpose-grid { display: grid; grid-template-columns: minmax(18rem, .8fr) minmax(24rem, 1.2fr); gap: 1.2rem; margin-top: .8rem; }
  .purpose-bars { display: grid; align-content: start; gap: .55rem; }
  .purpose-bars article { display: grid; grid-template-columns: 7rem 1fr 6.5rem; align-items: center; gap: .55rem; font-size: .68rem; }
  .purpose-bars article > div { height: 7px; border-radius: 4px; background: color-mix(in srgb, currentColor 8%, transparent); overflow: hidden; }
  .purpose-bars i { display: block; height: 100%; border-radius: inherit; background: #4f7fa8; }
  .purpose-bars .unreviewed i { background: #a65f4c; }
  .purpose-bars article > strong { text-align: right; font-variant-numeric: tabular-nums; }
  .purpose-bars article > small { grid-column: 2 / 4; margin-top: -.35rem; }
  .trip-summary { border-left: 1px solid var(--border, #333); padding-left: 1rem; }
  .trip-summary > article { display: grid; grid-template-columns: minmax(12rem, 1fr) repeat(4, auto); align-items: center; gap: .8rem; padding: .55rem 0; border-bottom: 1px solid var(--border, #333); font-size: .68rem; }
  .trip-summary article > div, .trip-summary article > span { display: grid; gap: .12rem; }
  .trip-summary article > span { justify-items: end; }
  .trip-summary article > span strong { font-variant-numeric: tabular-nums; }
  @media (max-width: 850px) { .purpose-grid { grid-template-columns: 1fr; } .trip-summary { border-left: 0; border-top: 1px solid var(--border, #333); padding: .8rem 0 0; } }
  @media (max-width: 620px) { .heading { flex-direction: column; } .trip-summary > article { grid-template-columns: 1fr 1fr; } .trip-summary article > span { justify-items: start; } }
</style>
