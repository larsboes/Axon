<script lang="ts">
  /**
   * One probable duplicate pair: both records side by side, why they were paired, what the
   * fields both carry say, the on-device model's advice when it had any, and two actions.
   * Nothing merges without the tap (capabilities/entities/src/duplicates.rs).
   */
  import Icon from '$lib/Icon.svelte';
  import type { DuplicateCandidate, DuplicateProfile } from '$lib/api';
  import { executeDistinct, executeMerge, keepSide, mergedName, settleCardAction, type CardStatus } from '../executor';

  let { card }: { card: DuplicateCandidate } = $props();

  let status = $state<CardStatus>({ kind: 'idle' });
  const keep = $derived(keepSide(card));
  const name = $derived(mergedName(card));

  const strengthLabel = { strong: 'Shared contact detail', name: 'Same name', partial: 'First name only' } as const;

  function lines(p: DuplicateProfile): string[] {
    const text = (v: unknown) => (Array.isArray(v) ? v.join(', ') : v == null ? '' : String(v));
    return [
      p.lives_in ? `lives in ${p.lives_in}` : '',
      text(p.company),
      text(p.relation),
      text(p.emails),
      text(p.phones),
      p.birthday ? `born ${p.birthday}` : '',
      `from ${p.sources.join(', ') || 'no source'}${p.has_note ? ' · has note' : ''}`,
    ].filter(Boolean);
  }

  async function act(run: () => ReturnType<typeof executeMerge>) {
    status = { kind: 'pending' };
    status = await settleCardAction(run);
  }
</script>

<div class="merge-card">
  <div class="card-header">
    <span class="title"><Icon name="users" size={14} /> {strengthLabel[card.strength]}</span>
    <span class="reason">{card.reasons.join(' · ')}</span>
  </div>

  <div class="sides">
    {#each [['a', card.a], ['b', card.b]] as const as [side, record] (side)}
      <div class="side" class:kept={keep === side}>
        <strong>{record.profile.name}</strong>
        {#each lines(record.profile) as line, i (i)}<span>{line}</span>{/each}
      </div>
    {/each}
  </div>

  {#if card.evidence.same.length || card.evidence.different.length}
    <p class="evidence">
      {#if card.evidence.same.length}Same: {card.evidence.same.join(', ')}.{/if}
      {#if card.evidence.different.length} Different: {card.evidence.different.join(', ')}.{/if}
    </p>
  {/if}
  {#if card.verdict}
    <p class="verdict">
      Local model: {card.verdict.same === true ? 'probably the same' : card.verdict.same === false ? 'probably not' : 'cannot tell'}
      — {card.verdict.why}
    </p>
  {/if}

  {#if status.kind === 'failed'}<p class="error-desc" role="alert">{status.message}</p>{/if}
  {#if status.kind === 'applied'}<p class="done">{status.message}</p>{/if}

  {#if status.kind !== 'applied'}
    <div class="card-footer">
      <button class="btn btn-soft btn-sm" disabled={status.kind === 'pending'} onclick={() => act(() => executeMerge(card))}>
        <Icon name="check" size={12} /> Merge as "{name}"
      </button>
      <button class="btn btn-outline btn-sm" disabled={status.kind === 'pending'} onclick={() => act(() => executeDistinct(card))}>
        Not the same
      </button>
    </div>
  {/if}
</div>

<style>
  .merge-card {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    margin-top: var(--space-2);
    padding: var(--space-3) var(--space-4);
    border: 1px solid var(--card-border);
    border-radius: var(--radius);
    background: var(--card-bg);
    box-shadow: var(--card-shadow);
  }

  .card-header {
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    justify-content: space-between;
    gap: var(--space-2);
  }

  .title {
    display: inline-flex;
    align-items: center;
    gap: 0.3rem;
    font-size: var(--text-xs);
    font-weight: 600;
  }

  .reason,
  .evidence,
  .verdict,
  .done {
    margin: 0;
    font-size: var(--text-2xs);
    color: var(--text-secondary);
  }

  .sides {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: var(--space-2);
  }

  .side {
    display: flex;
    flex-direction: column;
    gap: 0.1rem;
    padding: var(--space-2);
    border: 1px solid var(--card-border);
    border-radius: var(--radius-sm);
    font-size: var(--text-2xs);
    color: var(--text-secondary);
    overflow-wrap: anywhere;
  }

  .side strong {
    font-size: var(--text-xs);
    color: var(--text-primary);
  }

  .side.kept {
    border-color: var(--primary);
  }

  .error-desc {
    margin: 0;
    font-size: var(--text-2xs);
    color: var(--danger);
  }

  .card-footer {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
  }
</style>
