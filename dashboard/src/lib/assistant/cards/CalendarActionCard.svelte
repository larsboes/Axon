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
    background: var(--card-bg);
    border: 1px solid var(--card-border);
    border-radius: var(--radius);
    padding: var(--space-3) var(--space-4);
    box-shadow: var(--card-shadow);
    margin-top: var(--space-2);
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
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
    font-size: var(--text-xs);
    font-weight: 600;
    color: var(--text-primary);
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
    gap: 0.25rem;
    font-size: var(--text-2xs);
    font-weight: 600;
    color: var(--success);
    background: var(--success-soft);
    padding: 0.1rem 0.45rem;
    border-radius: var(--radius-sm);
  }

  .slot-meta {
    display: flex;
    gap: var(--space-3);
    font-size: var(--text-xs);
    color: var(--text-secondary);
  }

  .date {
    font-weight: 600;
  }

  .error-desc {
    font-size: var(--text-2xs);
    color: var(--danger);
    margin: 0;
  }

  .card-footer {
    display: flex;
    justify-content: flex-end;
    border-top: 1px solid var(--card-border);
    padding-top: var(--space-2);
  }

  .action-btn {
    font-size: var(--text-xs);
    padding: 0.3rem 0.65rem;
    display: inline-flex;
    align-items: center;
    gap: 0.35rem;
    border-radius: var(--radius-sm);
  }

  .btn-applied {
    background: var(--success-soft);
    color: var(--success);
  }
</style>
