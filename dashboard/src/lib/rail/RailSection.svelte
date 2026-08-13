<script lang="ts">
  import type { Snippet } from "svelte";
  import Icon from "$lib/Icon.svelte";

  let {
    label,
    count = null,
    open = $bindable(false),
    children,
  }: {
    label: string;
    /** Badge value. `null` drops the badge for sections that are not lists. `0` renders
     * as a muted dot rather than vanishing: a row that disappears when empty reads as a
     * broken rail, and you can no longer tell "nothing waiting" from "not loaded". */
    count?: number | null;
    open?: boolean;
    children: Snippet;
  } = $props();
</script>

<details class="section" class:quiet={count === 0} bind:open>
  <summary>
    <span class="chevron"><Icon name="chevron" size={12} /></span>
    <span class="label">{label}</span>
    {#if count !== null}
      <span class="count">{count === 0 ? "·" : count}</span>
    {/if}
  </summary>
  <div class="body">
    {@render children()}
  </div>
</details>

<style>
  .section {
    border-bottom: 1px solid var(--card-border);
  }

  .section:last-child {
    border-bottom: 0;
  }

  summary {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.6rem 0.25rem;
    list-style: none;
    cursor: pointer;
    color: var(--text-primary);
    font-size: 0.8125rem;
    font-weight: 500;
    border-radius: var(--radius-sm);
  }

  summary::-webkit-details-marker {
    display: none;
  }

  summary:hover {
    color: var(--primary);
  }

  summary:focus-visible {
    outline: 2px solid var(--primary);
    outline-offset: -2px;
  }

  .chevron {
    display: flex;
    color: var(--text-tertiary);
    transition: transform 0.15s ease;
  }

  [open] > summary .chevron {
    transform: rotate(90deg);
  }

  .label {
    flex: 1;
    min-width: 0;
  }

  .count {
    min-width: 1.25rem;
    padding: 0.05rem 0.35rem;
    border-radius: var(--radius-sm);
    background-color: var(--primary-soft);
    color: var(--primary);
    font-size: 0.6875rem;
    font-weight: 600;
    font-variant-numeric: tabular-nums;
    text-align: center;
  }

  /* Nothing waiting: the row stays, its weight does not. */
  .quiet > summary {
    color: var(--text-tertiary);
  }

  .quiet > summary .count {
    background-color: transparent;
    color: var(--text-tertiary);
  }

  .body {
    padding: 0.15rem 0.25rem 0.85rem;
  }
</style>
