<script lang="ts">
  import Icon from '$lib/Icon.svelte';
  import { link } from '$lib/nav';
  import type { CalendarEntry, MacmonSample, TripPlan } from '$lib/api';

  let {
    entries = [],
    plans = [],
    macmon = null,
  }: {
    entries: CalendarEntry[];
    plans: TripPlan[];
    macmon: MacmonSample | null;
  } = $props();

  const now = new Date();
  const todayStr = now.toISOString().slice(0, 10);

  // Next event today
  const todayEntries = $derived(
    entries
      .filter((e) => e.starts_at.slice(0, 10) === todayStr)
      .sort((a, b) => a.starts_at.localeCompare(b.starts_at))
  );

  const nextEntry = $derived.by(() => {
    const currentMs = now.getTime();
    for (const e of todayEntries) {
      const endMs = new Date(e.ends_at).getTime();
      if (endMs > currentMs) {
        const startMs = new Date(e.starts_at).getTime();
        const isNow = currentMs >= startMs;
        const diffMin = Math.round((startMs - currentMs) / 60_000);
        return {
          entry: e,
          isNow,
          proximityText: isNow
            ? 'Happening now'
            : diffMin < 60
              ? `Starts in ${diffMin}m`
              : `Starts in ${Math.floor(diffMin / 60)}h ${diffMin % 60}m`,
        };
      }
    }
    return null;
  });

  // Upcoming trip within 14 days
  const nextTrip = $derived.by(() => {
    const upcoming = plans
      .filter((p) => p.date_start >= todayStr)
      .sort((a, b) => a.date_start.localeCompare(b.date_start));
    return upcoming[0] ?? null;
  });

  // System stats
  const cpuTemp = $derived(
    macmon?.temp?.cpu_temp_avg != null ? `${macmon.temp.cpu_temp_avg.toFixed(0)}°C` : null
  );
  const ramUsage = $derived(
    macmon?.memory?.ram_usage != null && macmon.memory.ram_total != null
      ? `${(macmon.memory.ram_usage / 1073741824).toFixed(1)} / ${(macmon.memory.ram_total / 1073741824).toFixed(0)} GB`
      : null
  );
</script>

<aside class="axon-glance card" aria-label="Axon day summary">
  <div class="glance-top">
    <div class="glance-title">
      <span class="live-dot"></span>
      <Icon name="sparkles" size={14} />
      <strong>Axon Glance</strong>
    </div>

    {#if cpuTemp || ramUsage}
      <div class="hw-chip mono">
        {#if cpuTemp}<span>CPU {cpuTemp}</span>{/if}
        {#if cpuTemp && ramUsage}<span class="sep">·</span>{/if}
        {#if ramUsage}<span>RAM {ramUsage}</span>{/if}
      </div>
    {/if}
  </div>

  <div class="glance-signals">
    {#if nextEntry}
      <div class="signal-item" class:urgent={nextEntry.isNow}>
        <div class="signal-icon">
          <Icon name="calendar" size={14} />
        </div>
        <div class="signal-content">
          <div class="signal-meta">
            <span class="signal-badge" class:badge-now={nextEntry.isNow}>
              {nextEntry.proximityText}
            </span>
            <span class="signal-time mono">{nextEntry.entry.starts_at.slice(11, 16)}</span>
          </div>
          <span class="signal-text">{nextEntry.entry.title}</span>
        </div>
      </div>
    {:else}
      <div class="signal-item idle">
        <div class="signal-icon">
          <Icon name="check" size={14} />
        </div>
        <div class="signal-content">
          <span class="signal-badge">Schedule clear</span>
          <span class="signal-text">No further calendar commitments today.</span>
        </div>
      </div>
    {/if}

    {#if nextTrip}
      <a class="signal-item trip-link" href={link('/travel')}>
        <div class="signal-icon trip-icon">
          <Icon name="train" size={14} />
        </div>
        <div class="signal-content">
          <div class="signal-meta">
            <span class="signal-badge trip-badge">Upcoming Travel</span>
            <span class="signal-time mono">{nextTrip.date_start}</span>
          </div>
          <span class="signal-text">{nextTrip.title}</span>
        </div>
        <Icon name="arrow-right" size={12} />
      </a>
    {/if}
  </div>
</aside>

<style>
  .axon-glance {
    position: relative;
    overflow: hidden;
    padding: var(--space-4) var(--space-5);
    margin-bottom: var(--space-4);
    background:
      radial-gradient(120% 90% at 100% 0%, var(--primary-soft) 0%, transparent 60%),
      var(--card-bg);
    border: 1px solid var(--card-border);
    border-radius: var(--radius-lg);
    box-shadow: var(--card-shadow);
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  .glance-top {
    display: flex;
    justify-content: space-between;
    align-items: center;
  }

  .glance-title {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    font-size: var(--text-xs);
    color: var(--primary);
  }

  .live-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background-color: var(--primary);
    box-shadow: 0 0 6px var(--primary);
  }

  .hw-chip {
    display: inline-flex;
    align-items: center;
    gap: 0.3rem;
    font-size: var(--text-2xs);
    color: var(--text-tertiary);
    background-color: var(--surface);
    padding: 0.15rem 0.5rem;
    border-radius: var(--radius-sm);
  }

  .sep {
    opacity: 0.5;
  }

  .glance-signals {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(240px, 1fr));
    gap: var(--space-3);
  }

  .signal-item {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-3);
    border-radius: var(--radius-md);
    background-color: var(--surface);
    text-decoration: none;
    color: inherit;
    border: 1px solid transparent;
    transition: background-color 0.15s ease, border-color 0.15s ease;
  }

  .signal-item.urgent {
    background-color: color-mix(in srgb, var(--warning) 10%, var(--surface));
    border-color: color-mix(in srgb, var(--warning) 30%, transparent);
  }

  .trip-link:hover {
    background-color: var(--card-bg);
    border-color: var(--primary);
  }

  .signal-icon {
    display: grid;
    place-items: center;
    width: 2rem;
    height: 2rem;
    border-radius: var(--radius-sm);
    background-color: var(--card-bg);
    color: var(--primary);
    flex-shrink: 0;
  }

  .trip-icon {
    background-color: var(--primary-soft);
  }

  .signal-content {
    display: flex;
    flex-direction: column;
    gap: 0.15rem;
    min-width: 0;
    flex: 1;
  }

  .signal-meta {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .signal-badge {
    font-size: var(--text-2xs);
    font-weight: 600;
    color: var(--primary);
  }

  .badge-now {
    color: var(--warning-ink);
  }

  .trip-badge {
    color: var(--text-secondary);
  }

  .signal-time {
    font-size: var(--text-2xs);
    color: var(--text-tertiary);
  }

  .signal-text {
    font-size: var(--text-xs);
    font-weight: 600;
    color: var(--text-primary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
