<script lang="ts">
  import type {
    CalendarContext,
    CalendarNewContext,
    CalendarUpdateContext,
  } from "$lib/api";

  let {
    contexts,
    rangeLabel,
    defaultFrom,
    defaultUntil,
    openId = null,
    onCreate,
    onUpdate,
    onDelete,
  }: {
    contexts: CalendarContext[];
    rangeLabel: string;
    defaultFrom: string;
    defaultUntil: string;
    /** Deep-link target: open this context's editor as soon as it is loaded. */
    openId?: string | null;
    onCreate: (context: CalendarNewContext) => Promise<void>;
    onUpdate: (id: string, context: CalendarUpdateContext) => Promise<void>;
    onDelete: (id: string) => Promise<void>;
  } = $props();

  const kinds = [
    { value: "uncertainty", label: "Time window", color: "#7c3aed" },
    { value: "transition", label: "Transition", color: "#d97706" },
    { value: "preference", label: "Preference", color: "#0891b2" },
    { value: "planning_gap", label: "Open planning", color: "#db2777" },
    { value: "note", label: "Note", color: "#64748b" },
  ];

  let editing = $state<CalendarContext | null>(null);
  let showForm = $state(false);
  let kind = $state("uncertainty");
  let title = $state("");
  let details = $state("");
  let validFrom = $state("");
  let validUntil = $state("");
  let saving = $state(false);
  let error = $state("");

  function kindMeta(value: string) {
    return kinds.find((item) => item.value === value) ?? kinds[kinds.length - 1];
  }

  function openCreate() {
    editing = null;
    kind = "uncertainty";
    title = "";
    details = "";
    validFrom = defaultFrom;
    validUntil = defaultUntil;
    error = "";
    showForm = true;
  }

  function openEdit(context: CalendarContext) {
    editing = context;
    kind = context.kind;
    title = context.title;
    details = context.details;
    validFrom = context.valid_from;
    validUntil = context.valid_until;
    error = "";
    showForm = true;
  }

  function close() {
    if (!saving) showForm = false;
  }

  // A deep link names one context; open its editor once it is in the loaded
  // set. Consumed once, so closing the sheet does not immediately reopen it.
  let openedId = "";

  $effect(() => {
    if (!openId || openId === openedId) return;
    const context = contexts.find((candidate) => candidate.id === openId);
    if (!context) return;
    openedId = openId;
    openEdit(context);
  });

  async function save() {
    if (!title.trim()) {
      error = "Title is required";
      return;
    }
    if (!validFrom || !validUntil || validUntil < validFrom) {
      error = "Choose a valid period";
      return;
    }
    saving = true;
    error = "";
    try {
      const data = {
        kind,
        title: title.trim(),
        details: details.trim(),
        valid_from: validFrom,
        valid_until: validUntil,
      };
      if (editing) await onUpdate(editing.id, data);
      else await onCreate(data);
      showForm = false;
    } catch (cause) {
      error = String(cause);
    } finally {
      saving = false;
    }
  }

  async function remove() {
    if (!editing || !window.confirm(`Delete “${editing.title}”?`)) return;
    saving = true;
    error = "";
    try {
      await onDelete(editing.id);
      showForm = false;
    } catch (cause) {
      error = String(cause);
    } finally {
      saving = false;
    }
  }

  function dateLabel(value: string) {
    return new Date(`${value}T12:00:00`).toLocaleDateString("en-GB", {
      day: "numeric",
      month: "short",
      year: "numeric",
    });
  }
</script>

<p class="hint">{rangeLabel} · affects guidance and ranking but blocks no time.</p>

{#if contexts.length === 0}
  <p class="empty">Nothing noted for this period.</p>
{:else}
  <div class="context-list">
    {#each contexts as context (context.id)}
      {@const meta = kindMeta(context.kind)}
      <button
        type="button"
        class="context-card"
        style={`--context-color: ${meta.color}`}
        onclick={() => openEdit(context)}
      >
        <span class="context-kind">{meta.label}</span>
        <strong>{context.title}</strong>
        {#if context.details}<p>{context.details}</p>{/if}
        <small>{dateLabel(context.valid_from)} – {dateLabel(context.valid_until)}</small>
      </button>
    {/each}
  </div>
{/if}

<button type="button" class="btn btn-outline add" onclick={openCreate}>+ Context</button>

{#if showForm}
  <div class="overlay">
    <button class="backdrop" aria-label="Close dialog" onclick={close}></button>
    <div class="sheet" role="dialog" aria-modal="true" aria-labelledby="context-form-title">
      <div class="heading">
        <div>
          <p>Planning context</p>
          <h2 id="context-form-title">{editing ? "Edit context" : "New context"}</h2>
        </div>
        <button class="close" aria-label="Close dialog" onclick={close}>×</button>
      </div>

      {#if error}<p class="form-error" role="alert">{error}</p>{/if}

      <fieldset>
        <legend>Type</legend>
        <div class="kind-picker">
          {#each kinds as option (option.value)}
            <button
              type="button"
              class:selected={kind === option.value}
              style={`--context-color: ${option.color}`}
              onclick={() => (kind = option.value)}
            >
              {option.label}
            </button>
          {/each}
        </div>
      </fieldset>

      <label>
        <span>Title</span>
        <input type="text" bind:value={title} placeholder="What is not fixed in the calendar yet?" />
      </label>
      <label>
        <span>Context</span>
        <textarea bind:value={details} rows="3" placeholder="Why does this matter for planning?"></textarea>
      </label>
      <div class="dates">
        <label>
          <span>Relevant from</span>
          <input type="date" bind:value={validFrom} />
        </label>
        <label>
          <span>Relevant until</span>
          <input type="date" bind:value={validUntil} min={validFrom} />
        </label>
      </div>

      <div class="form-actions">
        {#if editing}
          <button class="danger" type="button" disabled={saving} onclick={remove}>Delete</button>
        {/if}
        <span></span>
        <button type="button" disabled={saving} onclick={close}>Cancel</button>
        <button class="primary" type="button" disabled={saving} onclick={save}>
          {saving ? "Saving…" : "Save"}
        </button>
      </div>
    </div>
  </div>
{/if}

<style>
  .hint { margin: 0 0 0.5rem; color: var(--text-secondary); font-size: 0.75rem; line-height: 1.45; }
  .empty { margin: 0; color: var(--text-tertiary); font-size: 0.78rem; }

  /* The overlay form still owns its own button styling; only the rail body defers to
   * the `.btn` primitive in app.css. Scoped rules would otherwise outrank it. */
  .overlay button,
  .context-card {
    border: 1px solid var(--card-border);
    border-radius: 7px;
    background: var(--card-bg);
    color: var(--text-primary);
    font: inherit;
    cursor: pointer;
  }

  h2 { margin: 0; font-size: 0.95rem; }

  .context-list {
    display: grid;
    gap: 0.35rem;
  }

  .context-card {
    display: grid;
    gap: 0.15rem;
    min-width: 0;
    padding: 0.5rem 0.6rem;
    border-left: 3px solid var(--context-color);
    text-align: left;
  }

  .context-card:hover { border-color: var(--context-color); background: var(--surface); }
  .context-kind { color: var(--context-color); font-size: 0.6rem; font-weight: 700; text-transform: uppercase; }
  .context-card strong { font-size: 0.78rem; }
  .context-card p { margin: 0; color: var(--text-secondary); font-size: 0.7rem; line-height: 1.4; }
  .context-card small { color: var(--text-tertiary); font-size: 0.62rem; }

  .add { width: 100%; margin-top: 0.5rem; padding: 0.3rem 0.5rem; font-size: 0.72rem; }

  .overlay {
    position: fixed;
    inset: 0;
    z-index: 110;
    display: grid;
    place-items: center;
    padding: 20px;
  }

  .backdrop { position: absolute; inset: 0; width: 100%; border: 0; border-radius: 0; background: rgba(0, 0, 0, 0.48); }
  .sheet { position: relative; width: min(520px, 100%); padding: 24px; border: 1px solid var(--card-border); border-radius: 14px; background: var(--card-bg); box-shadow: 0 16px 48px rgba(0,0,0,.35); }
  .heading { display: flex; justify-content: space-between; gap: 16px; margin-bottom: 18px; }
  .heading h2 { font-size: 1.1rem; }

  .heading p {
    display: block;
    margin: 0 0 3px;
    color: var(--primary);
    font-size: 0.65rem;
    font-weight: 700;
    letter-spacing: 0.08em;
    text-transform: uppercase;
  }

  .close { width: 30px; height: 30px; border-radius: 50%; font-size: 1.2rem; }

  fieldset { margin: 0 0 13px; padding: 0; border: 0; }
  legend, label span { margin-bottom: 5px; color: var(--text-secondary); font-size: 0.78rem; font-weight: 600; }
  .kind-picker { display: flex; flex-wrap: wrap; gap: 5px; }
  .kind-picker button { padding: 5px 9px; border-color: var(--context-color); color: var(--context-color); font-size: 0.7rem; }
  .kind-picker button.selected { background: var(--context-color); color: white; }
  label { display: flex; flex-direction: column; margin-bottom: 13px; }
  input, textarea { box-sizing: border-box; width: 100%; padding: 8px 10px; border: 1px solid var(--card-border); border-radius: 7px; background: var(--surface); color: var(--text-primary); font: inherit; }
  .dates { display: grid; grid-template-columns: 1fr 1fr; gap: 10px; }
  .form-error { color: var(--danger); font-size: 0.75rem; }
  .form-actions { display: grid; grid-template-columns: auto 1fr auto auto; gap: 7px; margin-top: 18px; }
  .form-actions button { padding: 7px 12px; font-size: 0.75rem; }
  .form-actions .primary { border-color: var(--primary); background: var(--primary); color: white; }
  .form-actions .danger { color: var(--danger); }

  @media (max-width: 38rem) {
    .dates { grid-template-columns: 1fr; }
  }
</style>
