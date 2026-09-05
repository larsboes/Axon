<script lang="ts">
  import { calendar, type CalendarEntry } from "../../api";
  import Icon from "../../Icon.svelte";
  import ListRow from "../../ListRow.svelte";
  import RowMeta from "../../RowMeta.svelte";
  import { dateLabel, metaLine } from "../format";
  import type { DecisionRowProps } from "../decisions";

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
  }: DecisionRowProps<CalendarEntry> = $props();
</script>

<ListRow {id} {current} {tone} {href}>
  {#snippet mark()}<span class="event"><Icon name="ticket" size={15} /></span>{/snippet}

  <span class="row-kind">
    {metaLine(
      "Calendar opportunity",
      dateLabel(row.starts_at),
      row.location,
      row.source === "web" && "added deliberately",
    )}
  </span>
  <a class="row-title" {href}>{row.title}</a>
  {#if row.notes}<p class="row-text">{row.notes}</p>{/if}

  {#snippet meta()}<RowMeta {whyHere} {candidateStatus} />{/snippet}

  {#snippet actions()}
    <a class="btn" {href}>Calendar</a>
    <button
      class="btn btn-primary"
      type="button"
      disabled={busy}
      onclick={() =>
        act(async () => {
          await calendar.entries.update(row.id, { commitment: "planned" });
        }, { dismiss: true })}
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
