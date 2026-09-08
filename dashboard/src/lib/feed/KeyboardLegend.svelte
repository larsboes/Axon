<script lang="ts">
  // The shortcuts, said out loud. A keyboard surface nobody can see is a
  // keyboard surface nobody uses; `?` hides it once it has been read.
  let { open = true, ontoggle }: { open?: boolean; ontoggle?: () => void } = $props();

  const KEYS: { key: string; action: string }[] = [
    { key: "j", action: "down" },
    { key: "k", action: "up" },
    { key: "s", action: "keep" },
    { key: "d", action: "dismiss" },
    { key: "o", action: "open" },
    { key: "e", action: "explain" },
    { key: "u", action: "undo" },
  ];
</script>

<p class="legend">
  {#if open}
    {#each KEYS as entry (entry.key)}
      <span class="pair"><kbd>{entry.key}</kbd> {entry.action}</span>
    {/each}
  {/if}
  <button class="toggle" onclick={ontoggle} aria-expanded={open}>
    <kbd>?</kbd>
    {open ? "hide" : "shortcuts"}
  </button>
</p>

<style>
  .legend {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0.5rem 0.75rem;
    margin: 0 0 0.5rem;
    color: var(--text-tertiary);
    font-size: var(--text-xs);
  }

  .pair {
    display: inline-flex;
    align-items: center;
    gap: 0.25rem;
  }

  .toggle {
    display: inline-flex;
    align-items: center;
    gap: 0.25rem;
    padding: 0;
    border: 0;
    background: none;
    color: inherit;
    font: inherit;
    cursor: pointer;
  }

  .toggle:hover {
    color: var(--text-secondary);
  }

  kbd {
    min-width: 1.25rem;
    padding: 0.1rem 0.25rem;
    border: 1px solid var(--card-border);
    border-bottom-color: var(--card-border-hover);
    border-radius: 3px;
    background: var(--surface);
    color: var(--text-secondary);
    font: 600 0.5625rem var(--font-mono);
    text-align: center;
  }
</style>
