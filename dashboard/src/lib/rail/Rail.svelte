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
     * once expanded sections outgrow the viewport. `4.75rem` clears the sticky header
     * (bar + nav row). */
    position: sticky;
    top: 4.75rem;
    max-height: calc(100vh - 6rem);
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
