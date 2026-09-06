<script lang="ts">
  import { link } from "$lib/nav";
  import FactorBars, { type Factor } from "$lib/FactorBars.svelte";
  import type { FeedEvaluation } from "$lib/api";

  /**
   * Why this item scored what it scored.
   *
   * Two changes from what this drew before. The compact grid is `auto-fit` rather than a
   * hardcoded four columns against a loop over however many factors the evaluator
   * published — a fifth factor wrapped into a ragged second row. And the travel factor no
   * longer wears `--success`: a reserved status colour used as series identity, and the
   * context link that was supposed to carry the meaning instead renders only in the
   * non-compact form, so on a card the distinction was colour and nothing else. It is a
   * `map-pin` mark now, which also survives a reader who cannot separate the two hues.
   */
  let {
    evaluation,
    compact = false,
  }: {
    evaluation: FeedEvaluation;
    compact?: boolean;
  } = $props();

  const percentage = $derived(Math.round(evaluation.overall_score * 100));
  const modeLabel = $derived(
    evaluation.mode === "reranked"
      ? "reranked"
      : evaluation.mode === "semantic"
        ? "semantic"
        : evaluation.mode === "lexical"
          ? "lexical"
          : "without TELOS",
  );

  const factors = $derived<Factor[]>(
    evaluation.factors.map((factor) => ({
      key: factor.key,
      label: factor.label,
      score: factor.score,
      weight: factor.weight,
      mark: factor.context?.kind === "trip" ? "map-pin" : undefined,
      rationale: factor.rationale,
      // The href is /travel, so only a trip context earns one. `kind` is typed open
      // (`'trip' | string`) and the evaluator emits only "trip" today, so a second kind
      // would otherwise arrive silently linked to the wrong page. Its label still renders.
      context: factor.context
        ? {
            label: factor.context.label,
            href: factor.context.kind === "trip" ? link("/travel") : undefined,
            terms: factor.context.matched_terms,
          }
        : null,
    })),
  );
</script>

<div class="evaluation" class:compact>
  <div class="head">
    <div class="figure">
      <span class="score mono">{percentage}</span>
      <span class="unit mono">/100</span>
    </div>
    <div class="title">
      <strong>Explained ranking</strong>
      <span>{modeLabel} · {evaluation.evaluator_revision}</span>
    </div>
  </div>

  <FactorBars {factors} {compact} weighted>
    {#snippet detail(factor)}
      {#if factor.rationale}
        <p class="rationale">{factor.rationale} · weight {Math.round(factor.weight * 100)}%</p>
      {/if}
      {#if factor.context}
        <!-- A link only where there is somewhere to go. A context with no href still
             names itself; it just does not pretend to be a destination. -->
        <svelte:element
          this={factor.context.href ? "a" : "span"}
          class="context"
          href={factor.context.href}
        >
          {factor.context.label}
          {#if factor.context.terms?.length}· {factor.context.terms.join(", ")}{/if}
        </svelte:element>
      {/if}
    {/snippet}
  </FactorBars>

  {#if !compact}
    <p class="explanation">{evaluation.explanation}</p>
  {/if}
</div>

<style>
  .evaluation {
    min-width: 0;
  }

  .head {
    display: flex;
    align-items: baseline;
    gap: var(--space-3);
    margin-bottom: var(--space-4);
  }

  .figure {
    white-space: nowrap;
  }

  /* Proportional figures, not tabular: a standalone value at display size looks loose
     when every digit is given the width of a zero. */
  .score {
    color: var(--primary);
    font-size: var(--text-lg);
    font-weight: 620;
  }

  .unit {
    color: var(--text-tertiary);
    font-size: var(--text-2xs);
  }

  .title {
    display: flex;
    min-width: 0;
    flex-direction: column;
  }

  .title strong {
    color: var(--text-primary);
    font-size: var(--text-xs);
    font-weight: 600;
  }

  .title span {
    overflow: hidden;
    color: var(--text-tertiary);
    font-size: var(--text-2xs);
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .rationale {
    margin: var(--space-1) 0 0;
    color: var(--text-tertiary);
    font-size: var(--text-2xs);
    line-height: var(--leading-normal);
  }

  .context {
    display: inline-block;
    /* A non-trip context renders as a <span>, so the colour carries the affordance and
       the shared link rules do not. */
    margin-top: var(--space-1);
    color: var(--primary);
    font-size: var(--text-2xs);
  }

  .explanation {
    margin: var(--space-4) 0 0;
    padding-top: var(--space-4);
    border-top: 1px solid var(--card-border);
    color: var(--text-secondary);
    font-size: var(--text-2xs);
    line-height: var(--leading-normal);
  }

  .compact .head {
    margin-bottom: var(--space-3);
  }

  .compact .score {
    font-size: var(--text-base);
  }

  .compact .title strong {
    font-size: var(--text-2xs);
  }

  .compact .title span,
  .compact .unit {
    font-size: 0.5rem;
  }
</style>
