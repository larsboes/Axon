<script lang="ts">
  import type { DataClass } from "./home/decisions";

  /**
   * The one place a row states why it is here and where its rank came from.
   *
   * PRD:227 — "Evidence before automation. Every claim the system shows keeps its source
   * and its decision state. Agents rank and explain." A row that moved because a model
   * said so and cannot say why is the failure that principle names, so `method` renders
   * beside `whyHere` whenever a model produced the rank.
   *
   * Every optional field renders nothing when null. The data class is shown where a
   * capability publishes one — mail today — and is silent where none does, rather than
   * guessed: the dashboard owns no data (dashboard/README.md:7-9) and a class it invented
   * would be a false provenance claim.
   */
  let {
    whyHere,
    dataClass = null,
    processingRoute = null,
    method = null,
    candidateStatus,
  }: {
    whyHere: string;
    dataClass?: DataClass | null;
    processingRoute?: "local" | "cloud" | null;
    /** The classifier or evaluator that produced the rank, e.g. a model revision. */
    method?: string | null;
    candidateStatus?: "proposed" | "accepted" | "open";
  } = $props();

  const CLASS_LABEL: Record<DataClass, string> = {
    c0: "Public",
    c1: "Money",
    c2: "Private",
    c3: "Sensitive",
  };

  const redacted = $derived(dataClass === "c2" || dataClass === "c3");
</script>

<p class="why">
  {#if whyHere}<span class="text">{whyHere}</span>{/if}
  {#if method}<span class="method mono" title="What ranked this row">{method}</span>{/if}
  {#if dataClass}
    <!-- Visible, and with no effect on rank: the ranking policy states plainly that the
         data class does not move a score. It says where the row may be processed. -->
    <span class="class" class:redacted>{CLASS_LABEL[dataClass]}</span>
  {/if}
  {#if processingRoute}<span class="route">{processingRoute}</span>{/if}
  {#if candidateStatus && candidateStatus !== "open"}
    <span class="status">{candidateStatus}</span>
  {/if}
</p>

<style>
  .why {
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    gap: var(--space-1) var(--space-3);
    margin: 0;
    color: var(--text-tertiary);
    font-size: var(--text-2xs);
    line-height: var(--leading-normal);
  }

  .text {
    color: var(--text-secondary);
  }

  .method,
  .route,
  .status {
    font-size: var(--text-2xs);
  }

  .class {
    padding: 0 0.3em;
    border-radius: var(--radius-sm);
    background-color: var(--surface);
  }

  .class.redacted {
    color: var(--warning-ink);
    background-color: var(--warning-soft);
  }
</style>
