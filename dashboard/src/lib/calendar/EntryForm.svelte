<script lang="ts">
  import { onMount } from "svelte";
  import { COMMITMENTS, KINDS, type Commitment } from "./types";
  import type {
    CalendarEntry,
    CalendarGoogleExportOptIn,
    CalendarNewEntry,
    CalendarUpdateEntry,
  } from "$lib/api";

  let {
    entry,
    draft,
    date,
    rangeEndDate,
    eyebrow,
    notice,
    googleExport,
    onSave,
    onGoogleExportChange,
    onDelete,
    onClose,
  }: {
    entry?: CalendarEntry;
    draft?: CalendarNewEntry;
    date?: string;
    rangeEndDate?: string;
    /** Overrides the kicker above the heading — a draft can come from
     * Discover or from an empty slot in a time view. */
    eyebrow?: string;
    notice?: string;
    googleExport?: CalendarGoogleExportOptIn;
    onSave: (data: CalendarNewEntry | CalendarUpdateEntry) => Promise<void>;
    onGoogleExportChange?: (entry: CalendarEntry, optedIn: boolean) => Promise<void>;
    onDelete?: () => Promise<void>;
    onClose: () => void;
  } = $props();

  let dialog: HTMLDivElement;
  let titleInput: HTMLInputElement;
  let kind = $state("busy");
  // Typing something in by hand means it is real; scouting's promotions are
  // the ones that arrive as mere possibilities. The wire default is
  // `possible`, so this form says what it means instead of inheriting it.
  let commitment = $state<Commitment>("committed");
  let title = $state("");
  let allDay = $state(true);
  let startDate = $state("");
  // Inclusive in the UI. The HTTP contract remains exclusive for all-day entries.
  let endDate = $state("");
  let startTime = $state("");
  let endTime = $state("");
  let location = $state("");
  let notes = $state("");
  let saving = $state(false);
  let updatingGoogleExport = $state(false);
  let googleExported = $state(false);
  let error = $state("");

  function shiftDate(value: string, days: number): string {
    const shifted = new Date(`${value}T12:00:00`);
    shifted.setDate(shifted.getDate() + days);
    return [
      shifted.getFullYear(),
      String(shifted.getMonth() + 1).padStart(2, "0"),
      String(shifted.getDate()).padStart(2, "0"),
    ].join("-");
  }

  function initialise() {
    kind = entry?.kind ?? draft?.kind ?? "busy";
    commitment = entry?.commitment ?? "committed";
    title = entry?.title ?? draft?.title ?? "";
    allDay = entry?.all_day ?? draft?.all_day ?? true;
    startDate = entry?.starts_at.slice(0, 10) ?? draft?.starts_at.slice(0, 10) ?? date ?? "";
    endDate = entry
      ? entry.all_day
        ? shiftDate(entry.ends_at.slice(0, 10), -1)
        : entry.ends_at.slice(0, 10)
      : draft
        ? draft.all_day
          ? shiftDate(draft.ends_at.slice(0, 10), -1)
          : draft.ends_at.slice(0, 10)
        : rangeEndDate ?? date ?? "";
    startTime = entry && !entry.all_day
      ? entry.starts_at.slice(11, 16)
      : draft && !draft.all_day
        ? draft.starts_at.slice(11, 16)
        : "09:00";
    endTime = entry && !entry.all_day
      ? entry.ends_at.slice(11, 16)
      : draft && !draft.all_day
        ? draft.ends_at.slice(11, 16)
        : "10:00";
    location = entry?.location ?? draft?.location ?? "";
    notes = entry?.notes ?? draft?.notes ?? "";
  }

  onMount(() => {
    initialise();
    googleExported = Boolean(googleExport);
    dialog.focus();
    titleInput.focus();
  });

  function handleKeydown(event: KeyboardEvent) {
    if (event.key === "Escape" && !saving) onClose();
  }

  function googleExportUnavailable(): string | null {
    if (!entry) return null;
    if (entry.source === "google") return "This entry came from Google and will not be exported back.";
    if (entry.rhythm_id) return "Rhythm instances are not exported to Google individually.";
    return null;
  }

  async function toggleGoogleExport() {
    if (!entry || !onGoogleExportChange || googleExportUnavailable()) return;
    const next = !googleExported;
    updatingGoogleExport = true;
    error = "";
    try {
      await onGoogleExportChange(entry, next);
      googleExported = next;
    } catch (cause) {
      error = String(cause);
    } finally {
      updatingGoogleExport = false;
    }
  }

  async function save() {
    if (!title.trim()) {
      error = "Title is required";
      return;
    }
    if (!startDate || !endDate) {
      error = "Date is required";
      return;
    }
    if (endDate < startDate) {
      error = "The end must be on or after the start";
      return;
    }

    error = "";
    saving = true;
    try {
      const common = {
        kind,
        commitment,
        title: title.trim(),
        all_day: allDay,
        location: location.trim() || null,
        notes: notes.trim() || null,
        ...(!entry && draft
          ? {
              source: draft.source,
              external_id: draft.external_id,
              rhythm_id: draft.rhythm_id,
              payload: draft.payload,
            }
          : {}),
      };
      if (allDay) {
        await onSave({
          ...common,
          starts_at: startDate,
          ends_at: shiftDate(endDate, 1),
        });
      } else {
        await onSave({
          ...common,
          starts_at: `${startDate}T${startTime || "09:00"}:00`,
          ends_at: `${endDate}T${endTime || "10:00"}:00`,
        });
      }
    } catch (cause) {
      error = String(cause);
    } finally {
      saving = false;
    }
  }

  async function deleteEntry() {
    if (!onDelete || !window.confirm(`Delete “${title}”?`)) return;
    error = "";
    saving = true;
    try {
      await onDelete();
    } catch (cause) {
      error = String(cause);
      saving = false;
    }
  }
</script>

<svelte:window onkeydown={handleKeydown} />

<div class="overlay">
  <button class="backdrop" aria-label="Close dialog" onclick={onClose}></button>
  <div
    class="sheet"
    bind:this={dialog}
    role="dialog"
    aria-modal="true"
    aria-labelledby="entry-form-title"
    tabindex="-1"
  >
    <div class="heading">
      <div>
        <p class="eyebrow">{eyebrow ?? (entry ? "Calendar entry" : draft ? "From Discover" : rangeEndDate ? "Date range" : "Calendar entry")}</p>
        <h2 id="entry-form-title">{entry ? "Edit entry" : "New entry"}</h2>
      </div>
      <button class="close" aria-label="Close dialog" onclick={onClose}>×</button>
    </div>

    {#if entry?.rhythm_id}
      <p class="notice">Changes detach this entry from its rhythm. The rhythm itself remains unchanged.</p>
    {:else if notice}
      <p class="notice">{notice}</p>
    {/if}

    {#if error}
      <p class="error" role="alert">{error}</p>
    {/if}

    <fieldset>
      <legend>Type</legend>
      <div class="kind-picker">
        {#each KINDS as option}
          <button
            type="button"
            class="kind-btn"
            class:selected={option.value === kind}
            style={`--kind-color: ${option.color}`}
            onclick={() => (kind = option.value)}
          >
            {option.label}
          </button>
        {/each}
      </div>
    </fieldset>

    <fieldset>
      <legend>Commitment</legend>
      <div class="kind-picker">
        {#each COMMITMENTS as option}
          <button
            type="button"
            class="kind-btn commitment-btn"
            class:selected={option.value === commitment}
            title={option.hint}
            onclick={() => (commitment = option.value)}
          >
            {option.label}
          </button>
        {/each}
      </div>
      <p class="fieldnote">{COMMITMENTS.find((c) => c.value === commitment)?.hint}</p>
    </fieldset>

    <label>
      <span>Title</span>
      <input bind:this={titleInput} type="text" bind:value={title} placeholder="What for?" />
    </label>

    <label class="checkbox">
      <input type="checkbox" bind:checked={allDay} />
      <span>All day</span>
    </label>

    <div class="dates">
      <label>
        <span>Start</span>
        <input type="date" bind:value={startDate} />
        {#if !allDay}
          <input type="time" bind:value={startTime} />
        {/if}
      </label>
      <label>
        <span>{allDay ? "Last day" : "End"}</span>
        <input type="date" bind:value={endDate} min={startDate} />
        {#if !allDay}
          <input type="time" bind:value={endTime} />
        {/if}
      </label>
    </div>

    <label>
      <span>Location</span>
      <input type="text" bind:value={location} placeholder="Optional" />
    </label>

    <label>
      <span>Note</span>
      <textarea bind:value={notes} rows="3" placeholder="Optional"></textarea>
    </label>

    {#if entry}
      <fieldset class="google-export">
        <legend>Google Calendar</legend>
        {#if googleExportUnavailable()}
          <p class="fieldnote">{googleExportUnavailable()}</p>
        {:else}
          <label class="checkbox">
            <input
              type="checkbox"
              checked={googleExported}
              disabled={updatingGoogleExport || saving}
              onchange={toggleGoogleExport}
            />
            <span>Approve for Google export</span>
          </label>
          <p class="fieldnote">
            {googleExported
              ? "In the export queue. The entry is sent only when you run Google sync."
              : "Not yet approved for Google."}
          </p>
        {/if}
      </fieldset>
    {/if}

    <div class="actions">
      {#if entry && onDelete}
        <button class="btn danger" onclick={deleteEntry} disabled={saving}>
          Delete
        </button>
      {/if}
      <div class="spacer"></div>
      <button class="btn secondary" onclick={onClose} disabled={saving}>Cancel</button>
      <button class="btn primary" onclick={save} disabled={saving}>
        {saving ? "Saving…" : "Save"}
      </button>
    </div>
  </div>
</div>

<style>
  .overlay {
    position: fixed;
    inset: 0;
    z-index: 100;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 20px;
  }

  .backdrop {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    border: 0;
    background: rgba(0, 0, 0, 0.48);
    cursor: default;
    animation: fade-in 0.15s ease-out;
  }

  .sheet {
    position: relative;
    width: min(520px, 100%);
    max-height: 90vh;
    overflow-y: auto;
    padding: 24px;
    border: 1px solid var(--card-border);
    border-radius: 14px;
    background: var(--card-bg);
    box-shadow: 0 16px 48px rgba(0, 0, 0, 0.35);
  }

  .sheet:focus {
    outline: none;
  }

  .heading {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 16px;
    margin-bottom: 18px;
  }

  .eyebrow {
    margin: 0 0 3px;
    color: var(--text-secondary);
    font-size: 0.6875rem;
    font-weight: 700;
    letter-spacing: 0.08em;
    text-transform: uppercase;
  }

  h2 {
    margin: 0;
    font-size: 1.125rem;
  }

  .close {
    width: 30px;
    height: 30px;
    border: 0;
    border-radius: 50%;
    background: var(--surface);
    color: var(--text-primary);
    font-size: 1.25rem;
    line-height: 1;
    cursor: pointer;
  }

  label {
    display: flex;
    flex-direction: column;
    gap: 5px;
    margin-bottom: 13px;
    color: var(--text-secondary);
    font-size: 0.8125rem;
    font-weight: 600;
  }

  fieldset {
    min-width: 0;
    margin: 0 0 13px;
    padding: 0;
    border: 0;
  }

  legend {
    margin-bottom: 5px;
    padding: 0;
    color: var(--text-secondary);
    font-size: 0.8125rem;
    font-weight: 600;
  }

  .checkbox {
    flex-direction: row;
    align-items: center;
  }

  .kind-picker {
    display: flex;
    flex-wrap: wrap;
    gap: 5px;
  }

  .kind-btn {
    padding: 5px 10px;
    border: 1.5px solid var(--kind-color);
    border-radius: 6px;
    background: transparent;
    color: var(--kind-color);
    font-size: 0.75rem;
    cursor: pointer;
  }

  /* Neutral rather than kind-coloured: this axis is orthogonal to what the
     entry is, and colouring it the same way would suggest otherwise. */
  .commitment-btn {
    --kind-color: var(--text-secondary);
  }

  .fieldnote {
    margin: 6px 0 0;
    color: var(--text-secondary);
    font-size: 0.78em;
  }

  .google-export {
    margin-top: 18px;
    padding-top: 13px;
    border-top: 1px solid var(--card-border);
  }

  .kind-btn.selected {
    background: var(--kind-color);
    color: #fff;
  }

  input[type="text"],
  input[type="date"],
  input[type="time"],
  textarea {
    box-sizing: border-box;
    width: 100%;
    padding: 8px 10px;
    border: 1px solid var(--card-border);
    border-radius: 6px;
    background: var(--card-bg);
    color: var(--text-primary);
    font: inherit;
    font-size: 0.875rem;
  }

  textarea {
    resize: vertical;
  }

  .dates {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 12px;
  }

  .notice {
    margin: -5px 0 15px;
    padding: 9px 11px;
    border-radius: 7px;
    background: var(--surface);
    color: var(--text-secondary);
    font-size: 0.75rem;
    line-height: 1.45;
  }

  .actions {
    display: flex;
    gap: 8px;
    margin-top: 18px;
  }

  .spacer {
    flex: 1;
  }

  .btn {
    padding: 8px 16px;
    border: none;
    border-radius: 8px;
    font-size: 0.875rem;
    cursor: pointer;
  }

  .btn:disabled {
    opacity: 0.5;
    cursor: wait;
  }

  .primary {
    background: var(--primary);
    color: #fff;
  }

  .secondary {
    background: var(--surface);
    color: var(--text-primary);
  }

  .danger {
    background: #e11d48;
    color: #fff;
  }

  .error {
    margin: 0 0 12px;
    color: #e11d48;
    font-size: 0.8125rem;
  }

  @keyframes fade-in {
    from { opacity: 0; }
    to { opacity: 1; }
  }

  @media (max-width: 560px) {
    .sheet {
      padding: 20px;
    }

    .dates {
      grid-template-columns: 1fr;
      gap: 0;
    }
  }
</style>
