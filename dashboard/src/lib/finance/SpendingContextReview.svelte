<script lang="ts">
  import { onMount } from "svelte";
  import Icon from "$lib/Icon.svelte";
  import {
    axonStatus,
    finance,
    trips,
    type FinanceDashboard,
    type SpendingPurpose,
    type TransactionCandidate,
    type TripPlan,
  } from "$lib/api";

  let { data, onchanged }: { data: FinanceDashboard; onchanged?: () => void | Promise<void> } = $props();
  let candidates = $state<TransactionCandidate[]>([]);
  let plans = $state<TripPlan[]>([]);
  let expenseId = $state("");
  let inflowId = $state("");
  let reimbursementTarget = $state("");
  let personalAmount = $state("");
  let purpose = $state<SpendingPurpose>("day_to_day");
  let tripId = $state("");
  let reviewTripId = $state("");
  let tripProjection = $state<FinanceDashboard | null>(null);
  let tripBusy = $state(false);
  let busy = $state(false);
  let error = $state<string | null>(null);
  let notice = $state<string | null>(null);

  const expenses = $derived(candidates.filter((candidate) =>
    candidate.state === "confirmed"
      && candidate.amount_cents < 0
      && candidate.proposed_account.startsWith("expenses:"),
  ));
  const inflows = $derived(candidates.filter((candidate) =>
    candidate.state === "confirmed"
      && candidate.amount_cents > 0
      && candidate.proposed_account !== "assets:receivable:shared",
  ));
  const selectedExpense = $derived(expenses.find((candidate) => candidate.id === expenseId) ?? null);
  const selectedInflow = $derived(inflows.find((candidate) => candidate.id === inflowId) ?? null);
  const selectedTarget = $derived(
    (tripProjection?.shared_expenses ?? data.shared_expenses)
      .find((expense) => expense.candidate_id === reimbursementTarget) ?? null,
  );
  const selectedPlan = $derived(plans.find((plan) => plan.id === reviewTripId) ?? null);
  const tripCandidates = $derived(selectedPlan === null ? [] : expenses.filter((candidate) =>
    candidate.booked_at >= selectedPlan.date_start && candidate.booked_at <= selectedPlan.date_end,
  ));
  const tripRows = $derived((tripProjection?.transactions ?? []).filter((row) =>
    row.kind === "expense" && row.trip_id === reviewTripId,
  ));
  const tripSharedExpenses = $derived((tripProjection?.shared_expenses ?? []).filter((expense) =>
    expense.trip_id === reviewTripId,
  ));
  const tripPersonalCents = $derived(tripRows.reduce((total, row) => total + row.amount_cents, 0));
  const tripGrossCents = $derived(tripRows.reduce((total, row) => total + row.cash_amount_cents, 0));
  const tripOutstandingCents = $derived(tripSharedExpenses.reduce((total, expense) => total + expense.outstanding_cents, 0));
  const visibleSharedExpenses = $derived(selectedPlan ? tripSharedExpenses : data.shared_expenses);

  const money = (cents: number, currency: string) =>
    new Intl.NumberFormat("de-DE", { style: "currency", currency }).format(cents / 100);

  async function load() {
    try {
      candidates = await finance.candidates();
      try {
        plans = await trips.list();
      } catch {
        await axonStatus.start("trips").catch(() => undefined);
        plans = await trips.list().catch(() => []);
      }
      error = null;
    } catch (cause) {
      error = cause instanceof Error ? cause.message : String(cause);
    }
  }

  async function loadTrip(plan: TripPlan) {
    tripBusy = true;
    try {
      tripProjection = await finance.dashboard({
        start: plan.date_start,
        end: plan.date_end,
        currency: "EUR",
      });
      error = null;
    } catch (cause) {
      tripProjection = null;
      error = cause instanceof Error ? cause.message : String(cause);
    } finally {
      tripBusy = false;
    }
  }

  async function chooseTrip(event: Event) {
    reviewTripId = (event.currentTarget as HTMLSelectElement).value;
    expenseId = "";
    personalAmount = "";
    const plan = plans.find((candidate) => candidate.id === reviewTripId);
    if (!plan) {
      tripProjection = null;
      purpose = "day_to_day";
      tripId = "";
      return;
    }
    purpose = "trip";
    tripId = plan.id;
    await loadTrip(plan);
  }

  function chooseExpense(event: Event) {
    selectExpense((event.currentTarget as HTMLSelectElement).value);
  }

  function selectExpense(id: string) {
    expenseId = id;
    const candidate = expenses.find((entry) => entry.id === expenseId);
    if (!candidate) return;
    const existing = data.transactions.find((row) => row.source_id === candidate.fingerprint);
    personalAmount = ((existing?.amount_cents ?? -candidate.amount_cents) / 100)
      .toFixed(2)
      .replace(".", ",");
    purpose = reviewTripId ? "trip" : existing?.purpose ?? "day_to_day";
    tripId = reviewTripId || existing?.trip_id || "";
  }

  function personalCents(): number | null {
    const normalized = personalAmount.trim().replace(",", ".");
    if (!/^\d+(?:\.\d{1,2})?$/.test(normalized)) return null;
    const cents = Math.round(Number(normalized) * 100);
    return Number.isSafeInteger(cents) ? cents : null;
  }

  async function saveAllocation() {
    const cents = personalCents();
    if (!selectedExpense || cents === null) return;
    busy = true;
    error = null;
    notice = null;
    try {
      await finance.allocateExpense(
        selectedExpense.id,
        cents,
        purpose,
        purpose === "trip" ? tripId : null,
      );
      notice = "Spending context saved; personal cost and shared receivable were rebuilt from the journal.";
      await onchanged?.();
      if (selectedPlan) await loadTrip(selectedPlan);
    } catch (cause) {
      error = cause instanceof Error ? cause.message : String(cause);
    } finally {
      busy = false;
    }
  }

  async function saveReimbursement() {
    if (!selectedInflow || !selectedTarget) return;
    busy = true;
    error = null;
    notice = null;
    try {
      await finance.linkReimbursement(selectedInflow.id, selectedTarget.candidate_id);
      notice = "Repayment linked to the shared receivable; it no longer counts as income.";
      inflowId = "";
      await load();
      await onchanged?.();
      if (selectedPlan) await loadTrip(selectedPlan);
    } catch (cause) {
      error = cause instanceof Error ? cause.message : String(cause);
    } finally {
      busy = false;
    }
  }

  onMount(() => void load());
</script>

<section class="context-review">
  <div class="heading">
    <div>
      <h2>Review spending</h2>
      <p>Start with a trip, then review your share transaction by transaction. Category and purpose remain separate.</p>
    </div>
    <label class="trip-picker">Trip
      <select value={reviewTripId} onchange={(event) => void chooseTrip(event)}>
        <option value="">No trip selected</option>
        {#each plans as plan (plan.id)}
          <option value={plan.id}>{plan.title} · {plan.date_start}–{plan.date_end}</option>
        {/each}
      </select>
    </label>
  </div>

  {#if error}<p class="error"><Icon name="alert" size={14} /> {error}</p>{/if}
  {#if notice}<p class="notice">{notice}</p>{/if}

  {#if selectedPlan}
    <section class="trip-overview">
      <div class="trip-title">
        <div><strong>{selectedPlan.title}</strong><span>{selectedPlan.date_start}–{selectedPlan.date_end}</span></div>
        {#if tripBusy}<span>Loading trip spending…</span>{:else}<span>{tripCandidates.length} confirmed expenses in this date window</span>{/if}
      </div>
      <div class="trip-totals" aria-label="Trip spending totals">
        <div><span>Your reviewed cost</span><strong>{money(tripPersonalCents, "EUR")}</strong></div>
        <div><span>Gross paid</span><strong>{money(tripGrossCents, "EUR")}</strong></div>
        <div><span>Outstanding</span><strong>{money(tripOutstandingCents, "EUR")}</strong></div>
        <div><span>Assigned</span><strong>{tripRows.length} / {tripCandidates.length}</strong></div>
      </div>
      <div class="trip-transactions">
        {#each tripCandidates as candidate (candidate.id)}
          {@const existing = (tripProjection?.transactions ?? []).find((row) => row.source_id === candidate.fingerprint)}
          <article class:assigned={existing?.trip_id === reviewTripId} class:selected={candidate.id === expenseId}>
            <time>{candidate.booked_at}</time>
            <strong>{candidate.description}</strong>
            <span>{money(-candidate.amount_cents, candidate.currency)}</span>
            <span class="review-state">
              {existing?.trip_id === reviewTripId ? `${money(existing.amount_cents, candidate.currency)} yours` : "Not assigned"}
            </span>
            <button type="button" onclick={() => selectExpense(candidate.id)}>{candidate.id === expenseId ? "Selected" : "Review"}</button>
          </article>
        {/each}
        {#if tripCandidates.length === 0 && !tripBusy}<p class="empty">No confirmed expenses fall inside this trip’s dates.</p>{/if}
      </div>
    </section>
  {/if}

  <div class="review-grid">
    <form class="allocation" onsubmit={(event) => { event.preventDefault(); void saveAllocation(); }}>
      <div class="form-heading">
        <div><h3>Review one transaction</h3><p>{selectedPlan ? `Assign it to ${selectedPlan.title} and record your share.` : "Add purpose or split a non-trip expense."}</p></div>
      </div>
      {#if !selectedPlan}
        <label>Confirmed expense
          <select value={expenseId} onchange={chooseExpense}>
            <option value="">Select an expense</option>
            {#each expenses as expense (expense.id)}
              <option value={expense.id}>{expense.booked_at} · {expense.description} · {money(-expense.amount_cents, expense.currency)}</option>
            {/each}
          </select>
        </label>
      {:else if !selectedExpense}
        <p class="empty selection-hint">Choose Review beside a transaction above.</p>
      {:else}
        <div class="selected-transaction"><time>{selectedExpense.booked_at}</time><strong>{selectedExpense.description}</strong><span>{money(-selectedExpense.amount_cents, selectedExpense.currency)}</span></div>
      {/if}
      <div class="fields">
        <label>Your share
          <input inputmode="decimal" bind:value={personalAmount} placeholder="0,00" />
        </label>
        {#if selectedPlan}
          <label>Purpose<input value="Trip" disabled /></label>
        {:else}
          <label>Purpose
            <select bind:value={purpose}>
              <option value="day_to_day">Day to day</option>
              <option value="trip">Trip</option>
              <option value="work">Work</option>
              <option value="housing">Housing</option>
              <option value="other">Other</option>
            </select>
          </label>
        {/if}
      </div>
      {#if purpose === "trip" && !selectedPlan}
        <label>Trips plan
          <select bind:value={tripId}>
            <option value="">Select a plan</option>
            {#each plans as plan (plan.id)}
              <option value={plan.id}>{plan.title} · {plan.date_start}–{plan.date_end}</option>
            {/each}
          </select>
        </label>
      {/if}
      {#if selectedExpense && personalCents() !== null}
        <p class="calculation">
          {money(-selectedExpense.amount_cents, selectedExpense.currency)} paid =
          {money(personalCents() ?? 0, selectedExpense.currency)} yours +
          {money(Math.max(0, -selectedExpense.amount_cents - (personalCents() ?? 0)), selectedExpense.currency)} receivable
        </p>
      {/if}
      <button disabled={busy || !selectedExpense || personalCents() === null || (purpose === "trip" && !tripId)}>
        Save reviewed split
      </button>
    </form>

    <aside class="shared-ledger">
      <h3>{selectedPlan ? "Trip receivables" : "Shared-expense ledger"}</h3>
      {#if visibleSharedExpenses.length}
        {#each visibleSharedExpenses as expense (expense.source_id)}
          <article>
            <div><time>{expense.date}</time><strong>{expense.description}</strong></div>
            <span>{money(expense.personal_cents, expense.currency)} yours</span>
            <strong class:settled={expense.outstanding_cents === 0}>{expense.outstanding_cents === 0 ? "Settled" : `${money(expense.outstanding_cents, expense.currency)} open`}</strong>
          </article>
        {/each}
      {:else}
        <p class="empty">No shared expenses here yet.</p>
      {/if}
      <details>
        <summary>Link a repayment</summary>
        <form class="repayment" onsubmit={(event) => { event.preventDefault(); void saveReimbursement(); }}>
          <label>Confirmed inflow
            <select bind:value={inflowId}>
              <option value="">Select an inflow</option>
              {#each inflows as inflow (inflow.id)}
                <option value={inflow.id}>{inflow.booked_at} · {inflow.description} · {money(inflow.amount_cents, inflow.currency)}</option>
              {/each}
            </select>
          </label>
          <label>Shared expense
            <select bind:value={reimbursementTarget}>
              <option value="">Select an outstanding receivable</option>
              {#each visibleSharedExpenses.filter((expense) => expense.outstanding_cents > 0) as expense (expense.source_id)}
                <option value={expense.candidate_id}>{expense.date} · {expense.description} · {money(expense.outstanding_cents, expense.currency)} open</option>
              {/each}
            </select>
          </label>
          {#if selectedInflow && selectedTarget && selectedInflow.amount_cents > selectedTarget.outstanding_cents}
            <p class="error">This inflow is larger than the outstanding receivable.</p>
          {/if}
          <button disabled={busy || !selectedInflow || !selectedTarget || selectedInflow.amount_cents > selectedTarget.outstanding_cents}>Link as repayment</button>
        </form>
      </details>
    </aside>
  </div>
</section>

<style>
  .context-review { border: 1px solid var(--border, #333); border-radius: 8px; padding: .9rem; }
  .heading { display: flex; align-items: end; justify-content: space-between; gap: 1rem; }
  .heading h2, h3 { margin: 0; }
  .heading h2 { font-size: .95rem; }
  .heading p, .form-heading p, .calculation, .empty { margin: .2rem 0 0; color: var(--muted, #888); font-size: .7rem; }
  .trip-picker { width: min(30rem, 48%); }
  label { display: grid; gap: .25rem; color: var(--muted, #888); font-size: .68rem; }
  input, select, button { min-width: 0; border: 1px solid var(--border, #333); border-radius: 6px; padding: .42rem .55rem; font: inherit; font-size: .74rem; background: transparent; color: inherit; }
  button { justify-self: start; cursor: pointer; }
  button:disabled, input:disabled { opacity: .45; cursor: default; }
  .trip-overview { margin-top: .8rem; border-top: 1px solid var(--border, #333); }
  .trip-title { display: flex; align-items: center; justify-content: space-between; gap: 1rem; padding: .65rem 0; font-size: .72rem; }
  .trip-title > div { display: flex; align-items: baseline; gap: .7rem; }
  .trip-title span { color: var(--muted, #888); }
  .trip-totals { display: grid; grid-template-columns: repeat(4, minmax(0, 1fr)); border: 1px solid var(--border, #333); border-radius: 7px; }
  .trip-totals div { display: grid; gap: .25rem; padding: .6rem .7rem; border-left: 1px solid var(--border, #333); }
  .trip-totals div:first-child { border-left: 0; }
  .trip-totals span { color: var(--muted, #888); font-size: .65rem; }
  .trip-totals strong { font-size: .92rem; font-variant-numeric: tabular-nums; }
  .trip-transactions { margin-top: .45rem; max-height: 17rem; overflow: auto; }
  .trip-transactions article { display: grid; grid-template-columns: 5.5rem minmax(12rem, 1fr) 7rem 8rem 4rem; align-items: center; gap: .65rem; padding: .45rem .2rem; border-bottom: 1px solid var(--border, #333); font-size: .7rem; }
  .trip-transactions article.selected { background: color-mix(in srgb, var(--primary) 8%, transparent); }
  .trip-transactions article.assigned .review-state { color: var(--primary); }
  .trip-transactions article > span { text-align: right; font-variant-numeric: tabular-nums; }
  .review-state { color: var(--muted, #888); }
  .trip-transactions button { justify-self: end; padding: .3rem .5rem; }
  .review-grid { display: grid; grid-template-columns: minmax(0, 1.15fr) minmax(19rem, .85fr); gap: .75rem; margin-top: .8rem; }
  .allocation, .shared-ledger { min-width: 0; border: 1px solid var(--border, #333); border-radius: 7px; padding: .75rem; }
  .allocation { display: grid; align-content: start; gap: .65rem; }
  h3 { font-size: .78rem; }
  .fields { display: grid; grid-template-columns: 1fr 1fr; gap: .6rem; }
  .selection-hint { min-height: 2.2rem; display: flex; align-items: center; }
  .selected-transaction { display: grid; grid-template-columns: auto minmax(8rem, 1fr) auto; gap: .7rem; align-items: center; padding: .5rem .6rem; background: color-mix(in srgb, currentColor 4%, transparent); font-size: .72rem; }
  .selected-transaction time { color: var(--muted, #888); }
  .selected-transaction span { font-variant-numeric: tabular-nums; }
  .shared-ledger > h3 { margin-bottom: .35rem; }
  .shared-ledger > article { display: grid; grid-template-columns: minmax(9rem, 1fr) auto auto; align-items: center; gap: .7rem; padding: .45rem 0; border-bottom: 1px solid var(--border, #333); font-size: .68rem; }
  .shared-ledger > article > div { display: grid; grid-template-columns: auto minmax(6rem, 1fr); gap: .45rem; }
  .shared-ledger > article span { color: var(--muted, #888); }
  .shared-ledger > article > strong { text-align: right; font-variant-numeric: tabular-nums; }
  .shared-ledger > article > strong.settled { color: var(--primary); }
  details { margin-top: .7rem; }
  summary { cursor: pointer; color: var(--muted, #888); font-size: .7rem; }
  .repayment { display: grid; gap: .55rem; margin-top: .6rem; }
  .error { color: var(--danger, #b44); font-size: .72rem; }
  .notice { color: var(--primary); font-size: .72rem; }
  @media (max-width: 800px) {
    .heading { align-items: stretch; flex-direction: column; }
    .trip-picker { width: auto; }
    .trip-totals { grid-template-columns: 1fr 1fr; }
    .trip-totals div:nth-child(3) { border-left: 0; border-top: 1px solid var(--border, #333); }
    .trip-totals div:nth-child(4) { border-top: 1px solid var(--border, #333); }
    .trip-transactions article { grid-template-columns: auto 1fr auto; }
    .trip-transactions article .review-state { grid-column: 2; text-align: left; }
    .review-grid, .fields { grid-template-columns: 1fr; }
  }
</style>
