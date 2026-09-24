<script lang="ts">
  import Icon from "$lib/Icon.svelte";
  import PlaceField from "$lib/travel/PlaceField.svelte";
  import {
    trips,
    type BaseCandidate,
    type BaseResult,
    type PlaceRef,
  } from "$lib/api";

  let {
    initialFrom = null,
    initialAnchor = null,
    onAdopt,
  }: {
    initialFrom?: PlaceRef | null;
    initialAnchor?: PlaceRef | null;
    onAdopt: (base: BaseCandidate, from: PlaceRef, anchor: PlaceRef, fromDate: string, anchorDate: string) => Promise<void>;
  } = $props();

  const isoDate = (d: Date) =>
    [
      d.getFullYear(),
      String(d.getMonth() + 1).padStart(2, "0"),
      String(d.getDate()).padStart(2, "0"),
    ].join("-");

  const today = new Date();
  const nextWeek = new Date(today);
  nextWeek.setDate(today.getDate() + 7);
  const twoWeeks = new Date(today);
  twoWeeks.setDate(today.getDate() + 14);

  // svelte-ignore state_referenced_locally
  let from = $state<PlaceRef | null>(initialFrom);
  // svelte-ignore state_referenced_locally
  let anchor = $state<PlaceRef | null>(initialAnchor);
  let fromDate = $state(isoDate(nextWeek));
  let anchorDate = $state(isoDate(twoWeeks));
  let maxCandidates = $state(5);

  let searching = $state(false);
  let adopting = $state<string | null>(null);
  let error = $state<string | null>(null);
  let result = $state<BaseResult | null>(null);

  async function searchBases(): Promise<void> {
    if (!from || !anchor) {
      error = "Choose both a starting location and an anchor destination.";
      return;
    }

    if (fromDate >= anchorDate) {
      error = "Anchor date must be after the start date.";
      return;
    }

    searching = true;
    error = null;
    result = null;

    try {
      result = await trips.bases({
        from,
        anchor,
        from_date: fromDate,
        anchor_date: anchorDate,
        max_candidates: maxCandidates,
      });
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      searching = false;
    }
  }

  async function handleAdopt(candidate: BaseCandidate): Promise<void> {
    if (!from || !anchor) return;
    adopting = candidate.place_id;
    error = null;
    try {
      await onAdopt(candidate, from, anchor, fromDate, anchorDate);
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      adopting = null;
    }
  }

  function formatMoney(cents: number | null): string {
    if (cents === null) return "—";
    return `${(cents / 100).toFixed(2)} €`;
  }
</script>

<div class="base-finder">
  <p class="lede">
    Have days between being in one place and needing to be in another? Find intermediate
    hubs ranked by combined outbound and onward travel costs, with nearby companions noted.
  </p>

  {#if error}
    <p class="notice error"><Icon name="alert" size={15} /> {error}</p>
  {/if}

  <form
    class="finder-form"
    onsubmit={(e) => {
      e.preventDefault();
      void searchBases();
    }}
  >
    <div class="places-row">
      <PlaceField label="Starting from" placeholder="e.g. Berlin" bind:place={from} />
      <PlaceField label="Must end at (Anchor)" placeholder="e.g. Hamburg" bind:place={anchor} />
    </div>

    <div class="dates-row">
      <label>
        <span>Stay begins</span>
        <input class="input" type="date" bind:value={fromDate} />
      </label>
      <label>
        <span>Leave for anchor</span>
        <input class="input" type="date" min={fromDate} bind:value={anchorDate} />
      </label>
      <label class="limit-col">
        <span>Probes</span>
        <select class="input" bind:value={maxCandidates}>
          <option value={3}>3 bases</option>
          <option value={5}>5 bases</option>
          <option value={8}>8 bases</option>
          <option value={10}>10 bases</option>
        </select>
      </label>
      <button class="btn btn-primary search-btn" type="submit" disabled={searching}>
        {#if searching}
          <Icon name="loader" size={14} /> Pricing…
        {:else}
          <Icon name="compass" size={14} /> Find bases
        {/if}
      </button>
    </div>
  </form>

  {#if searching}
    <div class="loading-state">
      <Icon name="loader" size={20} /> Searching transit routes and companion overlap…
    </div>
  {:else if result}
    <div class="results-container">
      <header class="results-header">
        <div>
          <h3>Intermediate Base Candidates</h3>
          <p>
            Between <strong>{result.from}</strong> ({result.from_date}) and
            <strong>{result.anchor}</strong> ({result.anchor_date})
          </p>
        </div>
        <span class="meta-count">
          {result.priced} of {result.considered} priced
        </span>
      </header>

      {#if result.candidates.length === 0}
        <p class="empty-state">No candidate bases found between these two points.</p>
      {:else}
        <ol class="candidates-list">
          {#each result.candidates as cand (cand.place_id)}
            <li class="candidate-card card">
              <div class="card-main">
                <div class="title-row">
                  <h4>{cand.name}</h4>
                  <div class="fare-total">
                    <span class="fare-label">Total travel</span>
                    <strong class="fare-amt">{formatMoney(cand.travel_cents)}</strong>
                  </div>
                </div>

                <div class="legs-breakdown">
                  <span class="leg-pill">
                    {result.from} → {cand.name}: {formatMoney(cand.reach?.cents ?? null)}
                  </span>
                  <span class="arrow">·</span>
                  <span class="leg-pill">
                    {cand.name} → {result.anchor}: {formatMoney(cand.onward?.cents ?? null)}
                  </span>
                </div>

                {#if cand.known_companions && cand.known_companions > 0}
                  <p class="companion-tag">
                    <Icon name="compass" size={13} />
                    {cand.known_companions} companion{cand.known_companions === 1 ? "" : "s"} nearby
                    {#if cand.overlap_days}({cand.overlap_days} days overlap){/if}
                  </p>
                {/if}

                {#if cand.why.length > 0}
                  <ul class="why-list">
                    {#each cand.why as reason, i (i)}
                      <li>{reason}</li>
                    {/each}
                  </ul>
                {/if}
              </div>

              <div class="card-action">
                <button
                  class="btn btn-outline btn-sm adopt-btn"
                  type="button"
                  disabled={adopting !== null}
                  onclick={() => void handleAdopt(cand)}
                >
                  {#if adopting === cand.place_id}
                    <Icon name="loader" size={13} /> Creating…
                  {:else}
                    <Icon name="plus" size={13} /> Base trip here
                  {/if}
                </button>
              </div>
            </li>
          {/each}
        </ol>
      {/if}

      {#if result.degraded.length > 0}
        <div class="degraded-notes">
          <Icon name="alert" size={13} />
          <ul>
            {#each result.degraded as msg, i (i)}
              <li>{msg}</li>
            {/each}
          </ul>
        </div>
      {/if}
    </div>
  {/if}
</div>

<style>
  .base-finder {
    display: flex;
    flex-direction: column;
    gap: 1.25rem;
  }

  .lede {
    margin: 0;
    color: var(--text-secondary);
    font-size: var(--text-sm);
  }

  .notice {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.65rem 0.85rem;
    border-radius: var(--radius-md);
    font-size: var(--text-sm);
  }

  .notice.error {
    background: var(--danger-soft);
    color: var(--danger);
  }

  .finder-form {
    display: flex;
    flex-direction: column;
    gap: 0.85rem;
    padding: 1rem;
    border: 1px solid var(--card-border);
    border-radius: var(--radius);
    background: var(--surface);
  }

  .places-row {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 0.75rem;
  }

  .dates-row {
    display: grid;
    grid-template-columns: 1fr 1fr 6rem auto;
    align-items: end;
    gap: 0.75rem;
  }

  label {
    display: flex;
    flex-direction: column;
    gap: 0.3rem;
    font-size: var(--text-2xs);
    font-weight: 600;
    color: var(--text-tertiary);
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }

  .search-btn {
    height: 2.25rem;
    padding: 0 1rem;
  }

  .loading-state,
  .empty-state {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 0.5rem;
    padding: 3rem 1rem;
    color: var(--text-tertiary);
    font-size: var(--text-sm);
  }

  .results-container {
    display: flex;
    flex-direction: column;
    gap: 1rem;
  }

  .results-header {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 1rem;
  }

  .results-header h3 {
    margin: 0;
    font-size: var(--text-base);
    font-weight: 600;
  }

  .results-header p {
    margin: 0.2rem 0 0;
    font-size: var(--text-xs);
    color: var(--text-secondary);
  }

  .meta-count {
    font-size: var(--text-xs);
    color: var(--text-tertiary);
    font-family: var(--font-mono, monospace);
  }

  .candidates-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
  }

  .candidate-card {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 1.25rem;
    padding: 0.9rem 1.1rem;
  }

  .card-main {
    flex: 1 1 auto;
    min-width: 0;
  }

  .title-row {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 1rem;
    margin-bottom: 0.3rem;
  }

  .title-row h4 {
    margin: 0;
    font-size: var(--text-base);
    font-weight: 600;
    color: var(--text-primary);
  }

  .fare-total {
    display: flex;
    align-items: baseline;
    gap: 0.4rem;
  }

  .fare-label {
    font-size: var(--text-2xs);
    color: var(--text-tertiary);
    text-transform: uppercase;
  }

  .fare-amt {
    font-family: var(--font-mono, monospace);
    font-size: var(--text-base);
    color: var(--primary);
  }

  .legs-breakdown {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    font-size: var(--text-xs);
    color: var(--text-secondary);
    margin-bottom: 0.4rem;
  }

  .leg-pill {
    font-family: var(--font-mono, monospace);
  }

  .companion-tag {
    display: inline-flex;
    align-items: center;
    gap: 0.35rem;
    margin: 0.2rem 0 0;
    padding: 0.15rem 0.45rem;
    border-radius: var(--radius-sm);
    background: var(--primary-soft);
    color: var(--primary);
    font-size: var(--text-xs);
    font-weight: 500;
  }

  .why-list {
    margin: 0.4rem 0 0;
    padding-left: 1.2rem;
    font-size: var(--text-xs);
    color: var(--text-tertiary);
  }

  .degraded-notes {
    display: flex;
    gap: 0.5rem;
    padding: 0.75rem 1rem;
    border-radius: var(--radius);
    background: var(--card-bg);
    border: 1px solid var(--card-border);
    font-size: var(--text-xs);
    color: var(--text-tertiary);
  }

  .degraded-notes ul {
    margin: 0;
    padding-left: 1rem;
  }

  .adopt-btn {
    white-space: nowrap;
  }
</style>
