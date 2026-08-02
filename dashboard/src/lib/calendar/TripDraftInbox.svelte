<script lang="ts">
  import { calendar, type CalendarTripDraft, type CalendarTripDrafts } from "$lib/api";

  let { refresh = 0 }: { refresh?: number } = $props();

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
  let seenRefresh = -1;

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
  }

  $effect(() => {
    if (refresh === seenRefresh) return;
    seenRefresh = refresh;
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

<section class="trip-drafts" aria-labelledby="trip-drafts-title">
  <div class="heading">
    <div>
      <p class="eyebrow">Travel</p>
      <h2 id="trip-drafts-title">Trip drafts from Calendar</h2>
      <p>Similar events in one place are only proposed. A Travel plan is created only when you choose Create trip.</p>
    </div>
    <button class="btn" onclick={load} disabled={loading || materializing !== null}>
      {loading ? "Checking…" : "Refresh"}
    </button>
  </div>

  <div class="range">
    <label>From <input type="date" bind:value={from} disabled={loading || materializing !== null} /></label>
    <label>To <input type="date" bind:value={to} disabled={loading || materializing !== null} /></label>
  </div>

  {#if error}
    <p class="message error" role="alert">{error}</p>
  {:else if notice}
    <p class="message success">{notice}</p>
  {/if}

  {#if result?.drafts.length}
    <div class="draft-list">
      {#each result.drafts as draft (keyFor(draft))}
        {@const key = keyFor(draft)}
        {@const planId = materialized.get(key)}
        <article class="draft">
          <div class="draft-topline">
            <div>
              <h3>{draft.place}</h3>
              <p>{draft.starts_on} – {inclusiveEnd(draft.ends_before)} · {draft.titles.length} Fixpunkt{draft.titles.length === 1 ? "" : "e"}</p>
            </div>
            <span class="commitment {draft.commitment}">{commitmentLabel(draft.commitment)}</span>
          </div>
          <ul>
            {#each draft.titles as title}
              <li>{title}</li>
            {/each}
          </ul>
          {#if planId}
            <p class="created">Created as a trip. <a href={`/travel?plan=${encodeURIComponent(planId)}`}>Open Travel</a></p>
          {:else}
            <div class="create-row">
              <label>
                <span>Trip title</span>
                <input value={titleFor(draft)} onchange={(event) => setTitle(draft, event.currentTarget.value)} />
              </label>
              <button class="btn primary" onclick={() => materialize(draft)} disabled={materializing !== null}>
                {materializing === key ? "Creating…" : "Create trip"}
              </button>
            </div>
          {/if}
        </article>
      {/each}
    </div>
  {:else if !loading && result}
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
</section>

<style>
  .trip-drafts { margin: 0 0 20px; padding: 18px; border: 1px solid var(--card-border); border-radius: 12px; background: var(--card-bg); }
  .heading, .draft-topline, .create-row, .range { display: flex; align-items: center; gap: 10px; }
  .heading, .draft-topline { justify-content: space-between; align-items: flex-start; }
  .heading h2, .draft h3 { margin: 2px 0 4px; }
  .heading h2 { font-size: 1.1rem; }
  .heading p, .draft p { margin: 0; color: var(--text-secondary); font-size: .86rem; }
  .eyebrow { color: var(--primary) !important; font-size: .72rem !important; font-weight: 700; letter-spacing: .08em; text-transform: uppercase; }
  .range { margin-top: 14px; flex-wrap: wrap; }
  .range label, .create-row label { color: var(--text-secondary); font-size: .82rem; }
  input { box-sizing: border-box; border: 1px solid var(--card-border); border-radius: 6px; padding: 6px 8px; background: var(--surface); color: var(--text-primary); font: inherit; }
  .btn { padding: 7px 12px; border: 0; border-radius: 7px; background: var(--surface); color: var(--text-primary); font: inherit; font-size: .82rem; cursor: pointer; }
  .btn:disabled { opacity: .5; cursor: wait; }
  .primary { background: var(--primary); color: #fff; }
  .message { margin: 12px 0 0; padding: 8px 10px; border-radius: 7px; font-size: .84rem; }
  .error { color: #be123c; background: #fff1f2; }
  .success { color: #166534; background: #f0fdf4; }
  .draft-list { margin-top: 14px; border-top: 1px solid var(--card-border); }
  .draft { padding: 14px 0; border-bottom: 1px solid var(--card-border); }
  .draft h3 { font-size: .95rem; }
  .commitment { padding: 3px 7px; border-radius: 999px; background: var(--surface); color: var(--text-secondary); font-size: .72rem; white-space: nowrap; }
  .commitment.planned { color: #92400e; background: #fef3c7; }
  .commitment.committed { color: #166534; background: #dcfce7; }
  ul { margin: 10px 0; padding-left: 20px; color: var(--text-secondary); font-size: .84rem; }
  li + li { margin-top: 4px; }
  .create-row { justify-content: space-between; align-items: end; flex-wrap: wrap; }
  .create-row label { display: grid; gap: 5px; min-width: min(18rem, 100%); }
  .created { color: #166534 !important; }
  a { color: var(--primary); }
  .empty { margin: 14px 0 0; color: var(--text-secondary); font-size: .86rem; }
  .unclustered { margin-top: 14px; color: var(--text-secondary); font-size: .82rem; }
  .unclustered summary { cursor: pointer; }
  @media (max-width: 640px) { .heading, .draft-topline { display: block; } .heading .btn { margin-top: 10px; } .commitment { display: inline-block; margin-top: 8px; } .create-row label { width: 100%; } }
</style>
