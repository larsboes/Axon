<script lang="ts">
  import Icon from "$lib/Icon.svelte";
  import { saveRetrospective, type Retrospective, type RetrospectiveBody } from "$lib/travel/api";

  /**
   * Exactly three fields, matching the three-key body: what it cost, would I do
   * it again, what would I change.
   *
   * The cost is denominated in the PLAN's currency, which the form displays and
   * never asks for; a plan with no currency cannot record a cost, and the form
   * says so rather than letting the server refuse the submit.
   *
   * The prefill is one named quantity and the label says which one. Machine
   * proposes, human confirms — and a proposal has to be a named quantity in a
   * named unit.
   */
  let {
    planId,
    currency,
    existing = null,
    proposal = null,
    onSaved,
  }: {
    planId: string;
    currency: string | null;
    existing?: Retrospective | null;
    /** A named quantity in the plan's currency, or null for "no proposal". */
    proposal?: { cents: number; label: string } | null;
    onSaved?: (row: Retrospective) => void;
  } = $props();

  const AGAIN_OPTIONS: Array<{ id: RetrospectiveBody["again"]; label: string }> = [
    { id: "yes", label: "Yes" },
    { id: "maybe", label: "Maybe" },
    { id: "no", label: "No" },
  ];

  const centsToAmount = (cents: number | null): number | null =>
    cents === null ? null : cents / 100;

  let initializedFor = $state("");
  // A NUMBER, because `bind:value` on `<input type="number">` coerces to
  // `number | null` (svelte's `is_numberlike_input` -> `to_number`), and an
  // empty field is null rather than "". Held as a string this threw
  // `cost.trim is not a function` the moment the box was touched, and the
  // retrospective could not be filed at all.
  let cost = $state<number | null>(null);
  let again = $state<RetrospectiveBody["again"]>("maybe");
  let changeNote = $state("");
  let saving = $state(false);
  let error = $state<string | null>(null);
  /** Set on the first keystroke in the cost box. A proposal never overwrites a
   *  figure a human has already touched. */
  let costTouched = $state(false);

  $effect(() => {
    const proposed = centsToAmount(existing?.cost_cents ?? proposal?.cents ?? null);
    if (initializedFor !== planId) {
      initializedFor = planId;
      costTouched = false;
      cost = proposed;
      again = existing?.again ?? "maybe";
      changeNote = existing?.change_note ?? "";
      error = null;
      return;
    }
    // The proposal arrives AFTER this form mounts — the cost roll-up is a second
    // request the page fires beside the plan read — so the prefill has to be
    // applied when it lands, not only when the plan changes.
    if (!costTouched) cost = proposed;
  });

  const costHint = $derived(
    currency === null
      ? "This plan carries no currency, so a cost cannot be recorded. Set one in the editor first."
      : existing
        ? `In ${currency}`
        : proposal
          ? `In ${currency} · proposed from ${proposal.label}`
          : `In ${currency} · no proposal`,
  );

  async function submit(): Promise<void> {
    // `!(cost >= 0)` and not `cost < 0`: a half-typed entry reaches this as NaN.
    if (cost !== null && !(cost >= 0)) {
      error = "A cost is a number, and never negative.";
      return;
    }
    saving = true;
    error = null;
    try {
      const row = await saveRetrospective(planId, {
        // Euros in, integer minor units out. A price in floating point is a
        // price that eventually disagrees with the receipt.
        cost_cents: cost === null ? null : Math.round(cost * 100),
        again,
        change_note: changeNote.trim(),
      });
      onSaved?.(row);
    } catch (caught) {
      error = caught instanceof Error ? caught.message : String(caught);
    } finally {
      saving = false;
    }
  }
</script>

<section class="retrospective card" aria-labelledby="retrospective-title">
  <header>
    <span class="eyebrow">Trip closed</span>
    <h3 id="retrospective-title">
      {existing ? "Your retrospective" : "How did it go?"}
    </h3>
  </header>

  {#if error}
    <p class="error" aria-live="polite">{error}</p>
  {/if}

  <form
    onsubmit={(event) => {
      event.preventDefault();
      void submit();
    }}
  >
    <label>
      <span>What it cost</span>
      <input
        class="input"
        type="number"
        step="0.01"
        min="0"
        inputmode="decimal"
        bind:value={cost}
        oninput={() => (costTouched = true)}
        disabled={currency === null}
        placeholder={currency === null ? "—" : "0.00"}
      />
      <small>{costHint}</small>
    </label>

    <fieldset>
      <legend>Would I do it again</legend>
      <div class="again-list">
        {#each AGAIN_OPTIONS as option (option.id)}
          <button
            type="button"
            class:active={again === option.id}
            aria-pressed={again === option.id}
            onclick={() => (again = option.id)}
          >
            {option.label}
          </button>
        {/each}
      </div>
    </fieldset>

    <label>
      <span>What I would change</span>
      <textarea class="input" rows="3" bind:value={changeNote}></textarea>
    </label>

    <footer>
      <button class="save" type="submit" disabled={saving}>
        {#if saving}<Icon name="loader" size={13} />{/if}
        {saving ? "Saving…" : existing ? "Update" : "Record"}
      </button>
      {#if existing}
        <small>Recorded once already; saving again corrects it.</small>
      {/if}
    </footer>
  </form>
</section>

<style>
  .retrospective {
    display: grid;
    gap: 0.75rem;
    padding: 1rem;
  }

  header h3 {
    margin: 0.15rem 0 0;
    font-size: 1rem;
  }

  form {
    display: grid;
    gap: 0.75rem;
  }

  label {
    display: grid;
    gap: 0.3rem;
    font-size: 0.8rem;
    color: var(--text-secondary);
  }

  label small {
    font-size: 0.7rem;
    color: var(--text-tertiary);
  }

  fieldset {
    border: 0;
    margin: 0;
    padding: 0;
    display: grid;
    gap: 0.3rem;
  }

  legend {
    padding: 0;
    font-size: 0.8rem;
    color: var(--text-secondary);
  }

  .again-list {
    display: flex;
    gap: 0.4rem;
  }

  .again-list button {
    padding: 0.35rem 0.8rem;
    border: 1px solid var(--input-border);
    border-radius: var(--radius-sm);
    background: var(--card-bg);
    color: var(--text-secondary);
    cursor: pointer;
  }

  .again-list button.active {
    border-color: var(--primary);
    background: var(--primary-soft);
    color: var(--text-primary);
  }

  footer {
    display: flex;
    align-items: center;
    gap: 0.6rem;
  }

  footer small {
    font-size: 0.7rem;
    color: var(--text-tertiary);
  }

  .save {
    display: inline-flex;
    align-items: center;
    gap: 0.35rem;
    padding: 0.4rem 0.9rem;
    border: 0;
    border-radius: var(--radius-sm);
    background: var(--primary);
    color: var(--text-inverse);
    cursor: pointer;
  }

  .save:disabled {
    opacity: 0.6;
    cursor: default;
  }

  .error {
    margin: 0;
    font-size: 0.8rem;
    color: var(--danger);
  }
</style>
