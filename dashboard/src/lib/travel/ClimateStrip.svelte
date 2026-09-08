<script lang="ts">
  import type { ClimateResult } from "$lib/travel/api";

  /**
   * Twelve months of one destination's climate normal.
   *
   * Renders only what the server sent. The verdict (`best_month`), the window
   * flags (`in_window`) and the rule text all arrive in the response, so no
   * arithmetic happens here beyond rounding for display — the trips README's
   * rule that the frontend renders and does not compute.
   */
  let {
    result,
    rule,
    attribution,
  }: {
    result: ClimateResult;
    rule: string;
    attribution: string;
  } = $props();

  const MONTH_LABELS = ["J", "F", "M", "A", "M", "J", "J", "A", "S", "O", "N", "D"];

  const round = (value: number | null, digits = 0): string =>
    value === null ? "–" : value.toFixed(digits);
</script>

<section class="climate" aria-label="Climate normals by month">
  <header>
    <div>
      <span class="eyebrow">Seasonality</span>
      {#if result.period}
        <p class="period">
          Ten-year normal, {result.period.start.slice(0, 4)}–{result.period.end.slice(0, 4)}
        </p>
      {/if}
    </div>
    {#if result.resolved_by === "nearest" && result.matched_place}
      <!-- A distance match is shown, never assumed: 60 km is an assumption in the
           server's code, so a wrong match has to be visible here. -->
      <p class="matched">
        Nearest normals: {result.matched_place.name}
        {#if result.matched_place.distance_km !== null}
          · {result.matched_place.distance_km.toFixed(0)} km away
        {/if}
      </p>
    {/if}
  </header>

  {#if result.months.length === 0}
    <p class="empty">
      {#if result.reason}
        {result.reason}.
      {:else if result.matched_place}
        <!-- The bare verb fetches `kind = 'city'` rows only (places main.rs), so
             for a station or a venue it is the id that makes the instruction
             true. Naming the place also says which one has nothing. -->
        No normals for {result.matched_place.name} yet — run
        <code>places-server climate fetch --place {result.matched_place.id}</code>.
      {:else}
        No normals for this place yet — run <code>places-server climate fetch</code>.
      {/if}
    </p>
  {:else}
    <ol class="months">
      {#each result.months as month (month.month)}
        <li class:best={month.best_month} class:window={month.in_window}>
          <span class="label">{MONTH_LABELS[month.month - 1]}</span>
          <span class="temps">
            {round(month.t_max_mean)}° <small>/ {round(month.t_min_mean)}°</small>
          </span>
          <span class="rain">{round(month.rain_days_mean)} rain d</span>
        </li>
      {/each}
    </ol>
    <footer>
      <p class="rule">Marked months: {rule}.</p>
      <!-- A licence obligation (upstreams.toml [open-meteo-api]), not decoration.
           Removing this line makes the recorded adoption untrue. -->
      <p class="attribution">{attribution}</p>
    </footer>
  {/if}
</section>

<style>
  .climate {
    display: grid;
    gap: 0.5rem;
    padding: 0.75rem 0;
  }

  header {
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    justify-content: space-between;
    gap: 0.5rem;
  }

  .period,
  .matched,
  .rule,
  .attribution,
  .empty {
    margin: 0;
    font-size: var(--text-xs);
    color: var(--text-tertiary);
  }

  .matched {
    color: var(--text-secondary);
  }

  .months {
    display: grid;
    grid-template-columns: repeat(12, minmax(0, 1fr));
    gap: 0.25rem;
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .months li {
    display: grid;
    gap: 0.15rem;
    padding: 0.4rem 0.25rem;
    text-align: center;
    background: var(--surface);
    border: 1px solid transparent;
    border-radius: var(--radius-sm);
  }

  .months li.best {
    background: var(--success-soft);
  }

  .months li.window {
    border-color: var(--primary);
  }

  .label {
    font-size: 0.7rem;
    font-weight: 600;
    color: var(--text-secondary);
  }

  .temps {
    font-size: 0.8rem;
    color: var(--text-primary);
    font-variant-numeric: tabular-nums;
  }

  .temps small {
    color: var(--text-tertiary);
  }

  .rain {
    font-size: 0.65rem;
    color: var(--text-tertiary);
    font-variant-numeric: tabular-nums;
  }

  footer {
    display: flex;
    flex-wrap: wrap;
    justify-content: space-between;
    gap: 0.5rem;
  }

  @media (width < 48rem) {
    .months {
      grid-template-columns: repeat(6, minmax(0, 1fr));
    }
  }
</style>
