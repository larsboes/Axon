<script lang="ts">
  import type { HostWatchFinding } from "../../api";
  import { link } from "../../nav";
  import Icon from "../../Icon.svelte";
  import ListRow from "../../ListRow.svelte";
  import RowMeta from "../../RowMeta.svelte";
  import { metaLine, relativeDate } from "../format";
  import type { DecisionRowProps } from "../decisions";

  let { row, id, current, tone, whyHere }: DecisionRowProps<HostWatchFinding> = $props();

  /// The note's opening line states the condition and the rest is what to run about it.
  /// `whyHere` is that first line, so rendering the whole note below it printed the same
  /// sentence twice on every finding.
  const commands = $derived(
    row.note.split("\n").slice(1).join("\n").trim(),
  );
</script>

<ListRow {id} {current} {tone}>
  {#snippet mark()}<span class="alarm"><Icon name="alert" size={15} /></span>{/snippet}

  <span class="row-kind">{metaLine("Host", `first seen ${relativeDate(row.first_seen)}`)}</span>
  <span class="row-title">{row.title}</span>
  <!-- Four lines and pre-wrap, not the one-line clamp every other row uses: the note is
       what to run to look at the condition and what to run if it is stuck. It is the
       whole surface, because there is no button here — host-watch closes a finding itself
       when the next hourly run stops seeing it, and a Dismiss control would let an
       operator silence a machine fault that is still true. -->
  {#if commands}<pre class="note">{commands}</pre>{/if}

  {#snippet meta()}<RowMeta {whyHere} />{/snippet}

  {#snippet actions()}
    <a class="btn" href={link("/systems")}>Systems</a>
  {/snippet}
</ListRow>

<style>
  .alarm {
    color: var(--band-alarm);
  }

  .note {
    display: -webkit-box;
    overflow: hidden;
    margin: 0;
    color: var(--text-secondary);
    font-family: var(--font-mono);
    font-size: var(--text-2xs);
    line-height: var(--leading-normal);
    white-space: pre-wrap;
    -webkit-box-orient: vertical;
    -webkit-line-clamp: 4;
    line-clamp: 4;
  }
</style>
