<script lang="ts">
  import { scouting, type ScoutingOpportunity } from "../../api";
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
  }: DecisionRowProps<ScoutingOpportunity> = $props();
</script>

<ListRow {id} {current} {tone}>
  {#snippet mark()}<Icon name="compass" size={15} />{/snippet}

  <span class="row-kind">
    {metaLine("Opportunity", row.starts_at ? dateLabel(row.starts_at) : null, row.city)}
  </span>
  <a class="row-title" {href} target="_blank" rel="noreferrer">{row.title}</a>

  {#snippet meta()}<RowMeta {whyHere} {candidateStatus} />{/snippet}

  {#snippet actions()}
    <a class="btn" {href} target="_blank" rel="noreferrer">Open</a>
    <button
      class="btn btn-soft"
      type="button"
      disabled={busy}
      onclick={() => act(() => scouting.setStatus(row.id, "saved").then(() => undefined), { dismiss: true })}
    >
      {#if busy}<Icon name="loader" size={13} />{:else}Save{/if}
    </button>
    <button
      class="btn"
      type="button"
      disabled={busy}
      aria-label="Dismiss opportunity"
      title="Dismiss"
      onclick={() => act(() => scouting.setStatus(row.id, "dismissed").then(() => undefined), { dismiss: true })}
    >
      <Icon name="close" size={13} />
    </button>
  {/snippet}
</ListRow>
