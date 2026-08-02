<script lang="ts">
  import { calendar, type CalendarEntry } from "$lib/api";
  import { KINDS } from "./types";

  let {
    refresh,
    onChanged,
  }: {
    refresh: number;
    onChanged: () => Promise<void>;
  } = $props();

  function dateKey(date: Date): string {
    return [date.getFullYear(), String(date.getMonth() + 1).padStart(2, "0"), String(date.getDate()).padStart(2, "0")].join("-");
  }

  function plusDays(date: Date, days: number): string {
    const next = new Date(date);
    next.setDate(next.getDate() + days);
    return dateKey(next);
  }

  const today = new Date();
  const from = dateKey(today);
  const to = plusDays(today, 90);
  let proposals = $state<CalendarEntry[]>([]);
  let proposedKinds = $state<Record<string, string>>({});
  let loading = $state(false);
  let actingId = $state<string | null>(null);
  let error = $state("");
  let seenRefresh = -1;

  async function load() {
    loading = true;
    error = "";
    try {
      proposals = await calendar.proposals(from, to);
    } catch (cause) {
      error = String(cause);
    } finally {
      loading = false;
    }
  }

  $effect(() => {
    if (refresh === seenRefresh) return;
    seenRefresh = refresh;
    void load();
  });

  function kindFor(entry: CalendarEntry): string {
    return proposedKinds[entry.id] ?? entry.kind;
  }

  function setKind(entry: CalendarEntry, kind: string) {
    proposedKinds = { ...proposedKinds, [entry.id]: kind };
  }

  function when(entry: CalendarEntry): string {
    return entry.all_day ? `${entry.starts_at.slice(0, 10)} (all day)` : `${entry.starts_at} – ${entry.ends_at}`;
  }

  async function adopt(entry: CalendarEntry) {
    actingId = entry.id;
    error = "";
    try {
      await calendar.entries.update(entry.id, { kind: kindFor(entry), commitment: "planned" });
      await Promise.all([load(), onChanged()]);
    } catch (cause) {
      error = String(cause);
    } finally {
      actingId = null;
    }
  }

  async function remove(entry: CalendarEntry) {
    if (!window.confirm(`Remove “${entry.title}” from Axon only? The source remains unchanged.`)) return;
    actingId = entry.id;
    error = "";
    try {
      await calendar.entries.delete(entry.id);
      await Promise.all([load(), onChanged()]);
    } catch (cause) {
      error = String(cause);
    } finally {
      actingId = null;
    }
  }
</script>

<section class="inbox" aria-labelledby="calendar-proposals-title">
  <div class="heading">
    <div>
      <p class="eyebrow">Calendar</p>
      <h2 id="calendar-proposals-title">Proposed events</h2>
      <p>Review individual external events here, whether or not they ever become a trip.</p>
    </div>
    <button class="btn" onclick={load} disabled={loading || actingId !== null}>Refresh</button>
  </div>

  {#if error}<p class="message error" role="alert">{error}</p>{/if}

  {#if loading}
    <p class="empty">Loading proposals…</p>
  {:else if proposals.length === 0}
    <p class="empty">No pending calendar proposals in the next 90 days.</p>
  {:else}
    <div class="proposal-list">
      {#each proposals as entry (entry.id)}
        <article class="proposal">
          <div class="details">
            <strong>{entry.title}</strong>
            <p>{when(entry)}{#if entry.location} · {entry.location}{/if}</p>
            <p class="source">Source: {entry.source}</p>
          </div>
          <label>
            <span>As</span>
            <select value={kindFor(entry)} onchange={(event) => setKind(entry, event.currentTarget.value)} disabled={actingId !== null}>
              {#each KINDS as kind}<option value={kind.value}>{kind.label}</option>{/each}
            </select>
          </label>
          <div class="actions">
            <button class="btn primary" onclick={() => adopt(entry)} disabled={actingId !== null}>{actingId === entry.id ? "Adding…" : "Add"}</button>
            <button class="btn" onclick={() => remove(entry)} disabled={actingId !== null}>Remove</button>
          </div>
        </article>
      {/each}
    </div>
  {/if}
</section>

<style>
  .inbox { margin: 0 0 20px; padding: 18px; border: 1px solid var(--card-border); border-radius: 12px; background: var(--card-bg); }
  .heading, .proposal { display: flex; align-items: center; gap: 12px; }
  .heading { justify-content: space-between; align-items: flex-start; }
  .heading h2 { margin: 2px 0 4px; font-size: 1.1rem; }
  .heading p, .details p { margin: 0; color: var(--text-secondary); font-size: .86rem; }
  .eyebrow { color: var(--primary) !important; font-size: .72rem !important; font-weight: 700; letter-spacing: .08em; text-transform: uppercase; }
  .proposal-list { margin-top: 14px; border-top: 1px solid var(--card-border); }
  .proposal { padding: 11px 3px; border-bottom: 1px solid var(--card-border); }
  .details { min-width: 0; flex: 1; }
  .details strong { font-size: .9rem; }
  .details .source { margin-top: 3px; font-size: .75rem; }
  label { display: grid; gap: 4px; color: var(--text-secondary); font-size: .75rem; }
  select { min-width: 7.5rem; border: 1px solid var(--card-border); border-radius: 6px; padding: 5px; background: var(--surface); color: var(--text-primary); font: inherit; }
  .actions { display: flex; gap: 6px; }
  .btn { padding: 7px 11px; border: 0; border-radius: 7px; background: var(--surface); color: var(--text-primary); font: inherit; font-size: .8rem; cursor: pointer; }
  .btn:disabled { opacity: .5; cursor: wait; }
  .primary { background: var(--primary); color: #fff; }
  .message { margin: 12px 0 0; padding: 8px 10px; border-radius: 7px; font-size: .84rem; }
  .error { color: #be123c; background: #fff1f2; }
  .empty { margin: 14px 0 0; color: var(--text-secondary); font-size: .86rem; }
  @media (max-width: 720px) { .heading, .proposal { display: block; } .heading .btn, .proposal > label, .proposal .actions { margin-top: 10px; } }
</style>
