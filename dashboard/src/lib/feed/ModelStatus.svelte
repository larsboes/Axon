<script lang="ts">
  import type { CommsEvaluationStatus } from "$lib/api";

  let { status }: { status: CommsEvaluationStatus } = $props();

  function shortModel(model: string): string {
    return model.split("/").at(-1) ?? model;
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
