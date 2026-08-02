<script lang="ts">
  import { calendar, type CalendarEntry } from "$lib/api";
  import { KINDS } from "./types";

  let {
    refresh,
    onChanged,
  }: {
    /** Raised by the import review after it creates a draft. */
    refresh: number;
    onChanged: () => Promise<void>;
  } = $props();

  function dateKey(date: Date): string {
    return [
      date.getFullYear(),
      String(date.getMonth() + 1).padStart(2, "0"),
      String(date.getDate()).padStart(2, "0"),
    ].join("-");
  }

  function plusDays(date: Date, days: number): string {
    const shifted = new Date(date);
    shifted.setDate(shifted.getDate() + days);
    return dateKey(shifted);
  }

  const today = new Date();
  const from = dateKey(today);
  const to = plusDays(today, 90);
  let drafts = $state<CalendarEntry[]>([]);
  let proposedKinds = $state<Record<string, string>>({});
  let loading = $state(false);
  let actingId = $state<string | null>(null);
  let error = $state("");
  let seenRefresh = -1;

  async function loadDrafts() {
    loading = true;
    error = "";
    try {
      drafts = await calendar.google.drafts(from, to);
    } catch (cause) {
      error = String(cause);
    } finally {
      loading = false;
    }
  }

  $effect(() => {
    if (refresh === seenRefresh) return;
    seenRefresh = refresh;
    void loadDrafts();
  });

  function kindFor(entry: CalendarEntry): string {
    return proposedKinds[entry.id] ?? entry.kind;
  }

  function setKind(entry: CalendarEntry, kind: string) {
    proposedKinds = { ...proposedKinds, [entry.id]: kind };
  }

  function when(entry: CalendarEntry): string {
    if (entry.all_day) return `${entry.starts_at} (all day)`;
    return `${entry.starts_at} – ${entry.ends_at}`;
  }

  async function adopt(entry: CalendarEntry) {
    actingId = entry.id;
    error = "";
    try {
      // Raising commitment is the durable adoption signal: a later Google
      // refresh can no longer overwrite this entry even if its kind stays busy.
      await calendar.entries.update(entry.id, {
        kind: kindFor(entry),
        commitment: "planned",
      });
      await Promise.all([loadDrafts(), onChanged()]);
    } catch (cause) {
      error = String(cause);
    } finally {
      actingId = null;
    }
  }

  async function remove(entry: CalendarEntry) {
    if (!window.confirm(`Remove “${entry.title}” from Axon only? Google remains unchanged.`)) return;
    actingId = entry.id;
    error = "";
    try {
      await calendar.entries.delete(entry.id);
      await Promise.all([loadDrafts(), onChanged()]);
    } catch (cause) {
      error = String(cause);
    } finally {
      actingId = null;
    }
  }
</script>

<section class="inbox" aria-labelledby="google-draft-title">
  <div class="heading">
    <div>
      <p class="eyebrow">Google Calendar</p>
      <h2 id="google-draft-title">Review drafts</h2>
      <p>Adding an entry protects it from later Google updates. Removing it deletes only Axon's copy.</p>
    </div>
    <button class="btn" onclick={loadDrafts} disabled={loading || actingId !== null}>Refresh</button>
  </div>

  {#if error}<p class="message error" role="alert">{error}</p>{/if}

  {#if loading}
    <p class="empty">Loading Google drafts…</p>
  {:else if drafts.length === 0}
    <p class="empty">No pending Google drafts in the next 90 days.</p>
  {:else}
    <div class="draft-list">
      {#each drafts as entry (entry.id)}
        <article class="draft">
          <div class="details">
            <strong>{entry.title}</strong>
            <p>{when(entry)}{#if entry.location} · {entry.location}{/if}</p>
          </div>
          <label>
            <span>As</span>
            <select value={kindFor(entry)} onchange={(event) => setKind(entry, event.currentTarget.value)} disabled={actingId !== null}>
              {#each KINDS as kind}
                <option value={kind.value}>{kind.label}</option>
              {/each}
            </select>
          </label>
          <div class="actions">
            <button class="btn primary" onclick={() => adopt(entry)} disabled={actingId !== null}>
              {actingId === entry.id ? "Saving…" : "Add"}
            </button>
            <button class="btn danger" onclick={() => remove(entry)} disabled={actingId !== null}>Remove</button>
          </div>
        </article>
      {/each}
    </div>
  {/if}
</section>

<style>
  .inbox { margin: 0 0 20px; padding: 18px; border: 1px solid var(--card-border); border-radius: 12px; background: var(--card-bg); }
  .heading, .draft, .actions { display: flex; align-items: center; gap: 10px; }
  .heading { justify-content: space-between; align-items: flex-start; }
  .heading h2 { margin: 2px 0 4px; font-size: 1.1rem; }
  .heading p { margin: 0; color: var(--text-secondary); font-size: .86rem; max-width: 48rem; }
  .eyebrow { color: var(--primary) !important; font-size: .72rem !important; font-weight: 700; letter-spacing: .08em; text-transform: uppercase; }
  .message { margin: 12px 0 0; padding: 8px 10px; border-radius: 7px; font-size: .84rem; }
  .error { color: #be123c; background: #fff1f2; }
  .empty { margin: 14px 0 0; color: var(--text-secondary); font-size: .86rem; }
  .draft-list { margin-top: 14px; border-top: 1px solid var(--card-border); }
  .draft { padding: 10px 3px; border-bottom: 1px solid var(--card-border); }
  .details { flex: 1; min-width: 0; }
  .details strong { font-size: .9rem; }
  .details p { margin: 3px 0 0; color: var(--text-secondary); font-size: .8rem; }
  label { display: inline-flex; align-items: center; gap: 5px; color: var(--text-secondary); font-size: .78rem; }
  select { max-width: 10rem; padding: 5px 7px; border: 1px solid var(--card-border); border-radius: 6px; background: var(--surface); color: var(--text-primary); font: inherit; }
  .actions { justify-content: flex-end; }
  .danger { color: #be123c; }
  @media (max-width: 720px) {
    .heading, .draft { display: block; }
    .heading .btn, .draft label, .draft .actions { margin-top: 10px; }
  }
</style>
