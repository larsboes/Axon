<script lang="ts">
  import Icon from "$lib/Icon.svelte";
  import type { Journey } from "$lib/api";

  let {
    journey,
    expanded,
    saved,
    onToggle,
    onSave,
  }: {
    journey: Journey;
    expanded: boolean;
    saved: boolean;
    onToggle: () => void;
    onSave: () => void;
  } = $props();

  const time = (date: string) =>
    new Intl.DateTimeFormat("en-GB", { hour: "2-digit", minute: "2-digit" }).format(
      new Date(date),
    );

  const duration = (minutes: number) =>
    `${Math.floor(minutes / 60)}:${String(minutes % 60).padStart(2, "0")} h`;
</script>

<li class:expanded>
  <button class="journey-open" type="button" aria-expanded={expanded} onclick={onToggle}>
    <span class="journey-time">
      <strong>{time(journey.legs[0]?.departure_time ?? "")}</strong>
      <span>{duration(journey.total_duration_minutes)}</span>
    </span>
    <span class="journey-route">
      <strong>{journey.legs.map((leg) => leg.train_name || leg.train_number).join(" · ")}</strong>
      <span>
        {journey.legs.length - 1 === 0 ? "direkt" : `${journey.legs.length - 1} Umstieg`}
      </span>
    </span>
    <Icon name="arrow-right" size={13} />
  </button>

  <div class="journey-action">
    <strong>
      {journey.total_price === null ? "Preis offen" : `${journey.total_price.toFixed(2)} €`}
    </strong>
    <button class="save" type="button" disabled={saved} onclick={onSave}>
      {#if saved}
        <Icon name="check" size={13} /> Gespeichert
      {:else}
        <Icon name="plus" size={13} /> Ablauf
      {/if}
    </button>
  </div>

  {#if expanded}
    <ol class="leg-list">
      {#each journey.legs as leg, index (`${journey.id}:${index}`)}
        <li>
          <span class="leg-index">{index + 1}</span>
          <div>
            <strong>{leg.train_name || leg.train_number}</strong>
            <span>
              {time(leg.departure_time)} {leg.origin.name}
              <span aria-hidden="true">→</span>
              {time(leg.arrival_time)} {leg.destination.name}
            </span>
          </div>
          {#if leg.platform}
            <small>Gleis {leg.platform}</small>
          {/if}
        </li>
      {/each}
    </ol>
  {/if}
</li>

<style>
  li {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    gap: 0.75rem;
    align-items: center;
    padding: 0.75rem;
    border-bottom: 1px solid var(--card-border);
  }

  li:last-child {
    border-bottom: 0;
  }

  li.expanded {
    background: color-mix(in srgb, var(--primary-soft) 38%, var(--card-bg));
  }

  .journey-open {
    display: grid;
    grid-template-columns: 4rem minmax(0, 1fr) auto;
    gap: 0.75rem;
    align-items: center;
    min-width: 0;
    padding: 0;
    border: 0;
    background: transparent;
    color: var(--text-primary);
    font: inherit;
    text-align: left;
    cursor: pointer;
  }

  .journey-open :global(svg) {
    color: var(--text-tertiary);
    transition: transform 140ms ease;
  }

  .journey-open[aria-expanded="true"] :global(svg) {
    transform: rotate(90deg);
  }

  .journey-time strong,
  .journey-time span,
  .journey-route strong,
  .journey-route span,
  .journey-action strong {
    display: block;
  }

  .journey-time strong {
    font-family: var(--font-mono);
    font-size: 0.8125rem;
  }

  .journey-time span,
  .journey-route span {
    color: var(--text-tertiary);
    font-size: 0.625rem;
  }

  .journey-route {
    min-width: 0;
  }

  .journey-route strong {
    overflow: hidden;
    font-size: 0.75rem;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .journey-action {
    text-align: right;
  }

  .journey-action strong {
    font-size: 0.75rem;
  }

  .save {
    display: inline-flex;
    align-items: center;
    gap: 0.25rem;
    margin-top: 0.2rem;
    padding: 0.2rem 0.35rem;
    border: 0;
    border-radius: var(--radius-sm);
    background: var(--primary-soft);
    color: var(--primary);
    font: inherit;
    font-size: 0.625rem;
    font-weight: 600;
    cursor: pointer;
  }

  .save:disabled {
    color: var(--success);
    cursor: default;
  }

  .leg-list {
    grid-column: 1 / -1;
    list-style: none;
    margin: 0;
    padding: 0.35rem 0 0 4.75rem;
    border-top: 1px solid var(--card-border);
  }

  .leg-list > li {
    display: grid;
    grid-template-columns: 1.5rem minmax(0, 1fr) auto;
    gap: 0.55rem;
    align-items: start;
    padding: 0.6rem 0;
    border-bottom: 1px solid var(--card-border);
  }

  .leg-list > li:last-child {
    border-bottom: 0;
  }

  .leg-index {
    display: grid;
    place-items: center;
    width: 1.35rem;
    height: 1.35rem;
    border-radius: 50%;
    background: var(--primary-soft);
    color: var(--primary);
    font-family: var(--font-mono);
    font-size: 0.5625rem;
    font-weight: 700;
  }

  .leg-list strong,
  .leg-list span {
    display: block;
  }

  .leg-list strong {
    font-size: 0.6875rem;
  }

  .leg-list div > span,
  .leg-list small {
    margin-top: 0.15rem;
    color: var(--text-tertiary);
    font-size: 0.625rem;
  }

  .leg-list div > span > span {
    display: inline;
    margin: 0 0.2rem;
  }
</style>
