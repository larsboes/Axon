<script lang="ts">
  import type { Snippet } from "svelte";

  /**
   * The header 14 routes share.
   *
   * `badge` keeps its name although it is no longer styled as one — renaming it would
   * touch all 14 consumers while five other passes are editing them, and the restyle is
   * what was actually wrong. It is the section this page belongs to, in sentence case:
   * a tracked-out all-caps eyebrow above every heading is template chrome, and it read
   * that way on every route at once.
   *
   * The entrance animation is gone. A slide-up on fourteen pages is not a decision, and
   * it made the page's own header invisible for the first 300ms — which is exactly what
   * the 2026-09-03 baseline captures of /travel and /feed show: a band of empty space
   * where the title should be.
   */
  let {
    badge,
    title,
    desc,
    context,
    actions,
  }: {
    badge: string;
    title: string;
    desc?: string;
    /** A fact about this page — a count, a date range, a state. Renders nothing when absent. */
    context?: Snippet;
    /** Page-level controls, aligned with the title. */
    actions?: Snippet;
  } = $props();
</script>

<header>
  <div class="lead">
    <div class="titles">
      <p class="badge">{badge}</p>
      <h1>{title}</h1>
    </div>
    {#if actions}<div class="actions">{@render actions()}</div>{/if}
  </div>
  {#if desc}<p class="desc">{desc}</p>{/if}
  {#if context}<div class="context">{@render context()}</div>{/if}
</header>

<style>
  header {
    margin-bottom: var(--space-6);
  }

  .lead {
    display: flex;
    align-items: flex-end;
    justify-content: space-between;
    gap: var(--space-6);
  }

  .titles {
    min-width: 0;
  }

  .badge {
    margin: 0;
    color: var(--primary);
    font-size: var(--text-2xs);
    font-weight: 600;
  }

  h1 {
    margin: var(--space-1) 0 0;
    font-size: var(--text-xl);
    font-weight: 650;
    line-height: var(--leading-tight);
    letter-spacing: var(--tracking-tight);
  }

  .actions {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex-shrink: 0;
  }

  .desc {
    margin: var(--space-2) 0 0;
    max-width: var(--measure);
    color: var(--text-secondary);
    font-size: var(--text-sm);
    line-height: var(--leading-normal);
  }

  .context {
    margin-top: var(--space-3);
    color: var(--text-tertiary);
    font-size: var(--text-xs);
  }

  @media (width < 38rem) {
    .lead {
      flex-direction: column;
      align-items: flex-start;
      gap: var(--space-3);
    }
  }
</style>
