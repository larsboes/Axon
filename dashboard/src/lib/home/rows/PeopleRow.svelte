<script lang="ts">
  import Icon from "../../Icon.svelte";
  import ListRow from "../../ListRow.svelte";
  import RowMeta from "../../RowMeta.svelte";
  import { link } from "../../nav";
  import { metaParts } from "../format";
  import type { DecisionRowProps } from "../decisions";
  import type { PeopleSource, PersonDecisionRow } from "../kinds/people";

  let {
    row,
    id,
    current,
    tone,
    href,
    whyHere,
    dataClass,
    candidateStatus,
  }: DecisionRowProps<PersonDecisionRow, PeopleSource> = $props();

  const isBirthday = $derived(row.reason === "birthday");
  const kind = $derived(
    metaParts(
      isBirthday ? "Birthday Radar" : "Trip Proximity",
      row.daysUntil === 0 ? "today" : row.daysUntil === 1 ? "tomorrow" : `in ${row.daysUntil}d`,
      row.city,
    ),
  );
</script>

<ListRow {id} {current} {tone} {href}>
  {#snippet mark()}
    <span class="people-mark" class:birthday={isBirthday}>
      <Icon name={isBirthday ? "sparkles" : "users"} size={15} />
    </span>
  {/snippet}

  <span class="row-kind">
    {#each kind as part}
      <span>{part}</span>
    {/each}
  </span>

  <a class="row-title" {href}>{row.title}</a>
  {#if row.detail}<p class="row-text">{row.detail}</p>{/if}

  {#snippet meta()}
    <RowMeta {whyHere} {dataClass} {candidateStatus} />
  {/snippet}

  {#snippet actions()}
    <a class="btn btn-soft" {href}>View dossier</a>
    {#if row.city}
      <a
        class="btn"
        href={link(
          `/calendar?title=${encodeURIComponent(`Meetup with ${row.person.name}`)}&location=${encodeURIComponent(row.city)}`,
        )}
      >
        Plan meetup
      </a>
    {/if}
  {/snippet}
</ListRow>

<style>
  .people-mark {
    color: var(--primary);
  }
  .people-mark.birthday {
    color: var(--warning-ink);
  }
</style>
