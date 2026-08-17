<script lang="ts">
  import type { Snippet } from "svelte";

  let {
    title,
    count = null,
    children,
  }: {
    title: string;
    /** Sum across the group's sections. `null` or `0` drops the badge — a group heading
     * is not a list, so an empty one says nothing rather than showing a zero. */
    count?: number | null;
    children: Snippet;
  } = $props();
</script>

<div class="group">
  <h2>
    {title}
    {#if count}<span class="total">{count}</span>{/if}
  </h2>
  {@render children()}
</div>

<style>
  .group {
    padding: 0.35rem 0.75rem 0.25rem;
    border: 1px solid var(--card-border);
    border-radius: var(--radius);
    background-color: var(--card-bg);
  }

  h2 {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    margin: 0;
    padding: 0.5rem 0.25rem 0.4rem;
    color: var(--text-tertiary);
    font-size: 0.625rem;
    font-weight: 700;
    letter-spacing: 0.08em;
    text-transform: uppercase;
  }

  .total {
    padding: 0.05rem 0.35rem;
    border-radius: var(--radius-sm);
    background-color: var(--primary);
    color: var(--text-inverse);
    font-size: 0.625rem;
    font-variant-numeric: tabular-nums;
  }
</style>
