<script lang="ts">
  /**
   * Who you know near each leg, and the meetups the plan holds.
   *
   * Confirmed companion-register rows only: a proposal is a guess until it is
   * confirmed on /map. The join runs here, on this machine (who-is-around.ts).
   */
  import { link } from "$lib/nav";
  import Icon from "$lib/Icon.svelte";
  import { places, type PeopleLayer, type PersonFacts, type PlanItem, type TripStage } from "$lib/api";
  import { AROUND_RADIUS_KM, hostsAround, meetupsOf, staysOf, whoIsAround } from "$lib/travel/who-is-around";

  let {
    stages,
    items,
    layer,
    coordinates,
    people = [],
    notice = null,
    onStated,
  }: {
    stages: TripStage[];
    items: PlanItem[];
    layer: PeopleLayer | null;
    /** Each leg's destination coordinate, by stage id; null when none resolved. */
    coordinates: Record<string, [number, number] | null>;
    /** Atlas/People as vault reads it: names for the form, `host` for places to stay. */
    people?: PersonFacts[];
    notice?: string | null;
    /** Called after a stated place is confirmed, so the page reloads the layer. */
    onStated?: () => void;
  } = $props();

  const stays = $derived(staysOf(items));

  // "Where is someone": the operator states it, places writes it proposed, and this
  // same action confirms it (capabilities/places/src/server.rs, state_person_place).
  let formPerson = $state("");
  let formCity = $state("");
  let formFrom = $state("");
  let formTo = $state("");
  let formBusy = $state(false);
  let formMessage = $state<string | null>(null);

  async function statePlace(event: SubmitEvent): Promise<void> {
    event.preventDefault();
    formBusy = true;
    formMessage = null;
    try {
      const stated = await places.statePersonPlace({
        person: formPerson.trim(),
        city: formCity.trim(),
        from: formFrom || undefined,
        to: formTo || undefined,
      });
      await places.confirmProposal(stated.id).catch((caught: unknown) => {
        // 409: already confirmed, which is the state this form wants.
        if (!(caught instanceof Error && caught.message.includes("409"))) throw caught;
      });
      formMessage = `Saved: ${formPerson.trim()} in ${stated.place_name}.`;
      formCity = "";
      formFrom = "";
      formTo = "";
      onStated?.();
    } catch (caught) {
      formMessage = caught instanceof Error ? caught.message : String(caught);
    } finally {
      formBusy = false;
    }
  }

  const meetups = $derived(meetupsOf(items));
  const legs = $derived(
    stages.map((stage) => ({
      stage,
      resolved: coordinates[stage.id] != null,
      around: layer ? whoIsAround(coordinates[stage.id] ?? null, stage.date, layer) : [],
      hosts: layer ? hostsAround(coordinates[stage.id] ?? null, stage.date, layer, people) : [],
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
      {#if leg.hosts.length > 0}
        <p class="stay-line">
          <Icon name="home" size={12} /> Could stay with:
          {leg.hosts.map((h) => (h.note ? `${h.person} (${h.note})` : h.person)).join(", ")}
        </p>
      {/if}
    </li>
  {/each}
</ol>

{#if stays.length > 0}
  <p class="rail-hint">Stays on this plan</p>
  <ol class="around-list">
    {#each stays as stay (stay.title + stay.checkIn)}
      <li>
        <p class="who"><strong>{stay.title}</strong></p>
        <p class="meta">{stay.checkIn ?? "?"} – {stay.checkOut ?? "?"}</p>
      </li>
    {/each}
  </ol>
{/if}

<form class="state-form" onsubmit={statePlace}>
  <p class="rail-hint">Where is someone? Saved as confirmed.</p>
  <input aria-label="Person" placeholder="Person" list="people-names" bind:value={formPerson} required />
  <datalist id="people-names">
    {#each people as person (person.id)}
      <option value={person.name}></option>
    {/each}
  </datalist>
  <input aria-label="City" placeholder="City" bind:value={formCity} required />
  <div class="dates">
    <input aria-label="From" type="date" bind:value={formFrom} />
    <input aria-label="To" type="date" bind:value={formTo} />
  </div>
  <button class="btn btn-outline" type="submit" disabled={formBusy}>
    {formBusy ? "Saving…" : "Save"}
  </button>
  {#if formMessage}<p class="meta" aria-live="polite">{formMessage}</p>{/if}
</form>

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

  .stay-line {
    display: flex;
    align-items: center;
    gap: 0.3rem;
    margin: 0;
    font-size: var(--text-xs);
    color: var(--text-secondary);
  }

  .state-form {
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
    margin-top: 0.6rem;
  }

  .state-form input {
    padding: 0.3rem 0.45rem;
    border: 1px solid var(--card-border);
    border-radius: var(--radius-sm);
    background: var(--card-bg);
    color: var(--text-primary);
    font: inherit;
    font-size: var(--text-xs);
  }

  .state-form .dates {
    display: flex;
    gap: 0.35rem;
  }

  .state-form .dates input {
    flex: 1;
    min-width: 0;
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
