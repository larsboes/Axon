<script lang="ts">
  import Icon from '$lib/Icon.svelte';
  import { trips, type TripPlan } from '$lib/api';
  import { settleCardAction, type CardStatus } from '../executor';
  import type { ActionResult, JourneyOptionCardData } from '../types';

  let {
    card,
    onApply,
  }: {
    card: JourneyOptionCardData;
    onApply: (card: JourneyOptionCardData, plan: TripPlan | null) => Promise<ActionResult>;
  } = $props();

  const journey = $derived(card.journey);
  const first = $derived(journey.legs[0]);
  const last = $derived(journey.legs[journey.legs.length - 1]);
  const trains = $derived(
    [...new Set(journey.legs.map((leg) => leg.train_name).filter(Boolean))].join(' · '),
  );

  // Only draft plans are offered, and the operator picks one. No default plan, no plan
  // created on the side: see executeJourneyPin.
  let drafts = $state<TripPlan[] | null>(null);
  let planId = $state('');
  let loadError = $state<string | null>(null);
  let status = $state<CardStatus>({ kind: 'idle' });

  async function loadDrafts() {
    loadError = null;
    try {
      drafts = (await trips.list()).filter((plan) => plan.status === 'draft');
    } catch (err) {
      loadError = `trips did not answer: ${err instanceof Error ? err.message : String(err)}`;
    }
  }

  async function handlePin() {
    const plan = drafts?.find((p) => p.id === planId) ?? null;
    status = { kind: 'pending' };
    status = await settleCardAction(() => onApply(card, plan));
  }

  const hhmm = (iso: string | undefined) =>
    iso ? new Date(iso).toLocaleTimeString('en-GB', { hour: '2-digit', minute: '2-digit' }) : '–';

  function formatDuration(mins: number): string {
    const h = Math.floor(mins / 60);
    const m = mins % 60;
    return h > 0 ? `${h}h ${m}m` : `${m}m`;
  }
</script>

<div class="journey-card">
  <div class="card-header">
    <div class="mode-badge">
      <Icon name="train" size={14} />
      <span class="mode-text">{trains || 'Train'}</span>
    </div>
    <span class="price mono">
      {journey.total_price === null ? 'price unknown' : `${journey.total_price.toFixed(2)} €`}
    </span>
  </div>

  <div class="route-line">
    <div class="point">
      <span class="time mono">{hhmm(first?.departure_time)}</span>
      <span class="station">{journey.start_station.name}</span>
    </div>
    <div class="duration-middle">
      <span class="duration-text">{formatDuration(journey.total_duration_minutes)}</span>
      <div class="track-line">
        <span class="dot"></span>
        <span class="line"></span>
        {#if journey.legs.length > 1}
          <span class="transfer-pill">{journey.legs.length - 1} chg</span>
        {/if}
        <span class="dot end"></span>
      </div>
    </div>
    <div class="point end">
      <span class="time mono">{hhmm(last?.arrival_time)}</span>
      <span class="station">{journey.end_station.name}</span>
    </div>
  </div>

  {#if loadError}
    <p class="error-desc" role="alert">{loadError}</p>
  {/if}
  {#if status.kind === 'failed'}
    <p class="error-desc" role="alert">{status.message}</p>
  {/if}

  <div class="card-footer">
    {#if status.kind === 'applied'}
      <span class="applied-note">
        <Icon name="check" size={12} />
        {status.message}
      </span>
    {:else if drafts === null}
      <button class="btn btn-soft btn-sm action-btn" onclick={loadDrafts}>
        <Icon name="plus" size={12} />
        <span>Pin to a draft plan…</span>
      </button>
    {:else if drafts.length === 0}
      <span class="hint">No draft plan to pin into. Create one on Travel.</span>
    {:else}
      <select class="plan-select" bind:value={planId} aria-label="Draft plan">
        <option value="" disabled>Choose a draft plan</option>
        {#each drafts as plan (plan.id)}
          <option value={plan.id}>{plan.title}</option>
        {/each}
      </select>
      <button
        class="btn btn-soft btn-sm action-btn"
        onclick={handlePin}
        disabled={!planId || status.kind === 'pending'}
      >
        <Icon name="plus" size={12} />
        <span>{status.kind === 'pending' ? 'Pinning…' : 'Pin'}</span>
      </button>
    {/if}
  </div>
</div>

<style>
  .journey-card {
    background: var(--card-bg, #1a1a1a);
    border: 1px solid var(--card-border, #333);
    border-radius: var(--radius-md, 8px);
    padding: 0.75rem 0.85rem;
    margin-top: 0.5rem;
    box-shadow: var(--card-shadow, 0 2px 4px rgba(0, 0, 0, 0.1));
    display: flex;
    flex-direction: column;
    gap: 0.6rem;
  }

  .card-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
  }

  .mode-badge {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    font-size: var(--text-xs, 0.75rem);
    font-weight: 600;
    color: var(--primary, #06b6d4);
  }

  .price {
    font-size: var(--text-xs, 0.75rem);
    font-weight: 700;
    color: var(--text-primary, #f4f4f5);
  }

  .route-line {
    display: grid;
    grid-template-columns: auto 1fr auto;
    align-items: center;
    gap: 0.75rem;
  }

  .point {
    display: flex;
    flex-direction: column;
    gap: 0.1rem;
  }

  .point.end {
    align-items: flex-end;
  }

  .time {
    font-size: 0.85rem;
    font-weight: 700;
    color: var(--text-primary, #fff);
  }

  .station {
    font-size: 0.7rem;
    color: var(--text-tertiary, #a1a1aa);
    max-width: 90px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .duration-middle {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 0.2rem;
  }

  .duration-text {
    font-size: 0.65rem;
    color: var(--text-tertiary, #a1a1aa);
  }

  .track-line {
    position: relative;
    width: 100%;
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .line {
    position: absolute;
    left: 4px;
    right: 4px;
    height: 2px;
    background: var(--card-border, #444);
    z-index: 1;
  }

  .dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--primary, #06b6d4);
    z-index: 2;
    position: absolute;
    left: 0;
  }

  .dot.end {
    left: auto;
    right: 0;
  }

  .transfer-pill {
    position: relative;
    z-index: 3;
    background: var(--page-bg, #09090b);
    border: 1px solid var(--card-border, #333);
    padding: 0.05rem 0.3rem;
    border-radius: 4px;
    font-size: 0.6rem;
    color: var(--text-secondary, #d4d4d8);
  }

  .card-footer {
    display: flex;
    gap: 0.5rem;
    justify-content: flex-end;
    align-items: center;
    border-top: 1px solid var(--card-border, #2a2a2a);
    padding-top: 0.5rem;
  }

  .action-btn {
    font-size: 0.7rem;
    padding: 0.25rem 0.6rem;
    border-radius: var(--radius-sm, 4px);
    display: inline-flex;
    align-items: center;
    gap: 0.3rem;
  }

  .applied-note {
    display: inline-flex;
    align-items: center;
    gap: 0.3rem;
    font-size: 0.7rem;
    color: #10b981;
  }

  .hint {
    font-size: 0.7rem;
    color: var(--text-tertiary, #a1a1aa);
  }

  .error-desc {
    font-size: 0.7rem;
    color: #f87171;
    margin: 0;
  }

  .plan-select {
    flex: 1;
    min-width: 0;
    font-size: 0.7rem;
    background: var(--page-bg, #09090b);
    color: var(--text-primary, #fff);
    border: 1px solid var(--card-border, #333);
    border-radius: var(--radius-sm, 4px);
    padding: 0.2rem 0.35rem;
  }
</style>
