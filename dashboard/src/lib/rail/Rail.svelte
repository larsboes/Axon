<script lang="ts">
  import type { Snippet } from "svelte";

  let {
    label,
    children,
  }: {
    /** Names the landmark for screen readers — "Calendar review and planning". */
    label: string;
    children: Snippet;
  } = $props();
</script>

<aside class="rail" aria-label={label}>
  {@render children()}
</aside>

<style>
  .rail {
    display: flex;
    flex-direction: column;
    gap: 1rem;
    min-width: 0;

    /* Sticky so triage stays reachable while the page scrolls; it scrolls on its own
     * once expanded sections outgrow the viewport. The offset follows --header-stack
     * (bar + nav row) rather than a hand-copied 4.75rem, so it still clears the header
     * on a phone, where the bar is 3.25rem and the old constant left it drifting under. */
    position: sticky;
    top: calc(var(--header-stack) + var(--space-3));
    max-height: calc(100vh - var(--header-stack) - var(--space-6));
    overflow-y: auto;
  }

  /* Below the rail breakpoint it stacks under the main column, where sticky would trap it. */
  @media (width < 64rem) {
    .rail {
      position: static;
      max-height: none;
      overflow-y: visible;
    }
  }
</style>
