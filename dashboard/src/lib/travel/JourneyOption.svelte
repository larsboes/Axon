<script lang="ts">
  import Icon from "$lib/Icon.svelte";
  import type { Journey, ConnectionLeg } from "$lib/api";

  let {
    journey,
    expanded,
    saved,
    onToggle,
    onSave,
  }: {
    journey: Journey;
    expanded: boolean;
    saved: boolean;
    onToggle: () => void;
    onSave: () => void;
  } = $props();

  const time = (date: string) =>
    new Intl.DateTimeFormat("en-GB", { hour: "2-digit", minute: "2-digit" }).format(
      new Date(date),
    );

  const duration = (minutes: number) =>
    `${Math.floor(minutes / 60)}:${String(minutes % 60).padStart(2, "0")} h`;

  /// Minutes between the plan and reality, or null when HAFAS offered no real-time
  /// value. Null is "not reported", never "on time" -- rendering the two the same way
  /// is what made a late train look punctual before these fields were carried at all.
  const delayOf = (scheduled?: string | null, realtime?: string | null): number | null => {
    if (!scheduled || !realtime) return null;
    const delta = (new Date(realtime).getTime() - new Date(scheduled).getTime()) / 60000;
    return Number.isFinite(delta) ? Math.round(delta) : null;
  };

  const legDelay = (leg: ConnectionLeg) => ({
    departure: delayOf(leg.scheduled_departure, leg.realtime_departure),
    arrival: delayOf(leg.scheduled_arrival, leg.realtime_arrival),
  });

  const signed = (minutes: number) => (minutes > 0 ? `+${minutes}` : `${minutes}`);

  const punctuality = $derived(journey.arrival_punctuality ?? null);

  /// Punctuality's own sample floor is 30, so anything present cleared it. These bands
  /// only say how far past the floor it got -- the number is shown either way, because
  /// a reader who wants to judge 47 observations for themselves should be able to.
  const confidence = $derived.by(() => {
    if (!punctuality) return null;
    if (punctuality.n >= 1000) return { label: "lots of data", bars: 4 };
    if (punctuality.n >= 300) return { label: "solid sample", bars: 3 };
    if (punctuality.n >= 100) return { label: "narrow sample", bars: 2 };
    return { label: "few journeys", bars: 1 };
  });

  const tone = $derived.by(() => {
    if (!punctuality) return "unknown";
    if (punctuality.share_late_6 >= 0.35) return "bad";
    if (punctuality.share_late_6 >= 0.15) return "mixed";
    return "good";
  });

  const percent = (share: number) => `${Math.round(share * 100)} %`;

  /// The p50 marker's position on a track that ends at p90. Both are minutes late, so
  /// the track is the spread between the ordinary case and the unlucky one.
  const p50Offset = $derived.by(() => {
    if (!punctuality || punctuality.p90 <= 0) return 0;
    return Math.min(100, Math.max(0, (punctuality.p50 / punctuality.p90) * 100));
  });

  const anyCancelled = $derived(journey.legs.some((leg) => leg.cancelled));
</script>

<li class:expanded>
  <button class="journey-open" type="button" aria-expanded={expanded} onclick={onToggle}>
    <span class="journey-time">
      <strong>{time(journey.legs[0]?.departure_time ?? "")}</strong>
      <span>{duration(journey.total_duration_minutes)}</span>
    </span>
    <span class="journey-route">
      <strong>{journey.legs.map((leg) => leg.train_name || leg.train_number).join(" · ")}</strong>
      <span>
        {journey.legs.length - 1 === 0
          ? "direct"
          : `${journey.legs.length - 1} change${journey.legs.length - 1 === 1 ? "" : "s"}`}
        {#if punctuality}
          <span class="dot" aria-hidden="true">·</span>
          <span class="risk {tone}">{percent(punctuality.share_late_6)} ≥ 6 min late</span>
        {:else}
          <span class="dot" aria-hidden="true">·</span>
          <span class="risk unknown">no delay history</span>
        {/if}
      </span>
    </span>
    <Icon name="arrow-right" size={13} />
  </button>

  <div class="journey-action">
    <strong>
      {journey.total_price === null ? "price unknown" : `${journey.total_price.toFixed(2)} €`}
    </strong>
    <button class="save" type="button" disabled={saved} onclick={onSave}>
      {#if saved}
        <Icon name="check" size={13} /> Saved
      {:else}
        <Icon name="plus" size={13} /> Add
      {/if}
    </button>
  </div>

  {#if anyCancelled}
    <p class="cancelled-banner">At least one leg is reported cancelled.</p>
  {/if}

  {#if expanded}
    <div class="detail">
      <h4>Itinerary</h4>
      <ol class="leg-list">
        {#each journey.legs as leg, index (`${journey.id}:${index}`)}
          {@const delay = legDelay(leg)}
          <li class:leg-cancelled={leg.cancelled}>
            <span class="leg-index">{index + 1}</span>
            <div>
              <strong>{leg.train_name || leg.train_number}</strong>
              <span class="leg-stop">
                <span class="clock">{time(leg.departure_time)}</span>
                {leg.origin.name}
                {#if delay.departure !== null && delay.departure !== 0}
                  <span class="delay {delay.departure > 0 ? 'late' : 'early'}">
                    {signed(delay.departure)} min
                  </span>
                {/if}
              </span>
              <span class="leg-stop">
                <span class="clock">{time(leg.arrival_time)}</span>
                {leg.destination.name}
                {#if delay.arrival !== null && delay.arrival !== 0}
                  <span class="delay {delay.arrival > 0 ? 'late' : 'early'}">
                    {signed(delay.arrival)} min
                  </span>
                {/if}
              </span>
              {#if leg.cancelled}
                <span class="badge danger">Cancelled</span>
              {/if}
            </div>
            <small>
              {#if leg.platform}Platform {leg.platform}{:else}Platform TBA{/if}
              {#if leg.is_regional}<br />Deutschland-Ticket{/if}
            </small>
          </li>
        {/each}
      </ol>

      <h4>Punctuality at destination</h4>
      {#if punctuality}
        <p class="cell-key">
          {punctuality.station_name ?? journey.end_station.name} · {punctuality.train_type} ·
          {punctuality.weekend ? "weekend" : "weekday"}, {punctuality.hour}:00
        </p>

        <dl class="stats">
          <div>
            <dt>typically</dt>
            <dd class="mono">{signed(punctuality.p50)} min</dd>
          </div>
          <div>
            <dt>unlucky case</dt>
            <dd class="mono">{signed(punctuality.p90)} min</dd>
          </div>
          <div>
            <dt>≥ 6 min late</dt>
            <dd class="mono {tone}">{percent(punctuality.share_late_6)}</dd>
          </div>
          <div>
            <dt>cancelled</dt>
            <dd class="mono">{percent(punctuality.cancel_rate)}</dd>
          </div>
        </dl>

        <div class="spread" aria-hidden="true">
          <span class="spread-end">+0</span>
          <span class="track">
            <span class="fill {tone}" style="width: {p50Offset}%"></span>
            <span class="marker" style="left: {p50Offset}%"></span>
          </span>
          <span class="spread-end">{signed(punctuality.p90)}</span>
        </div>

        <p class="confidence">
          <span class="bars" aria-hidden="true">
            {#each [1, 2, 3, 4] as bar (bar)}
              <span class:on={confidence !== null && bar <= confidence.bars}></span>
            {/each}
          </span>
          {confidence?.label} · {punctuality.n.toLocaleString("en-GB")} journeys measured
        </p>
        <p class="caveat">
          Measured on comparable trains at this stop. Not a forecast for this journey,
          and not a statement about transfer risk.
        </p>
      {:else}
        <p class="caveat">
          No delay history for this stop, train type and hour. That means unmeasured,
          not punctual.
        </p>
      {/if}
    </div>
  {/if}
</li>

<style>
  li {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    gap: 0.75rem;
    align-items: center;
    padding: 0.75rem;
    border-bottom: 1px solid var(--card-border);
  }

  li:last-child {
    border-bottom: 0;
  }

  li.expanded {
    background: color-mix(in srgb, var(--primary-soft) 38%, var(--card-bg));
  }

  .journey-open {
    display: grid;
    grid-template-columns: 4rem minmax(0, 1fr) auto;
    gap: 0.75rem;
    align-items: center;
    min-width: 0;
    padding: 0;
    border: 0;
    background: transparent;
    color: var(--text-primary);
    font: inherit;
    text-align: left;
    cursor: pointer;
  }

  .journey-open :global(svg) {
    color: var(--text-tertiary);
    transition: transform 140ms ease;
  }

  .journey-open[aria-expanded="true"] :global(svg) {
    transform: rotate(90deg);
  }

  .journey-time strong,
  .journey-time span,
  .journey-route strong,
  .journey-route span,
  .journey-action strong {
    display: block;
  }

  .journey-time strong {
    font-family: var(--font-mono);
    font-size: 0.8125rem;
  }

  .journey-time span,
  .journey-route span {
    color: var(--text-tertiary);
    font-size: 0.625rem;
  }

  .journey-route {
    min-width: 0;
  }

  .journey-route strong {
    overflow: hidden;
    font-size: 0.75rem;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .dot,
  .risk {
    display: inline !important;
    margin-left: 0.25rem;
  }

  .risk.good {
    color: var(--success);
  }

  .risk.mixed {
    color: var(--warning);
  }

  .risk.bad {
    color: var(--danger);
  }

  .risk.unknown {
    color: var(--text-tertiary);
    font-style: italic;
  }

  .journey-action {
    text-align: right;
  }

  .journey-action strong {
    font-size: 0.75rem;
  }

  .save {
    display: inline-flex;
    align-items: center;
    gap: 0.25rem;
    margin-top: 0.2rem;
    padding: 0.2rem 0.35rem;
    border: 0;
    border-radius: var(--radius-sm);
    background: var(--primary-soft);
    color: var(--primary);
    font: inherit;
    font-size: 0.625rem;
    font-weight: 600;
    cursor: pointer;
  }

  .save:disabled {
    color: var(--success);
    cursor: default;
  }

  .cancelled-banner {
    grid-column: 1 / -1;
    margin: 0;
    padding: 0.35rem 0.5rem;
    border-radius: var(--radius-sm);
    background: var(--danger-soft);
    color: var(--danger);
    font-size: 0.625rem;
    font-weight: 600;
  }

  .detail {
    grid-column: 1 / -1;
    padding-top: 0.5rem;
    border-top: 1px solid var(--card-border);
  }

  h4 {
    margin: 0.6rem 0 0.35rem;
    color: var(--text-tertiary);
    font-size: 0.5625rem;
    font-weight: 700;
    letter-spacing: 0.08em;
    text-transform: uppercase;
  }

  .leg-list {
    list-style: none;
    margin: 0;
    padding: 0;
  }

  .leg-list > li {
    display: grid;
    grid-template-columns: 1.5rem minmax(0, 1fr) auto;
    gap: 0.55rem;
    align-items: start;
    padding: 0.6rem 0;
    border-bottom: 1px solid var(--card-border);
  }

  .leg-list > li:last-child {
    border-bottom: 0;
  }

  .leg-list > li.leg-cancelled {
    opacity: 0.7;
    text-decoration: line-through;
  }

  .leg-index {
    display: grid;
    place-items: center;
    width: 1.35rem;
    height: 1.35rem;
    border-radius: 50%;
    background: var(--primary-soft);
    color: var(--primary);
    font-family: var(--font-mono);
    font-size: 0.5625rem;
    font-weight: 700;
  }

  .leg-list strong {
    display: block;
    font-size: 0.6875rem;
  }

  .leg-stop {
    display: block;
    margin-top: 0.15rem;
    color: var(--text-tertiary);
    font-size: 0.625rem;
  }

  .clock {
    display: inline-block;
    min-width: 2.6rem;
    color: var(--text-secondary);
    font-family: var(--font-mono);
  }

  .delay {
    margin-left: 0.3rem;
    font-family: var(--font-mono);
    font-weight: 700;
  }

  .delay.late {
    color: var(--danger);
  }

  .delay.early {
    color: var(--success);
  }

  .badge {
    display: inline-block;
    margin-top: 0.25rem;
    padding: 0.1rem 0.3rem;
    border-radius: var(--radius-sm);
    font-size: 0.5625rem;
    font-weight: 700;
  }

  .badge.danger {
    background: var(--danger-soft);
    color: var(--danger);
  }

  .leg-list small {
    color: var(--text-tertiary);
    font-size: 0.625rem;
    text-align: right;
  }

  .cell-key {
    margin: 0 0 0.4rem;
    color: var(--text-secondary);
    font-size: 0.625rem;
  }

  .stats {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(5.5rem, 1fr));
    gap: 0.4rem;
    margin: 0;
  }

  .stats div {
    padding: 0.35rem 0.45rem;
    border: 1px solid var(--card-border);
    border-radius: var(--radius-sm);
    background: var(--card-bg);
  }

  .stats dt {
    color: var(--text-tertiary);
    font-size: 0.5625rem;
  }

  .stats dd {
    margin: 0.1rem 0 0;
    font-size: 0.75rem;
    font-weight: 700;
  }

  .mono {
    font-family: var(--font-mono);
  }

  .stats dd.good {
    color: var(--success);
  }

  .stats dd.mixed {
    color: var(--warning);
  }

  .stats dd.bad {
    color: var(--danger);
  }

  .spread {
    display: grid;
    grid-template-columns: auto minmax(0, 1fr) auto;
    gap: 0.4rem;
    align-items: center;
    margin-top: 0.5rem;
  }

  .spread-end {
    color: var(--text-tertiary);
    font-family: var(--font-mono);
    font-size: 0.5625rem;
  }

  .track {
    position: relative;
    display: block;
    height: 0.3rem;
    border-radius: 999px;
    background: var(--card-border);
  }

  .fill {
    position: absolute;
    inset: 0 auto 0 0;
    border-radius: 999px;
    background: var(--text-tertiary);
  }

  .fill.good {
    background: var(--success);
  }

  .fill.mixed {
    background: var(--warning);
  }

  .fill.bad {
    background: var(--danger);
  }

  .marker {
    position: absolute;
    top: -0.15rem;
    width: 2px;
    height: 0.6rem;
    background: var(--text-primary);
  }

  .confidence {
    display: flex;
    align-items: center;
    gap: 0.35rem;
    margin: 0.5rem 0 0;
    color: var(--text-secondary);
    font-size: 0.625rem;
  }

  .bars {
    display: inline-flex;
    gap: 1px;
  }

  .bars span {
    width: 3px;
    height: 0.6rem;
    border-radius: 1px;
    background: var(--card-border);
  }

  .bars span.on {
    background: var(--primary);
  }

  .caveat {
    margin: 0.35rem 0 0.2rem;
    color: var(--text-tertiary);
    font-size: 0.5625rem;
    line-height: 1.45;
  }
</style>
