<script lang="ts">
  import { comms, type FeedEntry } from "../../api";
  import FeedItemRow from "../../feed/FeedItemRow.svelte";
  import type { DecisionRowProps } from "../decisions";

  /** A thin wrapper, so Home's reading lane and /feed's inbox cannot drift apart. */
  let { row, busy, act, id, current, tone }: DecisionRowProps<FeedEntry> = $props();
</script>

<FeedItemRow
  entry={row}
  {id}
  {current}
  {busy}
  {tone}
  onkeep={() => act(() => comms.setStatus(row.id, "keeper").then(() => undefined), { dismiss: true })}
  ondismiss={() => act(() => comms.setStatus(row.id, "dismissed").then(() => undefined), { dismiss: true })}
/>
