<script lang="ts">
  import Icon from "$lib/Icon.svelte";
  import { bridgedSrc } from "$lib/bridged-url";
  import type { CalendarEntry, PlanItem, TripPlan, TripStage } from "$lib/api";

  let {
    plan,
    items,
    calendarEntries = [],
    onRemoveItem,
    onUpdateItemDay,
  }: {
    plan: TripPlan;
    items: PlanItem[];
    calendarEntries?: CalendarEntry[];
    onRemoveItem: (item: PlanItem) => Promise<void>;
    onUpdateItemDay?: (item: PlanItem, newDay: string | null) => Promise<void>;
  } = $props();

  let selectedDayFilter = $state<string | null>(null);

  interface DayBucket {
    dayIso: string;
    dayIndex: number;
    weekday: string;
    shortDate: string;
    stages: TripStage[];
    items: PlanItem[];
    commitments: CalendarEntry[];
  }

  function parseDate(iso: string): Date | null {
    if (!/^\d{4}-\d{2}-\d{2}$/.test(iso)) return null;
    const [y, m, d] = iso.split("-").map((v) => parseInt(v, 10));
    return new Date(Date.UTC(y, m - 1, d));
  }

  function formatIso(d: Date): string {
    const y = d.getUTCFullYear();
    const m = String(d.getUTCMonth() + 1).padStart(2, "0");
    const day = String(d.getUTCDate()).padStart(2, "0");
    return `${y}-${m}-${day}`;
  }

  const weekdays = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
  const months = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun",
    "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
  ];

  function formatShortDate(d: Date): string {
    return `${d.getUTCDate()} ${months[d.getUTCMonth()]}`;
  }

  // Same-origin paths only. A provider's `image_url` points at a remote host, and loading
  // it tells that host which place is on the itinerary each time the page renders.
  function itemImage(item: PlanItem): string | null {
    if (!item.payload || typeof item.payload !== "object") return null;
    const value = (item.payload as { image_url?: unknown }).image_url;
    return typeof value === "string" && value.startsWith("/") && !value.startsWith("//")
      ? value
      : null;
  }

  function itemPrice(item: PlanItem): string | null {
    if (!item.payload || typeof item.payload !== "object") return null;
    const p = item.payload as Record<string, unknown>;
    if (typeof p.total_price === "number") {
      return `${p.total_price.toFixed(2)} €`;
    }
    if (typeof p.estimated_cost_cents === "number") {
      return `${(p.estimated_cost_cents / 100).toFixed(2)} €`;
    }
    return null;
  }

  function itemTime(item: PlanItem): string | null {
    if (!item.payload || typeof item.payload !== "object") return null;
    const p = item.payload as Record<string, unknown>;
    if (typeof p.departure === "string") {
      const match = p.departure.match(/T(\d{2}:\d{2})/);
      if (match) return match[1];
    }
    if (typeof p.starts_at === "string") {
      const match = p.starts_at.match(/T(\d{2}:\d{2})/);
      if (match) return match[1];
    }
    return null;
  }

  function itemTypeBadge(type: PlanItem["item_type"]): { label: string; icon: "train" | "ticket" | "compass" | "home" | "layout" | "activity" } {
    switch (type) {
      case "journey":
      case "transport":
        return { label: "Transit", icon: "train" };
      case "event":
        return { label: "Event", icon: "ticket" };
      case "activity":
        return { label: "Activity", icon: "activity" };
      case "stay":
        return { label: "Stay", icon: "home" };
      default:
        return { label: "Item", icon: "layout" };
    }
  }

  const daysTimeline = $derived.by<DayBucket[]>(() => {
    const buckets: DayBucket[] = [];
    const startDate = parseDate(plan.date_start);
    const endDate = parseDate(plan.date_end);

    if (!startDate || !endDate || endDate < startDate) {
      // Fallback: build buckets from items' existing days
      const daysSet = new Set<string>();
      for (const item of items) {
        if (item.day) daysSet.add(item.day);
      }
      for (const stage of plan.stages ?? []) {
        if (stage.date) daysSet.add(stage.date);
      }
      const sortedDays = Array.from(daysSet).sort();
      sortedDays.forEach((iso, idx) => {
        const d = parseDate(iso) ?? new Date();
        buckets.push({
          dayIso: iso,
          dayIndex: idx + 1,
          weekday: weekdays[d.getUTCDay()],
          shortDate: formatShortDate(d),
          stages: (plan.stages ?? []).filter((s) => s.date === iso),
          items: items.filter((it) => it.day === iso),
          commitments: (calendarEntries ?? []).filter(
            (e) => e.starts_at && e.starts_at.slice(0, 10) === iso,
          ),
        });
      });
      return buckets;
    }

    const current = new Date(startDate);
    let index = 1;
    // Cap at 42 days to prevent runaway loops if dates are absurd
    while (current <= endDate && index <= 42) {
      const iso = formatIso(current);
      buckets.push({
        dayIso: iso,
        dayIndex: index,
        weekday: weekdays[current.getUTCDay()],
        shortDate: formatShortDate(current),
        stages: (plan.stages ?? []).filter((s) => s.date === iso),
        items: items.filter((it) => it.day === iso),
        commitments: (calendarEntries ?? []).filter(
          (e) => e.starts_at && e.starts_at.slice(0, 10) === iso,
        ),
      });
      current.setUTCDate(current.getUTCDate() + 1);
      index += 1;
    }
    return buckets;
  });

  const unscheduledItems = $derived.by<PlanItem[]>(() => {
    const daySet = new Set(daysTimeline.map((d) => d.dayIso));
    return items.filter((it) => !it.day || !daySet.has(it.day));
  });

  const displayedDays = $derived.by<DayBucket[]>(() => {
    if (!selectedDayFilter) return daysTimeline;
    return daysTimeline.filter((d) => d.dayIso === selectedDayFilter);
  });
</script>

<div class="timeline-container">
  <!-- Day Navigator Pills -->
  {#if daysTimeline.length > 1}
    <div class="day-pills-bar" role="tablist" aria-label="Timeline days">
      <button
        type="button"
        class="pill"
        class:active={selectedDayFilter === null}
        onclick={() => (selectedDayFilter = null)}
      >
        All days
      </button>
      {#each daysTimeline as bucket (bucket.dayIso)}
        <button
          type="button"
          class="pill"
          class:active={selectedDayFilter === bucket.dayIso}
          onclick={() => (selectedDayFilter = bucket.dayIso)}
        >
          <span class="pill-day">D{bucket.dayIndex}</span>
          <span class="pill-date">{bucket.weekday}</span>
          {#if bucket.items.length + bucket.stages.length > 0}
            <span class="pill-dot"></span>
          {/if}
        </button>
      {/each}
    </div>
  {/if}

  <!-- Days Track -->
  <div class="timeline-track">
    {#each displayedDays as bucket (bucket.dayIso)}
      <section class="timeline-day">
        <header class="day-header">
          <div class="day-marker">
            <span class="day-number">D{bucket.dayIndex}</span>
          </div>
          <div class="day-titles">
            <h4>{bucket.weekday}, {bucket.shortDate}</h4>
            <span class="day-meta">
              {bucket.items.length + bucket.stages.length}
              {bucket.items.length + bucket.stages.length === 1 ? "entry" : "entries"}
            </span>
          </div>
        </header>

        <div class="day-content">
          <!-- Calendar commitments on this day -->
          {#if bucket.commitments.length > 0}
            <div class="calendar-commitments">
              <span class="commitments-label">Calendar:</span>
              {#each bucket.commitments as entry (entry.id)}
                <div class="commitment-pill commitment-{entry.commitment}" title={entry.notes ?? entry.title}>
                  <Icon name="calendar" size={11} />
                  <span class="commitment-title">{entry.title}</span>
                  {#if entry.starts_at && entry.starts_at.includes("T")}
                    <span class="commitment-time">{entry.starts_at.slice(11, 16)}</span>
                  {/if}
                </div>
              {/each}
            </div>
          {/if}

          <!-- Stages on this day -->
          {#each bucket.stages as stage (stage.id)}
            <div class="timeline-card stage-card" class:stage-card-open={stage.status === 'open'}>
              <div class="card-icon stage-icon" class:stage-icon-open={stage.status === 'open'}>
                <Icon name={stage.status === 'open' ? 'git-branch' : 'map-pin'} size={14} />
              </div>
              <div class="card-body">
                <div class="card-topline">
                  <span class="badge badge-stage">Leg {stage.sequence + 1}</span>
                  {#if stage.status === 'open'}
                    <span class="badge badge-open">Open Branch</span>
                  {:else}
                    <span class="stage-status">{stage.status}</span>
                  {/if}
                </div>
                <div class="stage-route">
                  <strong>{stage.origin.name}</strong>
                  <Icon name="arrow-right" size={12} />
                  <strong>{stage.destination.name}</strong>
                </div>
                {#if stage.branch_note}
                  <div class="branch-note">
                    <Icon name="alert" size={11} />
                    <span>{stage.branch_note}</span>
                  </div>
                {/if}
              </div>
            </div>
          {/each}

          <!-- Items on this day -->
          {#each bucket.items as item (item.id)}
            {@const badge = itemTypeBadge(item.item_type)}
            {@const img = itemImage(item)}
            {@const timeStr = itemTime(item)}
            {@const priceStr = itemPrice(item)}
            <div class="timeline-card item-card">
              {#if img}
                <img class="card-thumb" use:bridgedSrc={img} alt="" />
              {:else}
                <div class="card-icon item-type-icon">
                  <Icon name={badge.icon} size={14} />
                </div>
              {/if}
              <div class="card-body">
                <div class="card-topline">
                  <span class="badge badge-{item.item_type}">{badge.label}</span>
                  {#if timeStr}
                    <span class="item-time">
                      <Icon name="clock" size={11} />
                      {timeStr}
                    </span>
                  {/if}
                  {#if priceStr}
                    <span class="item-price">{priceStr}</span>
                  {/if}
                </div>
                <strong class="item-title">{item.title}</strong>
              </div>
              <div class="card-actions">
                {#if onUpdateItemDay}
                  <select
                    class="day-assign-select"
                    value={item.day ?? ""}
                    onchange={(e) => void onUpdateItemDay?.(item, e.currentTarget.value || null)}
                    title="Change day or unschedule"
                  >
                    <option value="">Unschedule</option>
                    {#each daysTimeline as d (d.dayIso)}
                      <option value={d.dayIso}>D{d.dayIndex} ({d.shortDate})</option>
                    {/each}
                  </select>
                {/if}
                <button
                  type="button"
                  class="remove-btn"
                  onclick={() => void onRemoveItem(item)}
                  aria-label={`Remove ${item.title}`}
                  title="Remove from itinerary"
                >
                  <Icon name="close" size={13} />
                </button>
              </div>
            </div>
          {/each}

          <!-- Empty day notice -->
          {#if bucket.items.length === 0 && bucket.stages.length === 0 && bucket.commitments.length === 0}
            <div class="empty-day-placeholder">
              <span class="empty-dash"></span>
              <p>Free day · Nothing scheduled yet</p>
            </div>
          {/if}
        </div>
      </section>
    {/each}

    <!-- Unscheduled Section -->
    {#if unscheduledItems.length > 0 && selectedDayFilter === null}
      <section class="timeline-day unscheduled-day">
        <header class="day-header">
          <div class="day-marker unscheduled-marker">
            <Icon name="layout" size={13} />
          </div>
          <div class="day-titles">
            <h4>Unscheduled & Notes</h4>
            <span class="day-meta">{unscheduledItems.length} items</span>
          </div>
        </header>

        <div class="day-content">
          {#each unscheduledItems as item (item.id)}
            {@const badge = itemTypeBadge(item.item_type)}
            {@const img = itemImage(item)}
            <div class="timeline-card item-card">
              {#if img}
                <img class="card-thumb" use:bridgedSrc={img} alt="" />
              {:else}
                <div class="card-icon item-type-icon">
                  <Icon name={badge.icon} size={14} />
                </div>
              {/if}
              <div class="card-body">
                <div class="card-topline">
                  <span class="badge badge-{item.item_type}">{badge.label}</span>
                  {#if item.day}
                    <span class="item-time">{item.day}</span>
                  {/if}
                </div>
                <strong class="item-title">{item.title}</strong>
              </div>
              <div class="card-actions">
                {#if onUpdateItemDay}
                  <select
                    class="day-assign-select unscheduled-assign"
                    value=""
                    onchange={(e) => void onUpdateItemDay?.(item, e.currentTarget.value || null)}
                    title="Schedule to day"
                  >
                    <option value="" disabled selected>+ Schedule to day</option>
                    {#each daysTimeline as d (d.dayIso)}
                      <option value={d.dayIso}>Day {d.dayIndex} ({d.weekday} {d.shortDate})</option>
                    {/each}
                  </select>
                {/if}
                <button
                  type="button"
                  class="remove-btn"
                  onclick={() => void onRemoveItem(item)}
                  aria-label={`Remove ${item.title}`}
                >
                  <Icon name="close" size={13} />
                </button>
              </div>
            </div>
          {/each}
        </div>
      </section>
    {/if}
  </div>
</div>

<style>
  .timeline-container {
    display: flex;
    flex-direction: column;
    gap: 1rem;
  }

  .day-pills-bar {
    display: flex;
    align-items: center;
    gap: 0.35rem;
    overflow-x: auto;
    padding-bottom: 0.4rem;
    scrollbar-width: thin;
  }

  .pill {
    display: inline-flex;
    align-items: center;
    gap: 0.3rem;
    padding: 0.25rem 0.6rem;
    border-radius: var(--radius-full, 9999px);
    border: 1px solid var(--border);
    background: var(--surface-1);
    color: var(--text-muted);
    font-size: var(--text-xs);
    cursor: pointer;
    white-space: nowrap;
    transition: all 0.15s ease;
  }

  .pill:hover {
    background: var(--surface-2);
    color: var(--text);
  }

  .pill.active {
    background: var(--accent);
    color: #fff;
    border-color: var(--accent);
  }

  .pill-day {
    font-weight: 600;
  }

  .pill-dot {
    width: 5px;
    height: 5px;
    border-radius: 50%;
    background: currentColor;
    opacity: 0.8;
  }

  .timeline-track {
    display: flex;
    flex-direction: column;
    gap: 1.25rem;
    position: relative;
  }

  .timeline-day {
    display: flex;
    flex-direction: column;
    gap: 0.6rem;
    position: relative;
    padding-left: 2rem;
  }

  .timeline-day::before {
    content: "";
    position: absolute;
    top: 1.5rem;
    bottom: -0.75rem;
    left: 0.75rem;
    width: 2px;
    background: var(--border);
  }

  .timeline-day:last-child::before {
    display: none;
  }

  .day-header {
    display: flex;
    align-items: center;
    gap: 0.65rem;
    margin-left: -2rem;
  }

  .day-marker {
    width: 1.6rem;
    height: 1.6rem;
    border-radius: 50%;
    background: var(--surface-2);
    border: 2px solid var(--border);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 1;
    font-size: 0.65rem;
    font-weight: 700;
    color: var(--text);
  }

  .unscheduled-marker {
    background: var(--surface-1);
    color: var(--text-muted);
  }

  .day-titles {
    display: flex;
    align-items: baseline;
    gap: 0.5rem;
  }

  .day-titles h4 {
    margin: 0;
    font-size: var(--text-sm);
    font-weight: 600;
    color: var(--text);
  }

  .day-meta {
    font-size: var(--text-xs);
    color: var(--text-muted);
  }

  .day-content {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }

  .timeline-card {
    display: flex;
    align-items: center;
    gap: 0.65rem;
    padding: 0.55rem 0.75rem;
    background: var(--surface-1);
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    transition: border-color 0.15s ease;
  }

  .timeline-card:hover {
    border-color: var(--accent);
  }

  .stage-card {
    background: var(--surface-2);
    border-left: 3px solid var(--accent);
  }

  .card-icon {
    width: 1.75rem;
    height: 1.75rem;
    border-radius: var(--radius-sm);
    display: flex;
    align-items: center;
    justify-content: center;
    background: var(--surface-2);
    color: var(--text);
    flex-shrink: 0;
  }

  .stage-icon {
    background: var(--accent-soft, rgba(59, 130, 246, 0.1));
    color: var(--accent);
  }

  .card-thumb {
    width: 2rem;
    height: 2rem;
    border-radius: var(--radius-sm);
    object-fit: cover;
    flex-shrink: 0;
  }

  .card-body {
    display: flex;
    flex-direction: column;
    gap: 0.15rem;
    min-width: 0;
    flex: 1;
  }

  .card-topline {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    font-size: var(--text-xs);
  }

  .badge {
    display: inline-block;
    padding: 0.1rem 0.4rem;
    border-radius: var(--radius-sm);
    font-size: 0.65rem;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    background: var(--surface-2);
    color: var(--text-muted);
  }

  .badge-stage {
    background: var(--accent-soft, rgba(59, 130, 246, 0.15));
    color: var(--accent);
  }

  .badge-journey,
  .badge-transport {
    background: rgba(16, 185, 129, 0.15);
    color: #10b981;
  }

  .badge-event {
    background: rgba(245, 158, 11, 0.15);
    color: #f59e0b;
  }

  .badge-activity {
    background: rgba(139, 92, 246, 0.15);
    color: #8b5cf6;
  }

  .badge-stay {
    background: rgba(236, 72, 153, 0.15);
    color: #ec4899;
  }

  .stage-status {
    font-size: var(--text-xs);
    color: var(--text-muted);
    text-transform: capitalize;
  }

  .stage-route {
    display: flex;
    align-items: center;
    gap: 0.35rem;
    font-size: var(--text-sm);
  }

  .item-title {
    font-size: var(--text-sm);
    color: var(--text);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .item-time {
    display: inline-flex;
    align-items: center;
    gap: 0.2rem;
    color: var(--text-muted);
    font-size: 0.7rem;
  }

  .item-price {
    font-weight: 600;
    color: var(--text);
    font-size: 0.7rem;
    margin-left: auto;
  }

  .remove-btn {
    background: transparent;
    border: none;
    color: var(--text-muted);
    cursor: pointer;
    padding: 0.25rem;
    border-radius: var(--radius-sm);
    display: flex;
    align-items: center;
    justify-content: center;
    opacity: 0.6;
    transition: opacity 0.15s ease, color 0.15s ease;
  }

  .remove-btn:hover {
    opacity: 1;
    color: var(--danger);
  }

  .empty-day-placeholder {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.35rem 0.5rem;
    color: var(--text-muted);
    font-size: var(--text-xs);
    font-style: italic;
    opacity: 0.7;
  }

  .empty-dash {
    width: 12px;
    height: 1px;
    background: var(--border);
  }

  .empty-day-placeholder p {
    margin: 0;
  }

  .calendar-commitments {
    display: flex;
    align-items: center;
    gap: 0.35rem;
    flex-wrap: wrap;
    padding: 0.25rem 0.5rem;
    border-radius: var(--radius-sm);
    background: var(--surface-1);
    border: 1px dashed var(--border);
  }

  .commitments-label {
    font-size: 0.65rem;
    font-weight: 600;
    color: var(--text-muted);
  }

  .commitment-pill {
    display: inline-flex;
    align-items: center;
    gap: 0.25rem;
    padding: 0.1rem 0.4rem;
    border-radius: var(--radius-full, 9999px);
    font-size: 0.65rem;
    background: var(--surface-2);
    color: var(--text);
  }

  .commitment-committed {
    border-left: 2px solid #ef4444;
  }

  .commitment-planned {
    border-left: 2px solid #f59e0b;
  }

  .commitment-time {
    color: var(--text-muted);
    font-size: 0.6rem;
  }

  .stage-card-open {
    border-left-color: #f59e0b;
    background: rgba(245, 158, 11, 0.05);
  }

  .stage-icon-open {
    background: rgba(245, 158, 11, 0.15);
    color: #f59e0b;
  }

  .badge-open {
    background: rgba(245, 158, 11, 0.15);
    color: #f59e0b;
    border: 1px solid rgba(245, 158, 11, 0.3);
  }

  .branch-note {
    display: flex;
    align-items: center;
    gap: 0.3rem;
    margin-top: 0.2rem;
    font-size: 0.7rem;
    color: #f59e0b;
  }

  .card-actions {
    display: flex;
    align-items: center;
    gap: 0.35rem;
  }

  .day-assign-select {
    padding: 0.15rem 0.35rem;
    border-radius: var(--radius-sm);
    border: 1px solid var(--border);
    background: var(--surface-2);
    color: var(--text-muted);
    font-size: 0.65rem;
    cursor: pointer;
    outline: none;
    transition: all 0.15s ease;
  }

  .day-assign-select:hover {
    color: var(--text);
    border-color: var(--accent);
  }

  .unscheduled-assign {
    background: var(--accent-soft, rgba(59, 130, 246, 0.1));
    color: var(--accent);
    border-color: var(--accent);
    font-weight: 600;
  }
</style>
