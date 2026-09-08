<script lang="ts" module>
  export interface TabItem {
    id: string;
    label: string;
    count?: number;
    /** An Icon name. Optional: a tab bar of four words needs no glyphs. */
    icon?: string;
    /** A real destination. Present renders a link; absent renders a tab button. */
    href?: string;
  }
</script>

<script lang="ts">
  import Icon from "./Icon.svelte";

  /**
   * The tab strip Finance and Travel each hand-rolled.
   *
   * Two forms, one look. `href` present means these are pages and the browser owns the
   * state, so they are links with `aria-current="page"`. `href` absent means they are
   * views of one page, so they are `role="tab"` buttons with `aria-selected`. Rendering
   * a button as a tab when it is really a link is the mistake that breaks middle-click.
   */
  let {
    items,
    value = $bindable(),
    label,
    onchange,
  }: {
    items: TabItem[];
    /** The selected id, for the button form. Bindable. */
    value?: string;
    /** Names the group for a screen reader — "Finance views", not "Tabs". */
    label: string;
    onchange?: (id: string) => void;
  } = $props();

  const linked = $derived(items.every((item) => item.href !== undefined));

  function select(item: TabItem): void {
    value = item.id;
    onchange?.(item.id);
  }
</script>

{#if linked}
  <nav class="tabs" aria-label={label}>
    {#each items as item (item.id)}
      <a
        class="tab"
        class:active={value === item.id}
        href={item.href}
        aria-current={value === item.id ? "page" : undefined}
      >
        {#if item.icon}<Icon name={item.icon as never} size={13} />{/if}
        {item.label}
        {#if item.count !== undefined}<span class="count mono">{item.count}</span>{/if}
      </a>
    {/each}
  </nav>
{:else}
  <div class="tabs" role="tablist" aria-label={label}>
    {#each items as item (item.id)}
      <button
        class="tab"
        class:active={value === item.id}
        type="button"
        role="tab"
        aria-selected={value === item.id}
        onclick={() => select(item)}
      >
        {#if item.icon}<Icon name={item.icon as never} size={13} />{/if}
        {item.label}
        {#if item.count !== undefined}<span class="count mono">{item.count}</span>{/if}
      </button>
    {/each}
  </div>
{/if}

<style>
  .tabs {
    display: flex;
    gap: var(--space-1);
    margin-bottom: var(--space-6);
    border-bottom: 1px solid var(--rule);
  }

  .tab {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-3) var(--space-4);
    border: 0;
    border-bottom: 2px solid transparent;
    margin-bottom: -1px;
    background: none;
    color: var(--text-secondary);
    font: inherit;
    font-size: var(--text-sm);
    font-weight: 500;
    cursor: pointer;
    transition:
      color var(--motion-fast) ease,
      border-color var(--motion-fast) ease;
  }

  .tab:hover {
    color: var(--text-primary);
  }

  .tab.active {
    color: var(--primary);
    border-bottom-color: var(--primary);
  }

  .count {
    color: var(--text-tertiary);
    font-size: var(--text-2xs);
    font-variant-numeric: tabular-nums;
  }

  .tab.active .count {
    color: var(--primary);
  }

  @media (width < 38rem) {
    .tabs {
      overflow-x: auto;
      scrollbar-width: none;
    }

    .tab {
      white-space: nowrap;
    }
  }
</style>
