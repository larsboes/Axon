<script lang="ts">
  /** Shared station picker for connection search and persistent trip planning. */
  import Icon from "$lib/Icon.svelte";
  import { transit, type Station } from "$lib/api";

  let {
    label,
    placeholder,
    station = $bindable(),
  }: { label: string; placeholder: string; station: Station | null } = $props();

  let typed = $state("");
  let suggestions = $state<Station[]>([]);
  let open = $state(false);
  let field: HTMLDivElement | undefined = $state();

  // The input shows the picked station's name, never a separate copy of it. That is
  // what lets the page swap origin and destination by assigning the bound values:
  // there is no second source of truth here to fall out of sync with them.
  const shown = $derived(station ? station.name : typed);

  // Debounced, because /api/suggest proxies bahn.de live — one request per keystroke
  // would be one request per keystroke against someone else's API.
  $effect(() => {
    const q = typed.trim();
    if (station || q.length < 2) {
      suggestions = [];
      return;
    }
    const timer = setTimeout(async () => {
      try {
        suggestions = await transit.suggest(q);
      } catch {
        // A dead suggest endpoint means no suggestions, not a broken page: the search
        // button reports the outage when it is actually pressed.
        suggestions = [];
      }
    }, 200);
    return () => clearTimeout(timer);
  });

  function pick(s: Station): void {
    station = s;
    typed = s.name;
    open = false;
  }

  // mousedown, not click: a click on a suggestion would otherwise race the close.
  function closeOnOutside(event: MouseEvent): void {
    if (field && !field.contains(event.target as Node)) open = false;
  }
</script>

<svelte:window onmousedown={closeOnOutside} />

<div class="field" bind:this={field}>
  <label>
    <span>{label}</span>
    <span class="wrap">
      <Icon name="map-pin" size={14} />
      <input
        class="input"
        {placeholder}
        value={shown}
        autocomplete="off"
        oninput={(e) => {
          typed = e.currentTarget.value;
          station = null;
          open = true;
        }}
        onfocus={() => (open = true)}
      />
    </span>
  </label>

  {#if open && suggestions.length > 0}
    <ul class="card">
      {#each suggestions as s (s.id)}
        <li>
          <button type="button" onclick={() => pick(s)}>
            <Icon name="train" size={13} />
            {s.name}
          </button>
        </li>
      {/each}
    </ul>
  {/if}
</div>

<style>
  .field {
    position: relative;
  }

  label {
    display: flex;
    flex-direction: column;
    gap: 0.3rem;
    font-size: 0.6875rem;
    font-weight: 600;
    color: var(--text-tertiary);
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }

  .wrap {
    position: relative;
    display: block;
  }

  .wrap :global(svg) {
    position: absolute;
    left: 0.7rem;
    top: 50%;
    transform: translateY(-50%);
    color: var(--text-tertiary);
    pointer-events: none;
  }

  input {
    padding-left: 2rem;
  }

  ul {
    position: absolute;
    z-index: 30;
    top: calc(100% + 0.25rem);
    width: 100%;
    max-height: 14rem;
    overflow-y: auto;
    list-style: none;
    margin: 0;
    padding: 0.25rem;
    box-shadow: var(--card-shadow-hover);
  }

  li button {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    width: 100%;
    padding: 0.5rem 0.6rem;
    border: 0;
    border-radius: var(--radius-sm);
    background: none;
    font: inherit;
    font-size: 0.8125rem;
    color: var(--text-primary);
    text-align: left;
    cursor: pointer;
  }

  li button:hover,
  li button:focus-visible {
    background-color: var(--primary-soft);
    color: var(--primary);
    outline: none;
  }
</style>
