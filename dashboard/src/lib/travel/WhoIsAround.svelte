<script lang="ts">
  /**
   * Who you know near each leg, and the meetups the plan holds.
   *
   * Confirmed companion-register rows only: a proposal is a guess until it is
   * confirmed on /map. The join runs here, on this machine (who-is-around.ts).
   */
  import { link } from "$lib/nav";
  import Icon from "$lib/Icon.svelte";
  import type { PeopleLayer, PlanItem, TripStage } from "$lib/api";
  import { AROUND_RADIUS_KM, meetupsOf, whoIsAround } from "$lib/travel/who-is-around";

  let {
    stages,
    items,
    layer,
    coordinates,
    notice = null,
  }: {
    stages: TripStage[];
    items: PlanItem[];
    layer: PeopleLayer | null;
    /** Each leg's destination coordinate, by stage id; null when none resolved. */
    coordinates: Record<string, [number, number] | null>;
    notice?: string | null;
  } = $props();

  const meetups = $derived(meetupsOf(items));
  const legs = $derived(
    stages.map((stage) => ({
      stage,
      resolved: coordinates[stage.id] != null,
      around: layer ? whoIsAround(coordinates[stage.id] ?? null, stage.date, layer) : [],
    })),
  );
</script>

<p class="rail-hint">
  Confirmed people within {AROUND_RADIUS_KM} km of each leg's destination on its day, and
  the meetups on this plan.
</p>

{#if notice}
  <p class="rail-notice" aria-live="polite">{notice}</p>
{/if}

{#if meetups.length > 0}
  <ol class="around-list">
    {#each meetups as meetup (meetup.itemId)}
      <li>
        <p class="who"><strong>{meetup.people.join(", ")}</strong><span>{meetup.title}</span></p>
        <p class="meta">
          {meetup.status} · {meetup.day ?? "no day yet"}{meetup.place ? ` · ${meetup.place}` : ""}
        </p>
      </li>
    {/each}
  </ol>
{/if}

<ol class="around-list">
  {#each legs as leg (leg.stage.id)}
    <li>
      <p class="who">
        <strong>{leg.stage.destination.name}</strong>
        <span>{leg.stage.date ?? "no date"}</span>
      </p>
      {#if !leg.resolved}
        <p class="meta">No coordinate for this destination, so nobody is matched.</p>
      {:else if leg.around.length === 0}
        <p class="meta">Nobody confirmed nearby.</p>
      {:else}
        <p class="when">
          {leg.around.map((p) => `${p.person} (${p.distanceKm.toFixed(0)} km)`).join(", ")}
        </p>
      {/if}
    </li>
  {/each}
</ol>

<a class="btn btn-outline rail-action" href={link("/map")}>
  <Icon name="map-pin" size={13} /> Confirm people on the map
</a>

<style>
  /* Local copies of the rail lines, for the reason CompanionRail.svelte states. */
  .rail-hint {
    margin: 0 0 0.4rem;
    color: var(--text-secondary);
    font-size: var(--text-xs);
    line-height: 1.45;
  }

  .rail-notice {
    margin: 0 0 0.4rem;
    padding: 0.4rem 0.5rem;
    border-radius: var(--radius-sm);
    background: var(--primary-soft);
    color: var(--text-secondary);
    font-size: 0.72rem;
    line-height: 1.4;
  }

  .around-list {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
    margin: 0 0 0.6rem;
    padding: 0;
    list-style: none;
  }

  .around-list li {
    display: flex;
    flex-direction: column;
    gap: 0.15rem;
    padding: 0.5rem 0.6rem;
    border: 1px solid var(--card-border);
    border-radius: var(--radius-sm);
  }

  .who {
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    gap: 0.4rem;
    margin: 0;
    font-size: var(--text-sm);
  }

  .who span {
    color: var(--text-secondary);
  }

  .when {
    margin: 0;
    font-size: var(--text-xs);
    color: var(--text-primary);
  }

  .meta {
    margin: 0;
    font-size: var(--text-2xs);
    color: var(--text-tertiary);
  }

  .rail-action {
    display: inline-flex;
    width: 100%;
    align-items: center;
    justify-content: center;
    gap: 0.35rem;
    margin-top: 0.5rem;
    padding: 0.3rem 0.5rem;
    font-size: 0.72rem;
    text-decoration: none;
  }
</style>
