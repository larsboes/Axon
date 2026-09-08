<script lang="ts">
  import type { Snippet } from "svelte";
  import Icon from "./Icon.svelte";

  /**
   * Loading, error and empty as one primitive, because every page had reinvented all
   * three — `.loading`, `.muted`, `.empty`, `.error`, `.notice`, `.queue-state`.
   *
   * The empty branch renders NOTHING unless a caller passes `empty`. PRD §8.1 is explicit
   * that a dashboard blank on a quiet day is working correctly and that a "0 items"
   * placeholder destroys the signal, so the empty state's default is silence and a filled
   * one has to be asked for. Loading is a state and not an empty state, and still shows.
   */
  let {
    state,
    message,
    onRetry,
    empty,
  }: {
    state: "loading" | "error" | "empty" | "ready";
    /** The loading line's words, or the error's. */
    message?: string;
    onRetry?: () => void;
    /** What to say when there is nothing. Absent means say nothing at all. */
    empty?: Snippet;
  } = $props();
</script>

{#if state === "loading"}
  <p class="line loading" aria-live="polite">
    <Icon name="loader" size={13} />
    {message ?? "Loading…"}
  </p>
{:else if state === "error"}
  <p class="line error" role="alert">
    <Icon name="alert" size={13} />
    <span>{message ?? "This could not be read."}</span>
    {#if onRetry}
      <button class="btn" type="button" onclick={onRetry}>Try again</button>
    {/if}
  </p>
{:else if state === "empty" && empty}
  <div class="line empty">{@render empty()}</div>
{/if}

<style>
  .line {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    margin: 0;
    padding: var(--space-4) 0;
    color: var(--text-tertiary);
    font-size: var(--text-sm);
  }

  .error {
    color: var(--warning-ink);
  }

  .empty {
    color: var(--text-secondary);
  }

  .error .btn {
    margin-left: auto;
  }
</style>
