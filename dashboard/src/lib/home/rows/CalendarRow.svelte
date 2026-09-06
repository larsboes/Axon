<script lang="ts">
  import { calendar, type CalendarEntry } from "../../api";
  import Icon from "../../Icon.svelte";
  import ListRow from "../../ListRow.svelte";
  import RowMeta from "../../RowMeta.svelte";
  import { dateLabel, metaParts } from "../format";
  import type { DecisionRowProps } from "../decisions";
  import type { CalendarSource } from "../kinds/calendar";

  let {
    row,
    busy,
    act,
    id,
    current,
    tone,
    href,
    whyHere,
    candidateStatus,
  }: DecisionRowProps<CalendarEntry, CalendarSource> = $props();

  /// The entry the capability answered with, held between the write and the patch below.
  let planned: CalendarEntry | null = null;

  /// The updated entry replaces the old one in the source, so the horizon above the ladder
  /// shows the entry the moment it is planned rather than at the next reload. Replaced and
  /// not removed: a planned entry is still a dated commitment, it has just stopped being a
  /// question. The base page wrote exactly this map.
  function plan(): void {
    act(
      async () => {
        const updated = await calendar.entries.update(row.id, { commitment: "planned" });
        planned = updated;
      },
      {
        dismiss: true,
        patch: (source) => ({
          ...source,
          entries: source.entries.map((entry) =>
            entry.id === row.id ? (planned ?? { ...entry, commitment: "planned" }) : entry,
          ),
        }),
      },
    );
  }
</script>

<ListRow {id} {current} {tone} {href}>
  {#snippet mark()}<span class="event"><Icon name="ticket" size={15} /></span>{/snippet}

  <span class="row-kind">
    {#each metaParts( "Calendar opportunity", dateLabel(row.starts_at), row.location, row.source === "web" && "added deliberately", ) as part}<span
      >{part}</span
    >{/each}
  </span>
  <a class="row-title" {href}>{row.title}</a>
  {#if row.notes}<p class="row-text">{row.notes}</p>{/if}

  {#snippet meta()}<RowMeta {whyHere} {candidateStatus} />{/snippet}

  {#snippet actions()}
    <a class="btn" {href}>Calendar</a>
    <button
      class="btn btn-soft"
      type="button"
      disabled={busy}
      onclick={plan}
    >
      {#if busy}<Icon name="loader" size={13} />{:else}Plan{/if}
    </button>
  {/snippet}
</ListRow>

<style>
  .event {
    color: var(--kind-event);
  }
</style>
