<script lang="ts">
  import type { CommsEvaluationStatus } from "$lib/api";
  import type { FeedStatusExtras } from "$lib/feed/api";

  // Widened, not replaced. Every added member is optional, so a caller holding a
  // plain `CommsEvaluationStatus` still satisfies this and keeps compiling; the
  // new cells sit behind `{#if}` and render nothing for it.
  let { status }: { status: CommsEvaluationStatus & Partial<FeedStatusExtras> } = $props();

  function shortModel(model: string): string {
    return model.split("/").at(-1) ?? model;
  }

  /** Epoch seconds, as the source-state table stores them, in local time. */
  function passTime(at: string): string {
    const seconds = Number(at);
    if (!Number.isFinite(seconds) || seconds <= 0) return "unknown";
    return new Date(seconds * 1000).toLocaleString("en-GB", {
      day: "numeric",
      month: "short",
      hour: "2-digit",
      minute: "2-digit",
    });
  }
</script>

<section class="model-status" aria-label="Local evaluation">
  <div class="intro">
    <p class="eyebrow mono">Local evaluation</p>
    <strong>{status.ledger.evaluated} entries in the evaluation ledger</strong>
  </div>
  <dl>
    <div>
      <dt>
        <span class:online={status.summarizer.reachable} class="dot"></span>
        Summary
      </dt>
      <dd>{shortModel(status.summarizer.model)}</dd>
      <small>{status.summarizer.reachable ? "available locally" : "configured, unavailable"}</small>
    </div>
    <div>
      <dt>
        <span class:online={status.relevance.reachable} class="dot"></span>
        TELOS relevance
      </dt>
      <dd>
        {status.reranker.reachable
          ? shortModel(status.reranker.model)
          : status.relevance.reachable
            ? shortModel(status.relevance.model)
          : "lexical fallback"}
      </dd>
      <small>{status.relevance.profile_count} lenses · {status.relevance.active_mode}</small>
    </div>
    <div>
      <dt>Evaluation mode</dt>
      <dd>{status.evaluator_revision}</dd>
      <small>{status.ledger.reranked} reranked · {status.ledger.semantic} semantic · {status.ledger.lexical} lexical</small>
    </div>
    {#if status.last_pass}
      <div>
        <dt>
          <span class:online={!status.last_pass.error_class} class="dot"></span>
          Last pass
        </dt>
        <dd>
          {status.last_pass.error_class
            ? "lexical fallback"
            : (status.last_pass.mode ?? "not run yet")}
        </dd>
        <!-- The line that would have made the 2026-08-30 degradation visible
             the day it happened: 525 rows were written lexical in one pass and
             nothing on the machine said which mode had answered. -->
        <small>
          {passTime(status.last_pass.at)} · {status.last_pass.considered} considered ·
          {status.last_pass.written} written{status.last_pass.error_class
            ? ` · ${status.last_pass.error_class}`
            : ""}
        </small>
      </div>
    {/if}
    {#if status.feedback_model}
      <div>
        <dt>
          <span class:online={status.feedback_model.active} class="dot"></span>
          Learned fit
        </dt>
        <dd>
          {status.feedback_model.active
            ? `AUC ${status.feedback_model.holdout.auc.toFixed(2)}`
            : "not yet learned"}
        </dd>
        <!-- The gate, and how far off it is. Of the live decisions, zero of the
             185 stored arXiv items has ever been kept — a model fitted on that
             learns "arXiv is never kept" and buries the largest source, which
             is why the factor stays at weight 0 until three measured conditions
             hold. -->
        <small>{status.feedback_model.gate_reason}</small>
      </div>
    {/if}
    {#if status.travel_context}
      <div>
        <dt>
          <span class:online={status.travel_context.upcoming_count > 0} class="dot"></span>
          Travel context
        </dt>
        <dd>{status.travel_context.upcoming_count} upcoming</dd>
        <small>
          {status.travel_context.refreshed_at
            ? status.travel_context.from_cache
              ? "stable snapshot"
              : "live from Trips"
            : "not compared yet"}
        </small>
      </div>
    {/if}
  </dl>
</section>

<style>
  .model-status {
    display: grid;
    grid-template-columns: minmax(12rem, 0.7fr) minmax(0, 2fr);
    align-items: center;
    gap: clamp(1.25rem, 4vw, 4rem);
    padding: 0.9rem 0;
    margin-bottom: 1rem;
    border-top: 1px solid var(--card-border);
    border-bottom: 1px solid var(--card-border);
  }

  .eyebrow {
    margin: 0 0 0.2rem;
    color: var(--primary);
    font-size: 0.5625rem;
    text-transform: uppercase;
    letter-spacing: 0.07em;
  }

  .intro strong {
    font-size: 0.75rem;
    font-weight: 580;
  }

  dl {
    display: grid;
    grid-template-columns: repeat(4, minmax(0, 1fr));
    gap: 1rem;
    margin: 0;
  }

  dl div {
    min-width: 0;
    padding-left: 0.75rem;
    border-left: 1px solid var(--card-border);
  }

  dt {
    display: flex;
    align-items: center;
    gap: 0.35rem;
    color: var(--text-tertiary);
    font-size: 0.5625rem;
  }

  dd {
    overflow: hidden;
    margin: 0.25rem 0 0;
    color: var(--text-primary);
    font-family: var(--font-mono);
    font-size: 0.625rem;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  small {
    display: block;
    margin-top: 0.15rem;
    color: var(--text-tertiary);
    font-size: 0.5rem;
  }

  .dot {
    width: 0.4rem;
    height: 0.4rem;
    border-radius: 50%;
    background: var(--warning);
  }

  .dot.online {
    background: var(--success);
  }

  @media (max-width: 52rem) {
    .model-status {
      grid-template-columns: 1fr;
      gap: 0.7rem;
    }
  }

  @media (max-width: 38rem) {
    dl {
      grid-template-columns: 1fr;
      gap: 0.55rem;
    }
  }
</style>
