<script lang="ts">
  /** What actually classified this mailbox, counted by the capability.
   *
   *  The panel this replaces hardcoded "Deterministic rules · local · no AI" and
   *  "Private rules first, generic heuristics second, then Active as the safe
   *  fallback", bound to nothing. With a second rung shipped, both sentences are
   *  false for any row a model wrote — and a dashboard that asserts something
   *  false about the row beside it is worse than one that says nothing. So the
   *  heading is the live distribution of `classification_method`, and the model
   *  rung's entry names its producer, its refusal and whether it may write. */
  import type { TriageClassifyReport } from "./api";

  let { report }: { report: TriageClassifyReport | null } = $props();

  const METHOD_LABEL: Record<string, string> = {
    deterministic: "deterministic rules",
    model: "the local model rung",
    human: "your own corrections",
    legacy: "rows classified before the rules existed",
  };

  // The distribution arrives counted and ordered from `GET
  // /triage/classify/report`. Frontend renders, backend computes: this used to
  // tally `classification_method` over the rows on screen, which is arithmetic.
  const methods = $derived(report?.by_classification_method ?? []);

  const heading = $derived(
    methods.length === 0
      ? "No mail classified yet"
      : methods.map((row) => `${row.n} by ${METHOD_LABEL[row.method] ?? row.method}`).join(" · ")
  );

  const verdicts = $derived(report?.verdicts ?? 0);
  const refused = $derived(
    report?.by_data_class.find((row) => row.data_class === "c3")?.n ?? 0
  );
</script>

<aside class="classifier card" aria-label="Mail classification method">
  <div>
    <p class="eyebrow mono">Current method</p>
    <h2>{heading}</h2>
  </div>
  <dl>
    <div>
      <dt>Rung 1 — rules</dt>
      <dd>
        Sender, subject, and whether List-Unsubscribe exists. Private rules first, generic
        heuristics second, then Active as the safe fallback. No model, no network.
      </dd>
    </div>
    <div>
      <dt>Rung 2 — local model</dt>
      <dd>
        {#if verdicts === 0}
          Not run yet. It only ever looks at the mail the rules left at the Active fallback, and it
          reads the sender's domain, the stored subject and the stored preview — never the address,
          never the message body.
        {:else}
          {verdicts} verdict(s) stored, from {report?.producer}. It only looks at the mail the rules
          left at the Active fallback, and it reads the sender's domain, the stored subject and the
          stored preview — never the address, never the message body.
        {/if}
      </dd>
    </div>
    <div>
      <dt>Secret mail</dt>
      <dd>
        Refused before a prompt is built and before any model is woken, by the same gate the labels
        come from{#if refused > 0}. {refused} thread(s) here carry a stored refusal rather than an
        absence, so "never asked" and "asked and refused" stay distinguishable{/if}.
      </dd>
    </div>
    <div>
      <dt>Shadow by default</dt>
      <dd>
        {#if report?.apply_enabled}
          Writing is enabled. A proposal that would raise a mail's data class is still held for you:
          the class change and the redaction that follows it cannot be undone.
        {:else}
          The model rung writes verdicts and moves no category. Turning that on is a key you set in
          the overlay after reading the measurement, not a button here.
        {/if}
      </dd>
    </div>
    <div>
      <dt>Urgency</dt>
      <dd>
        Measured and shown, and it ranks nothing.
        {#if report && !report.urgency_bp.validated}
          The frozen corpus has no measured error bound for it yet, so no list is ordered by it.
        {/if}
      </dd>
    </div>
    <div>
      <dt>Relevance inputs</dt>
      <dd>Sender, subject, and Gmail snippet compared with configured TELOS lenses.</dd>
    </div>
    <div>
      <dt>Relevance method</dt>
      <dd>
        Loopback embedding and reranking only; unavailable local models fall back to labelled
        lexical similarity.
      </dd>
    </div>
    <div>
      <dt>Never sent</dt>
      <dd>
        Message bodies and attachments are not fetched. Mail scoring and the model rung both reject
        non-loopback model endpoints.
      </dd>
    </div>
    <div>
      <dt>TELOS boundary</dt>
      <dd>Scoring reads TELOS. Categories and bulk decisions never rewrite TELOS files.</dd>
    </div>
    <div>
      <dt>Corrections</dt>
      <dd>
        A category you set here becomes a human override and survives later sweeps — including a
        model pass. A rule that later fires can take a model row back; nothing takes yours.
      </dd>
    </div>
    <div>
      <dt>Data classes</dt>
      <dd>
        Public may use approved cloud roles; Mine needs a reviewed pseudonymized derivative; Others
        and Secret never reach a cloud model, refused by the derivative builder, the tier check, the
        dispatch re-check against the row's current class, and the database constraint alike. Secret
        is refused local prompts too, by the same gate the labels are derived from — nothing
        summarizes, diagrams, charts or classifies it.
      </dd>
    </div>
  </dl>
</aside>

<style>
  .classifier {
    padding: 1rem;
    margin-bottom: 1rem;
  }

  .classifier h2 {
    margin: 0.15rem 0 0.85rem;
    color: var(--text-primary);
    font-size: 0.9rem;
  }

  .classifier dl {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(13rem, 1fr));
    gap: 0.85rem 1.25rem;
    margin: 0;
  }

  .classifier dt {
    color: var(--text-tertiary);
    font-family: var(--font-mono);
    font-size: 0.625rem;
    text-transform: uppercase;
  }

  .classifier dd {
    margin: 0.2rem 0 0;
    color: var(--text-secondary);
    font-size: var(--text-xs);
    line-height: 1.45;
  }

  .eyebrow {
    color: var(--text-tertiary);
    font-family: var(--font-mono);
    font-size: 0.625rem;
    letter-spacing: 0.06em;
    text-transform: uppercase;
  }
</style>
