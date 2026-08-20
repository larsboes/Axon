<script lang="ts">
  /**
   * Operator-pinned links, straight from the overlay's links.toml via axon-status.
   *
   * The shell owns the card; the overlay owns the entries. A deployment that pins
   * nothing renders nothing — no empty-state prose for a purely optional surface.
   * Read-only, outbound: these are the operator's own services (a tailnet page, a
   * family-node UI), so they open in a new tab and never route through nav.ts.
   */
  import { onMount } from "svelte";
  import Icon from "$lib/Icon.svelte";
  import { axonStatus, type PinnedLink } from "$lib/api";

  let links = $state<PinnedLink[]>([]);

  onMount(async () => {
    try {
      links = (await axonStatus.links()).links;
    } catch {
      // The card is decoration: an unreachable axon-status already renders loudly in
      // the surfaces that own health, so this stays quiet and empty.
    }
  });
</script>

{#if links.length > 0}
  <section class="pinned">
    <div class="head">
      <span class="kicker">Pinned</span>
      <h2>Links</h2>
    </div>
    <ul>
      {#each links as pinned (pinned.url)}
        <li>
          <a href={pinned.url} target="_blank" rel="noreferrer">
            <span class="name">{pinned.name}</span>
            {#if pinned.note}<small>{pinned.note}</small>{/if}
            <Icon name="external" size={12} />
          </a>
        </li>
      {/each}
    </ul>
  </section>
{/if}

<style>
  .pinned {
    border: 1px solid var(--card-border);
    border-radius: 10px;
    padding: 0.9rem 1rem;
  }
  .head {
    display: flex;
    flex-direction: column;
    gap: 0.1rem;
    margin-bottom: 0.5rem;
  }
  .kicker {
    font-size: 0.7rem;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--text-tertiary, #888);
  }
  h2 {
    font-size: 0.95rem;
    margin: 0;
  }
  ul {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    gap: 0.35rem;
  }
  a {
    display: flex;
    align-items: baseline;
    gap: 0.45rem;
    text-decoration: none;
    color: inherit;
    padding: 0.25rem 0.3rem;
    border-radius: 6px;
  }
  a:hover {
    background: var(--card-border, #eee);
  }
  .name {
    font-weight: 600;
    font-size: 0.85rem;
  }
  small {
    color: var(--text-tertiary, #888);
    font-size: 0.75rem;
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
