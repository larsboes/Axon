<script lang="ts">
  import type { Task } from "../../api";
  import Icon from "../../Icon.svelte";
  import ListRow from "../../ListRow.svelte";
  import RowMeta from "../../RowMeta.svelte";
  import { dateLabel, metaParts } from "../format";
  import type { DecisionRowProps } from "../decisions";

  let { row, id, current, tone, href, whyHere, dataClass }: DecisionRowProps<Task> = $props();



  const kind = $derived(metaParts("Task", row.due ? `due ${dateLabel(row.due)}` : null, ...row.projects));
  const overdue = $derived(row.due !== null && row.due.slice(0, 10) < new Date().toISOString().slice(0, 10));
</script>

<ListRow {id} {current} {tone}>
  {#snippet mark()}<Icon name="check" size={15} />{/snippet}

  <span class="row-kind">
    {#each kind as part}<span
      >{part}</span
    >{/each}
    {#if overdue}<span class="overdue">overdue</span>{/if}
  </span>
  <!-- The title links to the note, because the note IS the task. There is no in-page
       title for the same reason there is no Done button: this row reads a file a human
       owns, and Obsidian is where that file gets written (PRD Q48). -->
  <a class="row-title" {href}>{row.title}</a>
  {#if row.summary}<p class="row-text">{row.summary}</p>{/if}

  {#snippet meta()}<RowMeta {whyHere} {dataClass} candidateStatus="accepted" />{/snippet}

  {#snippet actions()}
    <a class="btn btn-soft" {href}>Open note</a>
  {/snippet}
</ListRow>

<style>
  .overdue {
    margin-left: 0.35em;
    padding: 0 0.3em;
    border-radius: var(--radius-sm);
    background-color: var(--warning-soft);
    color: var(--warning-ink);
    font-weight: 600;
  }
</style>
