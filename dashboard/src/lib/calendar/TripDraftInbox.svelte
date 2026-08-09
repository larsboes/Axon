<script lang="ts">
  import { link } from "$lib/nav";
  import { calendar, type CalendarTripDraft, type CalendarTripDrafts } from "$lib/api";

  let {
    refresh = 0,
    onCount,
  }: {
    refresh?: number;
    /** See GoogleDraftInbox: the rail renders the badge, the section owns the window. */
    onCount: (count: number) => void;
  } = $props();

  function dateKey(date: Date): string {
    return [
      date.getFullYear(),
      String(date.getMonth() + 1).padStart(2, "0"),
      String(date.getDate()).padStart(2, "0"),
    ].join("-");
  }

  function plusDays(date: Date, days: number): string {
    const next = new Date(date);
    next.setDate(next.getDate() + days);
    return dateKey(next);
  }

  function inclusiveEnd(endsBefore: string): string {
    const end = new Date(`${endsBefore}T12:00:00`);
    end.setDate(end.getDate() - 1);
    return dateKey(end);
  }

  const today = new Date();
  let from = $state(dateKey(today));
  let to = $state(plusDays(today, 180));
  let result = $state<CalendarTripDrafts | null>(null);
  let loading = $state(false);
  let materializing = $state<string | null>(null);
  let error = $state("");
  let notice = $state("");
  let titles = $state<Map<string, string>>(new Map());
  let materialized = $state<Map<string, string>>(new Map());
  let seenKey = "";

  function keyFor(draft: CalendarTripDraft): string {
    return draft.entry_ids.join("|");
  }

  function defaultTitle(draft: CalendarTripDraft): string {
    return `${draft.place} — ${draft.starts_on}`;
  }

  function titleFor(draft: CalendarTripDraft): string {
    return titles.get(keyFor(draft)) ?? defaultTitle(draft);
  }

  function setTitle(draft: CalendarTripDraft, title: string) {
    titles = new Map(titles).set(keyFor(draft), title);
  }

  function commitmentLabel(commitment: CalendarTripDraft["commitment"]): string {
    return { possible: "Possible", planned: "Planned", committed: "Committed" }[commitment];
  }

  async function load() {
    loading = true;
    error = "";
    notice = "";
    try {
      result = await calendar.tripDrafts.list(from, to);
    } catch (cause) {
      error = String(cause);
      result = null;
    } finally {
      loading = false;
    }
    onCount(result?.drafts.length ?? 0);
  }

  // The window is part of the query, so editing a date reloads. It used to take a
  // manual Refresh, which made a changed date look like it had no effect.
  $effect(() => {
    const key = `${refresh}|${from}|${to}`;
    if (key === seenKey) return;
    seenKey = key;
    void load();
  });

  async function materialize(draft: CalendarTripDraft) {
    const key = keyFor(draft);
    const title = titleFor(draft).trim();
    if (!window.confirm(`Create “${title || defaultTitle(draft)}” as a trip in Travel?`)) return;
    materializing = key;
    error = "";
    notice = "";
    try {
      const response = await calendar.tripDrafts.materialize(draft.entry_ids, title);
      materialized = new Map(materialized).set(key, response.plan_id);
      notice = response.created
        ? `Trip created. The calendar entries remain unchanged.`
        : `These entries already belong to the existing trip.`;
    } catch (cause) {
      error = String(cause);
    } finally {
      materializing = null;
    }
  }
</script>

<p class="hint">Similar events in one place are only proposed. A Travel plan is created when you choose Create trip.</p>

<div class="range">
  <label>From <input type="date" bind:value={from} disabled={loading || materializing !== null} /></label>
  <label>To <input type="date" bind:value={to} disabled={loading || materializing !== null} /></label>
</div>

{#if error}
  <p class="message error" role="alert">{error}</p>
{:else if notice}
  <p class="message success">{notice}</p>
{/if}

{#if loading}
  <p class="empty">Checking…</p>
{:else if result?.drafts.length}
  <div class="draft-list">
    {#each result.drafts as draft (keyFor(draft))}
      {@const key = keyFor(draft)}
      {@const planId = materialized.get(key)}
      <article class="draft">
        <div class="draft-topline">
          <strong>{draft.place}</strong>
          <span class="commitment {draft.commitment}">{commitmentLabel(draft.commitment)}</span>
        </div>
        <p>{draft.starts_on} – {inclusiveEnd(draft.ends_before)} · {draft.titles.length} fixed point{draft.titles.length === 1 ? "" : "s"}</p>
        <ul>
          {#each draft.titles as title}
            <li>{title}</li>
          {/each}
        </ul>
        {#if planId}
          <p class="created">Created as a trip. <a href={link(`/travel?plan=${encodeURIComponent(planId)}`)}>Open Travel</a></p>
        {:else}
          <label class="title-field">
            <span>Trip title</span>
            <input value={titleFor(draft)} onchange={(event) => setTitle(draft, event.currentTarget.value)} />
          </label>
          <button class="btn btn-primary create" onclick={() => materialize(draft)} disabled={materializing !== null}>
            {materializing === key ? "Creating…" : "Create trip"}
          </button>
        {/if}
      </article>
    {/each}
  </div>
{:else if result}
  <p class="empty">No trip drafts in this period. Events at home or without a location are intentionally excluded.</p>
{/if}

{#if result?.unclustered.length}
  <details class="unclustered">
    <summary>{result.unclustered.length} events that could not be grouped</summary>
    <ul>
      {#each result.unclustered as entry (entry.entry_id)}
        <li><strong>{entry.title}</strong> — {entry.reason}</li>
      {/each}
    </ul>
  </details>
{/if}

<style>
  .hint { margin: 0 0 0.5rem; color: var(--text-secondary); font-size: 0.75rem; line-height: 1.45; }
  .range { display: flex; gap: 0.4rem; margin-bottom: 0.5rem; }
  .range label { flex: 1; min-width: 0; display: grid; gap: 0.2rem; color: var(--text-tertiary); font-size: 0.68rem; }
  input { width: 100%; box-sizing: border-box; padding: 0.25rem 0.35rem; border: 1px solid var(--input-border); border-radius: var(--radius-sm); background-color: var(--input-bg); color: var(--text-primary); font: inherit; font-size: 0.72rem; }
  .message { margin: 0 0 0.5rem; padding: 0.5rem 0.6rem; border-radius: var(--radius-sm); font-size: 0.78rem; }
  .error { color: var(--danger); background-color: var(--danger-soft); }
  .success { color: var(--success); background-color: var(--success-soft); }
  .empty { margin: 0; color: var(--text-tertiary); font-size: 0.78rem; }
  .draft-list { display: flex; flex-direction: column; }
  .draft { padding: 0.6rem 0; border-top: 1px solid var(--card-border); }
  .draft-topline { display: flex; align-items: center; justify-content: space-between; gap: 0.4rem; flex-wrap: wrap; }
  .draft-topline strong { min-width: 0; overflow-wrap: anywhere; }
  .draft-topline strong { font-size: 0.8125rem; font-weight: 600; }
  .draft p { margin: 0.15rem 0 0; color: var(--text-secondary); font-size: 0.72rem; }
  .commitment { padding: 0.1rem 0.35rem; border-radius: var(--radius-sm); background-color: var(--surface); color: var(--text-secondary); font-size: 0.65rem; white-space: nowrap; }
  .commitment.planned { color: var(--warning); background-color: var(--warning-soft); }
  .commitment.committed { color: var(--success); background-color: var(--success-soft); }
  ul { margin: 0.35rem 0 0; padding-left: 1rem; color: var(--text-secondary); font-size: 0.72rem; }
  li + li { margin-top: 0.15rem; }
  .title-field { display: grid; gap: 0.2rem; margin-top: 0.45rem; color: var(--text-tertiary); font-size: 0.68rem; }
  .create { width: 100%; margin-top: 0.4rem; padding: 0.3rem 0.5rem; font-size: 0.72rem; }
  .created { color: var(--success) !important; }
  a { color: var(--primary); }
  .unclustered { margin-top: 0.5rem; color: var(--text-tertiary); font-size: 0.72rem; }
  .unclustered summary { cursor: pointer; }
</style>
