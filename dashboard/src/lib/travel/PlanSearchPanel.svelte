<script lang="ts">
  /**
   * "October, under 300 €, by train" — the form and the ranked answer.
   *
   * The page renders and does not compute: every number here comes from the
   * response, and the only arithmetic is formatting (`money`, `factorScorePercent`).
   * The score factors are shown rather than summarised, because a rank nobody
   * can inspect is a rank nobody can disagree with, and a factor that could not
   * be measured is ABSENT from the list rather than shown as a neutral middle.
   */
  import Icon from "$lib/Icon.svelte";
  import type { PlaceRef, TransportMode } from "$lib/api";
  import { ApiError } from "$lib/api";
  import { planSearch, type PlanSearchResult, type RankedCandidate } from "$lib/travel/api";
  import {
    coverageNotice,
    degradedNotice,
    factorScorePercent,
    money,
    pollJob,
    seedFromCandidate,
    unreachable,
  } from "$lib/travel/plan-search";

  let {
    origin,
    modeOptions,
    /** Hands a chosen candidate to the New-trip form. */
    onChoose,
    /**
     * Creates the plan and adopts the search onto it. Returns the plan id, or
     * throws. The panel never rolls a plan back: the operator asked for it.
     */
    onAdopt,
  }: {
    origin: PlaceRef | null;
    modeOptions: Array<{ id: TransportMode; label: string }>;
    onChoose: (seed: ReturnType<typeof seedFromCandidate>) => void;
    onAdopt: ((job: number, candidate: RankedCandidate) => Promise<string>) | null;
  } = $props();

  /**
   * Next month, not this one. A month search expands to that month's first and
   * last day, so opening the panel on the 20th and pressing Find would search a
   * window that mostly already happened.
   */
  const nextMonth = (() => {
    const now = new Date();
    return new Date(Date.UTC(now.getUTCFullYear(), now.getUTCMonth() + 1, 1))
      .toISOString()
      .slice(0, 7);
  })();

  /**
   * A month or an explicit window — the two shapes the route accepts, and it
   * refuses both at once. They are not the same search: a month with no
   * calendar FAILS, because a month has no dates until the calendar names the
   * feasible ones, while an explicit window degrades, reports
   * `window_source: "caller"` and drops the feasibility factor.
   */
  let windowMode = $state<"month" | "dates">("month");
  let month = $state(nextMonth);
  let dateFrom = $state("");
  let dateTo = $state("");
  let budgetEuros = $state<number | null>(300);
  let modes = $state<TransportMode[]>(["train"]);
  let interests = $state("");
  let maxCandidates = $state(8);

  let job = $state<number | null>(null);
  let running = $state(false);
  let sinceMs = $state(0);
  let result = $state<PlanSearchResult | null>(null);
  let error = $state<string | null>(null);
  /** Set when createPlan succeeded and adopt did not. The plan STAYS. */
  let adoptError = $state<string | null>(null);
  let adoptCandidate = $state<RankedCandidate | null>(null);
  let adopted = $state(false);

  const toggleMode = (mode: TransportMode) => {
    modes = modes.includes(mode) ? modes.filter((m) => m !== mode) : [...modes, mode];
  };

  async function run() {
    if (!origin) {
      error = "Pick an origin first — a fare needs somewhere to start.";
      return;
    }
    if (windowMode === "dates" && !(dateFrom && dateTo)) {
      error = "Give both dates, or search by month instead.";
      return;
    }
    running = true;
    error = null;
    adoptError = null;
    adopted = false;
    result = null;
    try {
      const started = await planSearch.start({
        origin,
        // Exactly one of the two; the route answers 400 for both or neither.
        ...(windowMode === "month" ? { month } : { date_window: { from: dateFrom, to: dateTo } }),
        budget_cents: budgetEuros === null ? undefined : Math.round(budgetEuros * 100),
        currency: "EUR",
        modes,
        interests,
        max_candidates: maxCandidates,
      });
      job = started.job;
      const outcome = await pollJob(started.job, {
        fetchStatus: (id) => planSearch.status(id),
        onUpdate: (answer) => {
          if (answer.state === "running") sinceMs = answer.since_ms;
        },
      });
      if (outcome.state === "done") result = outcome.result;
      else if (outcome.state === "failed" || outcome.state === "timeout") error = outcome.error;
    } catch (failure) {
      error = failure instanceof ApiError ? failure.message : String(failure);
    } finally {
      running = false;
    }
  }

  async function adopt(candidate: RankedCandidate) {
    if (job === null || !onAdopt) return;
    adoptCandidate = candidate;
    adoptError = null;
    try {
      await onAdopt(job, candidate);
      adopted = true;
    } catch (failure) {
      // The plan is not rolled back. Deleting a durable row the operator asked
      // for, to hide a transient error, is the wrong way round — the same
      // reasoning trips' own project_after_write gives for letting a plan write
      // succeed when the vault is unreachable. Adopt is idempotent, so Retry is
      // a correction and not a duplicate.
      adoptError =
        failure instanceof ApiError && failure.status === 404
          ? "That search result has expired — run the search again. The trip you created is already open."
          : failure instanceof ApiError
            ? failure.message
            : String(failure);
    }
  }

  const notice = $derived(result ? degradedNotice(result) : null);
  const coverage = $derived(result ? coverageNotice(result) : null);
  const missed = $derived(result ? unreachable(result) : []);
</script>

<form
  class="search-form"
  onsubmit={(event) => {
    event.preventDefault();
    void run();
  }}
>
  <div class="fields">
    <fieldset class="mode-field">
      <legend>When</legend>
      <div>
        <button
          type="button"
          class:active={windowMode === "month"}
          aria-pressed={windowMode === "month"}
          onclick={() => (windowMode = "month")}
        >
          A month
        </button>
        <button
          type="button"
          class:active={windowMode === "dates"}
          aria-pressed={windowMode === "dates"}
          onclick={() => (windowMode = "dates")}
        >
          Exact dates
        </button>
      </div>
    </fieldset>
    {#if windowMode === "month"}
      <label>
        <span>Month</span>
        <input class="input" type="month" bind:value={month} />
      </label>
    {:else}
      <label>
        <span>From</span>
        <input class="input" type="date" bind:value={dateFrom} />
      </label>
      <label>
        <span>To (inclusive)</span>
        <input class="input" type="date" bind:value={dateTo} />
      </label>
    {/if}
    <label>
      <span>Budget (EUR, one way)</span>
      <input class="input" type="number" min="1" step="10" bind:value={budgetEuros} />
    </label>
    <label>
      <span>How many to price</span>
      <input class="input" type="number" min="1" max="20" bind:value={maxCandidates} />
    </label>
    <label class="wide">
      <span>What should happen?</span>
      <input class="input" bind:value={interests} placeholder="Architecture, live music…" />
    </label>
    <fieldset class="mode-field">
      <legend>Transport</legend>
      <div>
        {#each modeOptions as option (option.id)}
          <button
            type="button"
            class:active={modes.includes(option.id)}
            aria-pressed={modes.includes(option.id)}
            onclick={() => toggleMode(option.id)}
          >
            {option.label}
          </button>
        {/each}
      </div>
    </fieldset>
  </div>
  <button class="btn btn-primary" type="submit" disabled={running}>
    <Icon name={running ? "loader" : "search"} size={14} />
    {running ? `Searching… ${Math.round(sinceMs / 1000)}s` : "Find a trip"}
  </button>
</form>

{#if error}
  <p class="search-error" aria-live="polite">{error}</p>
{/if}

{#if result}
  <div class="result-head">
    <p class="coverage">{coverage}</p>
    {#if notice}
      <p class="degraded" aria-live="polite">{notice}</p>
    {/if}
    {#if missed.length > 0}
      <p class="reach">Not reached: {missed.join(" · ")}</p>
    {/if}
    {#if result.window_source === "caller"}
      <p class="reach">
        The calendar was not read, so these dates are the ones you asked for, unchecked.
      </p>
    {/if}
    <p class="revision">Scored by {result.revision}, observed {result.observed_at}</p>
  </div>

  {#if adoptError}
    <p class="search-error" aria-live="polite">
      {adoptError}
      {#if adoptCandidate}
        <button class="btn btn-outline" type="button" onclick={() => void adopt(adoptCandidate!)}>
          Retry adopt
        </button>
      {/if}
    </p>
  {:else if adopted}
    <p class="adopted" aria-live="polite">
      The whole option space is recorded on the new trip as one option set.
    </p>
  {/if}

  <ol class="candidates">
    {#each result.candidates as candidate (candidate.place_id)}
      <li class:unpriced={candidate.estimated_cost_cents === null}>
        <p class="head">
          <strong>{candidate.destination.name}</strong>
          <span class="cost">{money(candidate.estimated_cost_cents, candidate.currency)}</span>
        </p>
        <p class="window">
          {candidate.window.starts_on} → {candidate.window.ends_before} · {candidate.mode}
        </p>

        {#if candidate.factors.length > 0}
          <ul class="factors">
            {#each candidate.factors as factor (factor.key)}
              <li>
                <span class="factor-label">{factor.label}</span>
                <span class="bar"><i style="width: {factorScorePercent(factor)}%"></i></span>
                <span class="factor-why">{factor.rationale}</span>
              </li>
            {/each}
          </ul>
        {:else}
          <p class="factor-why">Nothing about this destination could be measured.</p>
        {/if}

        {#if candidate.companion_hint}
          <p class="companion"><Icon name="map-pin" size={12} /> {candidate.companion_hint}</p>
        {/if}

        {#if candidate.events.length > 0}
          <ul class="events">
            {#each candidate.events.slice(0, 3) as event (event.id)}
              <li>
                <a href={event.url} target="_blank" rel="noreferrer">{event.title}</a>
                {#if event.distance_km !== null}<span>{event.distance_km} km</span>{/if}
              </li>
            {/each}
          </ul>
        {/if}

        <ul class="why">
          {#each candidate.why as reason (reason)}
            <li>{reason}</li>
          {/each}
        </ul>

        <div class="actions">
          <button type="button" onclick={() => onChoose(seedFromCandidate(candidate))}>
            <Icon name="map-pin" size={13} /> Use these fields
          </button>
          {#if onAdopt}
            <button type="button" onclick={() => void adopt(candidate)}>
              Create trip and record every option
            </button>
          {/if}
        </div>
      </li>
    {/each}
  </ol>
{/if}

<style>
  .search-form {
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
    margin-bottom: 1rem;
  }

  .fields {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(11rem, 1fr));
    gap: 0.6rem;
  }

  .fields label {
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
    font-size: 0.75rem;
    color: var(--text-secondary);
  }

  .fields .wide {
    grid-column: 1 / -1;
  }

  .mode-field {
    grid-column: 1 / -1;
    border: 0;
    margin: 0;
    padding: 0;
  }

  .mode-field legend {
    font-size: 0.75rem;
    color: var(--text-secondary);
    padding: 0 0 0.25rem;
  }

  .mode-field div {
    display: flex;
    flex-wrap: wrap;
    gap: 0.35rem;
  }

  .mode-field button {
    padding: 0.3rem 0.65rem;
    border: 1px solid var(--card-border);
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--text-secondary);
    font-size: 0.75rem;
    cursor: pointer;
  }

  .mode-field button.active {
    border-color: var(--primary);
    background-color: var(--primary-soft);
    color: var(--primary);
  }

  .search-error {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0.5rem;
    margin: 0 0 0.75rem;
    font-size: 0.8125rem;
    color: var(--danger, #b91c1c);
  }

  .result-head p {
    margin: 0 0 0.2rem;
    font-size: 0.75rem;
  }

  .coverage {
    color: var(--text-primary);
    font-variant-numeric: tabular-nums;
  }

  .degraded {
    color: var(--text-secondary);
  }

  .reach,
  .revision {
    color: var(--text-tertiary);
  }

  .adopted {
    margin: 0.5rem 0;
    font-size: 0.8125rem;
    color: var(--primary);
  }

  .candidates {
    display: flex;
    flex-direction: column;
    gap: 0.6rem;
    margin: 0.85rem 0 0;
    padding: 0;
    list-style: none;
  }

  .candidates li {
    padding: 0.7rem 0.8rem;
    border: 1px solid var(--card-border);
    border-radius: var(--radius-sm);
  }

  .candidates li.unpriced {
    border-style: dashed;
  }

  .head {
    display: flex;
    justify-content: space-between;
    gap: 0.75rem;
    margin: 0;
    font-size: 0.875rem;
  }

  .cost {
    font-variant-numeric: tabular-nums;
    color: var(--text-secondary);
  }

  .window {
    margin: 0.1rem 0 0.5rem;
    font-size: 0.75rem;
    color: var(--text-tertiary);
    font-variant-numeric: tabular-nums;
  }

  .factors {
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
    margin: 0 0 0.5rem;
    padding: 0;
    list-style: none;
  }

  .factors li {
    display: grid;
    grid-template-columns: 9rem 5rem 1fr;
    align-items: center;
    gap: 0.5rem;
    font-size: 0.6875rem;
  }

  .factor-label {
    color: var(--text-secondary);
  }

  .bar {
    display: block;
    height: 0.35rem;
    border-radius: 999px;
    background-color: var(--card-border);
    overflow: hidden;
  }

  .bar i {
    display: block;
    height: 100%;
    background-color: var(--primary);
  }

  .factor-why {
    color: var(--text-tertiary);
    font-size: 0.6875rem;
  }

  .companion {
    display: flex;
    align-items: center;
    gap: 0.35rem;
    margin: 0 0 0.4rem;
    font-size: 0.75rem;
    color: var(--primary);
  }

  .events,
  .why {
    margin: 0 0 0.5rem;
    padding-left: 1rem;
    font-size: 0.6875rem;
    color: var(--text-tertiary);
  }

  .events li {
    display: flex;
    gap: 0.4rem;
  }

  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: 0.4rem;
  }

  .actions button {
    padding: 0.3rem 0.65rem;
    border: 1px solid var(--card-border);
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--text-secondary);
    font-size: 0.75rem;
    cursor: pointer;
  }

  .actions button:hover {
    border-color: var(--primary);
    color: var(--primary);
  }
</style>
