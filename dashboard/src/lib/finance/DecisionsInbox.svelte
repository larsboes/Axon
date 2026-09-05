<script lang="ts">
  import { ApiError } from "$lib/api";
  import { recordVerdict, runDecisions, type Decision } from "$lib/finance/invest-api";

  let {
    decisions,
    onchanged,
  }: { decisions: Decision[]; onchanged: () => void | Promise<void> } = $props();

  let busy = $state<string | null>(null);
  let notes = $state<Record<string, string>>({});
  let failures = $state<Record<string, string>>({});
  let moved = $state<Record<string, boolean>>({});
  let runError = $state<string | null>(null);

  async function verdict(decision: Decision, value: "accepted" | "rejected") {
    const note = (notes[decision.id] ?? "").trim();
    if (value === "rejected" && note === "") {
      failures = { ...failures, [decision.id]: "A note is required to reject a proposal." };
      return;
    }
    busy = decision.id;
    failures = { ...failures, [decision.id]: "" };
    try {
      await recordVerdict(decision.id, {
        expected_proposal_id: decision.id,
        verdict: value,
        note,
      });
      await onchanged();
    } catch (cause) {
      // A 409 is not an error the reader caused: the numbers moved under them
      // and the row they are looking at is stale. It gets its own affordance
      // rather than a red message they cannot act on.
      if (cause instanceof ApiError && cause.status === 409) {
        moved = { ...moved, [decision.id]: true };
      } else {
        failures = {
          ...failures,
          [decision.id]: cause instanceof Error ? cause.message : String(cause),
        };
      }
    } finally {
      busy = null;
    }
  }

  async function recompute() {
    busy = "run";
    runError = null;
    try {
      await runDecisions(false);
      await onchanged();
    } catch (cause) {
      runError = cause instanceof Error ? cause.message : String(cause);
    } finally {
      busy = null;
    }
  }

  const money = (cents: number, currency: string) =>
    new Intl.NumberFormat("de-DE", { style: "currency", currency }).format(cents / 100);
  const signedPercent = (bp: number) => `${bp > 0 ? "+" : ""}${(bp / 100).toFixed(2)}%`;
</script>

<!-- Renders nothing at all when nothing is open. Not an empty list, not a zero,
     not "all clear": a quiet inbox is the strongest signal this surface can send,
     and a placeholder destroys it. -->
{#if decisions.length > 0}
  <section class="inbox">
    <div class="heading">
      <div>
        <h2>Decisions awaiting a call</h2>
        <p>
          Accepting records a decision and moves no money: no journal entry, no holdings
          snapshot, no order. The record is appended, so what you decided and when stays
          answerable a year from now.
        </p>
      </div>
      <button onclick={recompute} disabled={busy !== null}>Recompute</button>
    </div>
    {#if runError}<p class="failure">{runError}</p>{/if}

    <ul>
      {#each decisions as decision (decision.id)}
        <li>
          <div class="row">
            <div class="what">
              <span class="kind">{decision.kind}</span>
              <strong>{decision.proposal.title}</strong>
              <p>{decision.proposal.summary}</p>
              <dl>
                {#each Object.entries(decision.evidence.numbers) as [name, value] (name)}
                  <div><dt>{name.replaceAll("_", " ")}</dt><dd>{value}</dd></div>
                {/each}
              </dl>
              {#if decision.proposal.drift_bp !== null}
                <p class="drift">
                  Drift {signedPercent(decision.proposal.drift_bp)} against a target of
                  {((decision.proposal.target_bp ?? 0) / 100).toFixed(2)}% with a band of
                  ±{((decision.proposal.band_bp ?? 0) / 100).toFixed(2)}%.
                </p>
              {:else if decision.proposal.amount_cents !== null}
                <p class="drift">{money(decision.proposal.amount_cents, decision.proposal.currency)}</p>
              {/if}
              {#if decision.evidence.feed_items.length > 0}
                <div class="reading">
                  <!-- Labelled as a pointer, never as a reason. The match is a
                       token match on the instrument's configured label; it can be
                       wrong, and presenting it as evidence for the decision would
                       overstate what it is. -->
                  <span class="meta">Possibly worth reading — a pointer, not a reason:</span>
                  <ul class="links">
                    {#each decision.evidence.feed_items as item (item.id)}
                      <li><a href={item.url} rel="noreferrer noopener" target="_blank">{item.title}</a> <span class="meta">{item.day}</span></li>
                    {/each}
                  </ul>
                </div>
              {/if}
              {#if decision.evidence.risk}
                <p class="meta">
                  {#if decision.evidence.risk.portfolio_volatility_bp !== null}
                    Portfolio volatility {(decision.evidence.risk.portfolio_volatility_bp / 100).toFixed(2)}% annualised.
                  {:else}
                    No risk figures: not enough price history yet
                    ({decision.evidence.risk.min_observations} daily observations are needed per instrument).
                  {/if}
                </p>
              {/if}
              <span class="meta">
                Proposed {decision.proposed_at} · rung {decision.rung} · {decision.model_revision} · class {decision.data_class}
              </span>
            </div>

            <div class="act">
              {#if moved[decision.id]}
                <p class="moved">The numbers moved since this was proposed.</p>
                <button onclick={onchanged}>Reload</button>
              {:else}
                <label>
                  <span class="meta">Note (required to reject)</span>
                  <input
                    type="text"
                    bind:value={notes[decision.id]}
                    placeholder="Why?"
                    disabled={busy !== null}
                  />
                </label>
                <div class="buttons">
                  <button
                    class="primary"
                    onclick={() => verdict(decision, "accepted")}
                    disabled={busy !== null}
                  >Accept</button>
                  <button onclick={() => verdict(decision, "rejected")} disabled={busy !== null}>Reject</button>
                </div>
              {/if}
              {#if failures[decision.id]}<p class="failure">{failures[decision.id]}</p>{/if}
            </div>
          </div>
        </li>
      {/each}
    </ul>
  </section>
{/if}

<style>
  .inbox { margin-top: .75rem; padding: .9rem; border: 1px solid var(--card-border); border-radius: var(--radius-md); background: var(--card-bg); }
  .heading { display: flex; align-items: start; justify-content: space-between; gap: 1rem; }
  h2 { margin: 0; font-size: .85rem; }
  p { margin: .2rem 0 0; color: var(--text-secondary); font-size: .7rem; }
  ul { margin: .75rem 0 0; padding: 0; list-style: none; }
  li + li { margin-top: .55rem; }
  .row { display: flex; gap: 1rem; align-items: start; justify-content: space-between; padding: .65rem; border: 1px solid var(--card-border); border-radius: var(--radius-sm); }
  .what { min-width: 0; }
  .kind { display: inline-block; margin-bottom: .2rem; padding: 0 .3rem; border-radius: var(--radius-sm); background: var(--primary-soft); color: var(--primary); font-size: .66rem; }
  strong { display: block; font-size: .82rem; }
  dl { display: flex; flex-wrap: wrap; gap: .3rem .8rem; margin: .4rem 0 0; font-size: .68rem; }
  dl > div { display: flex; gap: .3rem; }
  dt { color: var(--text-tertiary); }
  dd { margin: 0; font-variant-numeric: tabular-nums; }
  .drift { font-variant-numeric: tabular-nums; }
  .reading { margin-top: .4rem; }
  .links { margin: .2rem 0 0; padding-left: .9rem; list-style: disc; font-size: .7rem; }
  .links li { margin: 0; }
  .meta { display: block; margin-top: .3rem; color: var(--text-tertiary); font-size: .66rem; }
  .act { display: flex; flex-direction: column; gap: .35rem; min-width: 12rem; }
  .act input { width: 100%; padding: .3rem .4rem; border: 1px solid var(--input-border); border-radius: var(--radius-sm); background: var(--input-bg); color: inherit; font: inherit; font-size: .72rem; }
  .buttons { display: flex; gap: .35rem; }
  button { padding: .3rem .6rem; border: 1px solid var(--card-border); border-radius: var(--radius-sm); background: var(--surface); color: inherit; font: inherit; font-size: .72rem; cursor: pointer; }
  button.primary { border-color: var(--primary); background: var(--primary); color: var(--text-inverse); }
  button:disabled { opacity: .55; cursor: default; }
  .failure { color: var(--danger); }
  .moved { color: var(--warning); }
</style>
