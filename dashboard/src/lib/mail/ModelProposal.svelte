<script lang="ts">
  /** The model rung's verdict beside the rule's, on one mail proposal card.
   *
   *  Accepting is the EXISTING human override path — `comms.setTriageCategory`,
   *  which is `POST /triage/:id/stream` and writes `method='human'`. There is no
   *  new write route on this card, and that is the point: a machine proposes, a
   *  person decides, and the decision lands where every other manual correction
   *  already lands. */
  import { comms, type MailCategory, type TriageItem } from "$lib/api";
  import { proposesAChange } from "./api";

  let { item, label, onaccept }: {
    item: TriageItem;
    label: (category: MailCategory) => string;
    onaccept?: () => void;
  } = $props();

  const verdict = $derived(item.model ?? null);
  const changes = $derived(proposesAChange(verdict));
  /** A held verdict is one apply refused because it would raise the data class.
   *  Accepting it is a one-way decision on two axes, so the button says so. */
  const held = $derived(verdict?.mode === "held");

  /** What to render when the model produced no proposal. A state, not an error:
   *  a refused mail and a mail nobody has asked about yet look identical without
   *  this line, and only one of them will ever change. */
  const STATE_TEXT: Record<string, string> = {
    local_refused: "Refused: a Secret mail never enters a prompt, local models included.",
    skipped_over_window: "Skipped: too long for the local model's window. No larger model was woken.",
    unconfigured: "No local model is configured for this rung on this machine.",
    invalid_stream: "The model named a category that does not exist. Nothing was stored.",
    unparseable: "The model's answer could not be read as JSON. It will be asked again.",
    remote_refused: "Refused: the resolved model was not on this machine.",
  };

  let busy = $state(false);
  let error = $state<string | null>(null);

  async function accept() {
    if (!verdict?.model_stream) return;
    busy = true;
    error = null;
    try {
      await comms.setTriageCategory(item.id, verdict.model_stream);
      onaccept?.();
    } catch (failure) {
      error = failure instanceof Error ? failure.message : "Could not set the category";
    } finally {
      busy = false;
    }
  }
</script>

{#if verdict}
  <div class="model-proposal" aria-label="Local model proposal">
    <div class="verdicts">
      <div class="verdict">
        <p class="eyebrow mono">The rules</p>
        <p class="stream">{label(item.stream)}</p>
        <p class="why">{item.rationale}</p>
        <p class="provenance mono">{item.classification_method} · {item.classification_version}</p>
      </div>

      <p class="relation mono" class:agrees={!changes}>
        {#if verdict.state !== "generated"}—{:else if changes}Proposes {label(verdict.model_stream!)}{:else}Agrees{/if}
      </p>

      <div class="verdict">
        <p class="eyebrow mono">The local model</p>
        {#if verdict.state === "generated" && verdict.model_stream}
          <p class="stream">{label(verdict.model_stream)}</p>
          <p class="why">{verdict.rationale ?? "No rationale was returned."}</p>
          <p class="provenance mono">
            model · {verdict.classification_version}
            {#if verdict.confidence_bp !== null}
              · {(verdict.confidence_bp / 100).toFixed(0)}% confident (self-reported, uncalibrated)
            {/if}
          </p>
        {:else}
          <p class="state">{STATE_TEXT[verdict.state] ?? `State: ${verdict.state}`}</p>
        {/if}
      </div>
    </div>

    {#if verdict.state === "generated" && verdict.urgency_bp !== null}
      <div class="urgency">
        <span class="eyebrow mono">Urgency</span>
        <span class="bar" role="img" aria-label={`Urgency ${(verdict.urgency_bp / 100).toFixed(0)} of 100`}>
          <span class="fill" style:width={`${verdict.urgency_bp / 100}%`}></span>
        </span>
        <span class="urgency-why">{verdict.urgency_rationale ?? ""}</span>
        {#if !verdict.urgency_validated}
          <span class="unvalidated mono">not yet validated — nothing is ranked on this</span>
        {/if}
      </div>
    {/if}

    {#if held && verdict.held_reason}
      <p class="held">Held for you: {verdict.held_reason}. A machine may not make that move.</p>
    {/if}

    {#if changes}
      <div class="actions">
        <button class="btn" disabled={busy} onclick={accept}>
          {#if busy}
            Working…
          {:else if held}
            Accept {label(verdict.model_stream!)} — this also raises the data class and permanently
            redacts the subject
          {:else}
            Accept {label(verdict.model_stream!)}
          {/if}
        </button>
        <span class="note mono">Accepting records it as your decision, and later sweeps keep it.</span>
      </div>
    {/if}
    {#if error}<p class="error">{error}</p>{/if}
  </div>
{/if}

<style>
  .model-proposal {
    border-top: 1px solid var(--card-border);
    margin-top: 0.75rem;
    padding-top: 0.75rem;
    display: grid;
    gap: 0.6rem;
  }
  .verdicts {
    display: grid;
    grid-template-columns: 1fr auto 1fr;
    gap: 0.75rem;
    align-items: start;
  }
  .verdict {
    display: grid;
    gap: 0.2rem;
    min-width: 0;
  }
  .eyebrow {
    color: var(--text-tertiary);
    font-family: var(--font-mono);
    font-size: 0.7rem;
    letter-spacing: 0.06em;
    text-transform: uppercase;
  }
  .stream {
    color: var(--text-primary);
    font-weight: 600;
  }
  .why,
  .state {
    color: var(--text-secondary);
    font-size: 0.85rem;
  }
  .provenance {
    color: var(--text-tertiary);
    font-size: 0.7rem;
  }
  .relation {
    align-self: center;
    border: 1px solid var(--card-border);
    border-radius: var(--radius-sm);
    padding: 0.15rem 0.5rem;
    font-size: 0.72rem;
    white-space: nowrap;
  }
  .relation.agrees {
    color: var(--text-tertiary);
  }
  .urgency {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    flex-wrap: wrap;
    font-size: 0.78rem;
  }
  .bar {
    display: inline-block;
    width: 6rem;
    height: 0.4rem;
    border: 1px solid var(--card-border);
    border-radius: 999px;
    overflow: hidden;
  }
  .fill {
    display: block;
    height: 100%;
    background: var(--accent);
  }
  .urgency-why,
  .unvalidated,
  .note {
    color: var(--text-tertiary);
  }
  .unvalidated,
  .note {
    font-size: 0.7rem;
  }
  .held {
    color: var(--text-secondary);
    font-size: 0.82rem;
  }
  .actions {
    display: flex;
    align-items: center;
    gap: 0.6rem;
    flex-wrap: wrap;
  }
  .error {
    color: var(--danger);
    font-size: 0.8rem;
  }
</style>
