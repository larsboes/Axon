<script lang="ts">
  import type { Snippet } from "svelte";

  /**
   * The row shell every inbox in this app sits on.
   *
   * Home's ladder, the feed inbox and the mail proposals had each grown their own row —
   * three grids, three paddings, three ideas of where the actions go. This is the one
   * shape: a mark, a body that may shrink to nothing, and actions that never do.
   *
   * It is deliberately never `role="option"`. Every real row holds a title link and one
   * or two buttons, and an option must not contain focusable descendants; the queue is a
   * list and the keyboard cursor moves real DOM focus onto the row instead.
   */
  let {
    id,
    role = "listitem",
    current = false,
    dimmed = false,
    tone = "none",
    href,
    mark,
    children,
    actions,
    meta,
  }: {
    /** Stable DOM id, so a cursor can find the element to focus. */
    id?: string;
    role?: "listitem" | "article";
    /** The keyboard cursor is on this row. Announced as `aria-current`, not selected. */
    current?: boolean;
    /** The row is spent — decided, expired, superseded — and stays in place greyed.
     *  Visual only: the content is still read, because a reader who cannot see the
     *  opacity must still be told what the row says. */
    dimmed?: boolean;
    /** The band's spine segment. `none` draws no spine at all. */
    tone?: "alarm" | "now" | "owed" | "offer" | "none";
    /** The row's primary destination. Renders a stretched hit area behind the content,
     *  so the whole row is clickable without wrapping the nested links. */
    href?: string;
    mark?: Snippet;
    children: Snippet;
    actions?: Snippet;
    meta?: Snippet;
  } = $props();
</script>

<svelte:element
  this={role === "listitem" ? "li" : "article"}
  {id}
  {role}
  class="row tone-{tone}"
  class:current
  class:dimmed
  class:linked={href !== undefined}
  tabindex="-1"
  aria-current={current ? "true" : undefined}
>
  {#if href}
    <!-- aria-hidden and untabbable: the title inside `children` is the accessible link,
         and two links to the same place would be read twice. -->
    <a class="hit" {href} tabindex="-1" aria-hidden="true">&nbsp;</a>
  {/if}

  {#if mark}<span class="mark">{@render mark()}</span>{/if}

  <div class="body">
    {@render children()}
    {#if meta}<div class="meta">{@render meta()}</div>{/if}
  </div>

  {#if actions}<div class="actions">{@render actions()}</div>{/if}
</svelte:element>

<style>
  .row.dimmed {
    opacity: 0.55;
  }

  .row {
    position: relative;
    display: grid;
    grid-template-columns: auto minmax(0, 1fr) auto;
    align-items: start;
    gap: var(--space-4);
    padding: var(--space-4) var(--space-5) var(--space-4) var(--space-4);
    border-bottom: 1px solid var(--card-border);
    list-style: none;
    transition: background-color var(--motion-fast) ease;
  }

  .row:last-child {
    border-bottom: 0;
  }

  .row:hover {
    background-color: var(--surface);
  }

  /* The spine. Two pixels of the band's tone against the row's leading edge — the one
   * place rank is visible without reading a number. Suppressed at `tone="none"`. */
  .row::before {
    content: "";
    position: absolute;
    inset: 0.55rem auto 0.55rem 0;
    width: 3px;
    border-radius: 3px;
    background-color: transparent;
  }

  .tone-alarm::before { background-color: var(--band-alarm); }
  .tone-now::before { background-color: var(--band-now); }
  .tone-owed::before { background-color: var(--band-owed); }
  .tone-offer::before { background-color: var(--band-offer); }

  /* The cursor is a HAIRLINE and a tint, not a filled block. A solid panel behind the
     selected row competed with the row's own content and read heavier than the alarm
     band above it, which inverted the ranking the ladder exists to show. */
  .row.current {
    background-color: color-mix(in srgb, var(--primary) 6%, transparent);
    box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--primary) 22%, transparent);
    border-radius: var(--radius-md);
  }

  .row:focus-visible {
    outline: none;
    box-shadow: var(--focus-ring);
  }

  .hit {
    position: absolute;
    inset: 0;
    z-index: 0;
    overflow: hidden;
    text-indent: -999em;
  }

  .mark,
  .body,
  .actions {
    position: relative;
    z-index: 1;
  }

  .mark {
    display: grid;
    place-items: center;
    height: 1.75rem;
    width: 1.75rem;
    border-radius: var(--radius-sm);
    background-color: var(--surface);
    color: var(--text-secondary);
  }

  .body {
    display: flex;
    min-width: 0;
    flex-direction: column;
    gap: var(--space-1);
  }

  .meta {
    margin-top: var(--space-1);
  }

  .actions {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  /* Below the tablet step the actions wrap under the body rather than squeezing the
     title into three words. The mark keeps its column so the spine stays readable. */
  @media (width < 48rem) {
    .row {
      grid-template-columns: auto minmax(0, 1fr);
      row-gap: var(--space-3);
    }

    .actions {
      grid-column: 2;
      flex-wrap: wrap;
    }
  }
</style>
