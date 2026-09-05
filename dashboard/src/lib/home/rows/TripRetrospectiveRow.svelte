<script lang="ts">
  import { link } from "$lib/nav";
  import type { PendingRetrospective } from "$lib/travel/api";

  /**
   * The ladder row `kinds/trip-retrospective.ts` names through `view`.
   *
   * MERGE NOTE, 2026-09-05: this file lands in a directory the dashboard-refresh
   * stream owns and had not created yet. It is written to the `DecisionViewProps`
   * contract that stream published; if the prop shape moved before it merged,
   * this component is the one file that follows it.
   *
   * The action is a link rather than an inline form: the three fields are a
   * human's considered answer about a trip that is over, and the page that shows
   * the trip is where they belong. `act()` is deliberately unused.
   */
  let {
    row,
    busy = false,
  }: {
    row: PendingRetrospective;
    busy?: boolean;
    act?: (run: () => Promise<void>, options?: { dismiss?: boolean }) => void;
  } = $props();
</script>

<div class="row">
  <div>
    <a class="title" href={link(`/travel?plan=${encodeURIComponent(row.plan_id)}`)}>
      {row.title}
    </a>
    <p class="why">
      {#if row.destinations.length > 0}
        Back from {row.destinations.join(", ")} · {row.days_since_close} days ago
      {:else}
        Closed {row.days_since_close} days ago
      {/if}
    </p>
  </div>
  <a
    class="action"
    class:busy
    href={link(`/travel?plan=${encodeURIComponent(row.plan_id)}`)}
  >
    Record it
  </a>
</div>

<style>
  .row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.75rem;
    width: 100%;
  }

  .title {
    font-weight: 600;
    color: var(--text-primary);
    text-decoration: none;
  }

  .title:hover {
    text-decoration: underline;
  }

  .why {
    margin: 0.1rem 0 0;
    font-size: 0.75rem;
    color: var(--text-tertiary);
  }

  .action {
    flex: none;
    padding: 0.3rem 0.7rem;
    border: 1px solid var(--card-border);
    border-radius: var(--radius-sm);
    font-size: 0.8rem;
    color: var(--text-secondary);
    text-decoration: none;
  }

  .action.busy {
    opacity: 0.6;
  }
</style>
