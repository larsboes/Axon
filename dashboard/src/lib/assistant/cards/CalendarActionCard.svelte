<script lang="ts">
  import Icon from '$lib/Icon.svelte';
  import { settleCardAction, type CardStatus } from '../executor';
  import type { ActionResult, CalendarSlotCardData } from '../types';

  let {
    card,
    onApply,
  }: { card: CalendarSlotCardData; onApply: (card: CalendarSlotCardData) => Promise<ActionResult> } = $props();

  let status = $state<CardStatus>({ kind: 'idle' });

  // Applied only after the calendar answered ok; a failure keeps the button and shows why.
  async function handleApply() {
    status = { kind: 'pending' };
    status = await settleCardAction(() => onApply(card));
  }

  const day = $derived(card.startsAt.slice(0, 10));
  const endDay = $derived(card.endsAt.slice(0, 10));
  const span = $derived(
    `${card.startsAt.slice(11, 16)} – ${endDay === day ? '' : `${endDay} `}${card.endsAt.slice(11, 16)}`,
  );
</script>

<div class="calendar-card">
  <div class="card-header">
    <div class="title-wrap">
      <Icon name="calendar" size={14} />
      <span class="slot-title">{card.title}</span>
    </div>
    <span class="free-badge">
      <Icon name="check" size={11} />
      No overlapping entry
    </span>
  </div>

  <div class="slot-meta">
    <span class="date mono">{day}</span>
    <span class="times mono">{span}</span>
  </div>

  {#if status.kind === 'failed'}
    <p class="error-desc" role="alert">{status.message}</p>
  {/if}

  <div class="card-footer">
    <button
      class="btn btn-soft btn-sm action-btn"
      class:btn-applied={status.kind === 'applied'}
      onclick={handleApply}
      disabled={status.kind === 'applied' || status.kind === 'pending'}
    >
      <Icon name={status.kind === 'applied' ? 'check' : 'plus'} size={12} />
      <span>
        {#if status.kind === 'applied'}
          Created in calendar
        {:else if status.kind === 'pending'}
          Creating…
        {:else if status.kind === 'failed'}
          Retry
        {:else}
          Add to calendar
        {/if}
      </span>
    </button>
  </div>
</div>

<style>
  .calendar-card {
    background: var(--card-bg, #1a1a1a);
    border: 1px solid var(--card-border, #333);
    border-radius: var(--radius-md, 8px);
    padding: 0.75rem 0.85rem;
    margin-top: 0.5rem;
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }

  .card-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
  }

  .title-wrap {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    font-size: var(--text-xs, 0.75rem);
    font-weight: 600;
    color: var(--text-primary, #fff);
  }

  .slot-title {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 200px;
  }

  .free-badge {
    display: inline-flex;
    align-items: center;
    gap: 0.2rem;
    font-size: 0.65rem;
    color: #10b981;
    background: rgba(16, 185, 129, 0.12);
    padding: 0.1rem 0.35rem;
    border-radius: 4px;
  }

  .slot-meta {
    display: flex;
    gap: 0.6rem;
    font-size: 0.75rem;
    color: var(--text-secondary, #d4d4d8);
  }

  .date {
    font-weight: 600;
  }

  .error-desc {
    font-size: 0.7rem;
    color: #f87171;
    margin: 0;
  }

  .card-footer {
    display: flex;
    justify-content: flex-end;
    border-top: 1px solid var(--card-border, #2a2a2a);
    padding-top: 0.4rem;
  }

  .action-btn {
    font-size: 0.7rem;
    padding: 0.25rem 0.6rem;
    display: inline-flex;
    align-items: center;
    gap: 0.3rem;
  }

  .btn-applied {
    background: rgba(16, 185, 129, 0.15);
    color: #10b981;
    border-color: rgba(16, 185, 129, 0.3);
  }
</style>
