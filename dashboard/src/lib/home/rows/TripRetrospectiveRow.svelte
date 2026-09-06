<script lang="ts">
  import type { PendingRetrospective } from "../../travel/api";
  import Icon from "../../Icon.svelte";
  import ListRow from "../../ListRow.svelte";
  import RowMeta from "../../RowMeta.svelte";
  import { dateLabel, metaParts } from "../format";
  import type { DecisionRowProps } from "../decisions";

  let {
    row,
    id,
    current,
    tone,
    href,
    whyHere,
    candidateStatus,
  }: DecisionRowProps<PendingRetrospective> = $props();
  const where = $derived(row.destinations.join(" → "));



  const kind = $derived(metaParts("Retrospective", dateLabel(row.date_end), where));
</script>

<ListRow {id} {current} {tone} {href}>
  {#snippet mark()}<Icon name="history" size={15} />{/snippet}

  <span class="row-kind">{#each kind as part}<span>{part}</span>{/each}</span>
  <a class="row-title" {href}>{row.title}</a>

  {#snippet meta()}<RowMeta {whyHere} {candidateStatus} />{/snippet}

  {#snippet actions()}
    <a class="btn btn-soft" {href}>
      Record it <Icon name="arrow-right" size={13} />
    </a>
  {/snippet}
</ListRow>
