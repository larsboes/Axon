<script lang="ts">
  import { link } from "$lib/nav";
  import type { PendingRetrospective } from "$lib/travel/api";

  /**
   * The ladder row `kinds/trip-retrospective.ts` names through `view`.
   *
   * MERGE NOTE, 2026-09-05, updated 2026-09-06: this file lands in a directory
   * the dashboard-refresh stream owns and had not created yet, so it cannot
   * import that stream's `ListRow` or its `DecisionRowProps`. It is written to
   * be a correct list child on its own: the ROOT IS AN `<li>` carrying `id`,
   * `tabindex="-1"` and `aria-current`, because Home renders rows directly
   * inside `<ul class="queue">` and moves the keyboard cursor with
   * `document.getElementById(rowId(decision))`. A `<div>` with no id is invalid
   * inside a `<ul>` and is unreachable by J/K.
   *
   * AT MERGE: replace the `<li>` with dashboard-refresh's `ListRow`, forwarding
   * `{id} {current} {tone} {href}`, exactly as `rows/TripRow.svelte` does. The
   * props below are already that stream's names, so nothing else changes.
   *
   * The action is a link rather than an inline form: the three fields are a
   * human's considered answer about a trip that is over, and the page that shows
   * the trip is where they belong. `act()` is deliberately unused.
   */
  let {
    row,
    id = undefined,
    current = false,
    tone = "none",
    busy = false,
  }: {
    row: PendingRetrospective;
    /** The ladder's `${kind}:${id}` key. The cursor resolves the element by it. */
    id?: string;
    current?: boolean;
    tone?: "alarm" | "now" | "owed" | "offer" | "none";
    busy?: boolean;
    /** Accepted and unused: this row's answer is three fields on another page. */
    act?: (run: () => Promise<void>, options?: { dismiss?: boolean }) => void;
    /** Everything else Home passes every row. Accepted and not destructured, so
     *  an unknown prop is not a type error while the two streams are still
     *  separate. */
    [key: string]: unknown;
  } = $props();
</script>

<li class="row" {id} tabindex="-1" aria-current={current ? "true" : undefined} data-tone={tone}>
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
</li>

<style>
  .row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.75rem;
    width: 100%;
    list-style: none;
  }

  /* The cursor focuses the row itself; the outline is the only thing that says
     where the keyboard is. */
  .row:focus-visible {
    outline: 2px solid var(--focus-ring, currentColor);
    outline-offset: 2px;
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
