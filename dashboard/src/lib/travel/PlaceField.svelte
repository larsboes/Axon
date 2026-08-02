<script lang="ts">
  import Icon from "$lib/Icon.svelte";
  import { transit, type PlaceRef, type Station } from "$lib/api";

  let {
    label,
    placeholder,
    place = $bindable(),
  }: { label: string; placeholder: string; place: PlaceRef | null } = $props();

  let typed = $state("");
  let suggestions = $state<Station[]>([]);
  let open = $state(false);
  let field: HTMLDivElement | undefined = $state();
  const shown = $derived(place ? place.name : typed);

  $effect(() => {
    const query = typed.trim();
    if (place || query.length < 2) {
      suggestions = [];
      return;
    }
    const timer = setTimeout(async () => {
      try {
        suggestions = await transit.suggest(query);
      } catch {
        suggestions = [];
      }
    }, 200);
    return () => clearTimeout(timer);
  });

  function pick(station: Station): void {
    place = { ...station, kind: "station" };
    typed = station.name;
    open = false;
  }

  function commitTypedPlace(): void {
    const name = typed.trim();
    if (!place && name) {
      const slug = name
        .toLocaleLowerCase("en-GB")
        .replace(/[^\p{Letter}\p{Number}]+/gu, "-")
        .replace(/^-|-$/g, "");
      place = {
        id: `place:${slug || encodeURIComponent(name)}`,
        name,
        kind: "city",
        latitude: null,
        longitude: null,
      };
    }
    open = false;
  }

  function closeOnOutside(event: MouseEvent): void {
    if (field && !field.contains(event.target as Node)) {
      commitTypedPlace();
    }
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
        oninput={(event) => {
          typed = event.currentTarget.value;
          place = null;
          open = true;
        }}
        onfocus={() => (open = true)}
        onblur={() => {
          if (suggestions.length === 0) commitTypedPlace();
        }}
        onkeydown={(event) => {
          if (event.key === "Enter" && open) {
            event.preventDefault();
            commitTypedPlace();
          }
        }}
      />
    </span>
  </label>

  {#if open && suggestions.length > 0}
    <div class="card">
      <p>Select a station or use a free-form place</p>
      <ul>
        {#each suggestions as station (station.id)}
          <li>
            <button type="button" onmousedown={(event) => event.preventDefault()} onclick={() => pick(station)}>
              <Icon name="train" size={13} />
              {station.name}
            </button>
          </li>
        {/each}
      </ul>
      <button class="free-place" type="button" onmousedown={(event) => event.preventDefault()} onclick={commitTypedPlace}>
        <Icon name="map-pin" size={13} />
        Use “{typed.trim()}” as a place
      </button>
    </div>
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
    color: var(--text-tertiary);
    font-size: 0.6875rem;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }

  .wrap {
    position: relative;
    display: block;
  }

  .wrap :global(svg) {
    position: absolute;
    top: 50%;
    left: 0.7rem;
    color: var(--text-tertiary);
    pointer-events: none;
    transform: translateY(-50%);
  }

  input {
    padding-left: 2rem;
  }

  .card {
    position: absolute;
    z-index: 30;
    top: calc(100% + 0.25rem);
    width: 100%;
    overflow: hidden;
    padding: 0.25rem;
    box-shadow: var(--card-shadow-hover);
  }

  p {
    margin: 0;
    padding: 0.35rem 0.5rem;
    color: var(--text-tertiary);
    font-size: 0.625rem;
  }

  ul {
    max-height: 11rem;
    overflow-y: auto;
    margin: 0;
    padding: 0;
    list-style: none;
  }

  button {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    width: 100%;
    padding: 0.5rem 0.6rem;
    border: 0;
    border-radius: var(--radius-sm);
    background: none;
    color: var(--text-primary);
    font: inherit;
    font-size: 0.75rem;
    text-align: left;
    cursor: pointer;
  }

  button:hover,
  button:focus-visible {
    background: var(--primary-soft);
    color: var(--primary);
    outline: none;
  }

  .free-place {
    border-top: 1px solid var(--card-border);
    border-radius: 0;
  }
</style>
