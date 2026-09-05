<script lang="ts">
  import type { TripPlan } from "../../api";
  import { link } from "../../nav";
  import Icon from "../../Icon.svelte";
  import ListRow from "../../ListRow.svelte";
  import RowMeta from "../../RowMeta.svelte";
  import { dateLabel, metaLine } from "../format";
  import type { DecisionRowProps } from "../decisions";

  let {
    row,
    id,
    current,
    tone,
    href,
    whyHere,
    candidateStatus,
  }: DecisionRowProps<TripPlan> = $props();

  const where = $derived(row.destinations.map((place) => place.name).join(" → "));
</script>

<ListRow {id} {current} {tone} {href}>
  {#snippet mark()}<Icon name="map-pin" size={15} />{/snippet}

  <span class="row-kind">{metaLine("Travel", dateLabel(row.date_start), where)}</span>
  <a class="row-title" {href}>{row.title}</a>

  {#snippet meta()}<RowMeta {whyHere} {candidateStatus} />{/snippet}

  {#snippet actions()}
    <a class="btn btn-primary" href={link("/travel")}>
      Continue planning <Icon name="arrow-right" size={13} />
    </a>
  {/snippet}
</ListRow>
