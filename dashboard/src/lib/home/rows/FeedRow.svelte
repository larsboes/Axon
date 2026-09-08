<script lang="ts">
  import { comms, type FeedEntry } from "../../api";
  import FeedItemRow from "../../feed/FeedItemRow.svelte";
  import type { DecisionRowProps } from "../decisions";

  /** A thin wrapper, so Home's reading lane and /feed's inbox cannot drift apart. */
  let { row, busy, act, id, current, tone }: DecisionRowProps<FeedEntry, FeedEntry[]> = $props();

  /// The entry leaves the loaded list as well as the ladder: Sources shows the size of the
  /// 30-day window, and a kept item is no longer in it.
  const decide = (status: "keeper" | "dismissed") => () =>
    act(() => comms.setStatus(row.id, status).then(() => undefined), {
      dismiss: true,
      patch: (source) => source.filter((entry) => entry.id !== row.id),
    });
</script>

<FeedItemRow
  entry={row}
  {id}
  {current}
  {busy}
  {tone}
  onkeep={decide("keeper")}
  ondismiss={decide("dismissed")}
/>
