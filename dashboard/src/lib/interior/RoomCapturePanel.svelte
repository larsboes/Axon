<script lang="ts">
  import { onMount } from "svelte";
  import {
    acceptPendingRoomPlan,
    cancelRoomPlanCapture,
    compareRoomPlanDrafts,
    currentRoomPlanState,
    importLegacyRoomPlanReference,
    isRoomPlanAvailable,
    previewImportedRoomPlanReference,
    rejectPendingRoomPlan,
    listenRoomPlan,
    startRoomPlanCapture,
    stopRoomPlanCapture,
    type RoomPlanCaptureMode,
    type RoomPlanDraft,
    type RoomPlanReference,
    type RoomPlanDiff,
  } from "$lib/roomplan";

  let available = $state<boolean | null>(null);
  let running = $state(false);
  let phase = $state("idle");
  let draft = $state<RoomPlanDraft | null>(null);
  let pending = $state<RoomPlanDraft | null>(null);
  let reference = $state<RoomPlanReference | null>(null);
  let diff = $state<RoomPlanDiff | null>(null);
  let captureMode = $state<RoomPlanCaptureMode | null>(null);
  let error = $state<string | null>(null);

  /**
   * The explanation is folded on a phone, where it filled the whole first screen, and open
   * above the phone breakpoint (38rem, `app.css`), where the summary is hidden and the text
   * reads as the paragraph it always was.
   */
  let explainOpen = $state(true);

  onMount(() => {
    const query = window.matchMedia("(width < 38rem)");
    const sync = () => (explainOpen = !query.matches);
    sync();
    query.addEventListener("change", sync);
    return () => query.removeEventListener("change", sync);
  });

  onMount(() => {
    const subscriptions = [
      listenRoomPlan<{ phase: string; draft_id?: string }>("capture-progress", (event) => {
        phase = event.phase;
        running = true;
      }),
      listenRoomPlan<{ draft: RoomPlanDraft }>("capture-completed", (event) => {
        captureMode = event.draft.provenance.capture_mode ?? captureMode;
        if (event.draft.provenance.capture_mode === "refine_existing") {
          pending = event.draft;
          diff = draft ? compareRoomPlanDrafts(draft, event.draft) : null;
          phase = "review";
        } else {
          draft = event.draft;
          phase = "complete";
        }
        running = false;
      }),
      listenRoomPlan<{ phase: string }>("capture-cancelled", () => {
        phase = "cancelled";
        running = false;
      }),
      listenRoomPlan<{ message: string }>("capture-failed", (event) => {
        error = event.message;
        phase = "failed";
        running = false;
      }),
    ];

    void isRoomPlanAvailable()
      .then((value) => (available = value))
      .catch(() => (available = false));
    void importLegacyRoomPlanReference()
      .then((value) => {
        if (value) reference = value;
      })
      .catch(() => undefined);
    void currentRoomPlanState()
      .then((value) => {
        draft = value.draft;
        pending = value.pending;
        reference = value.reference ?? reference;
        diff = draft && pending ? compareRoomPlanDrafts(draft, pending) : null;
        if (value.draft || value.pending || value.reference) phase = value.pending ? "review" : "loaded";
      })
      .catch(() => undefined);

    return () => {
      void Promise.all(subscriptions).then((listeners) =>
        Promise.all(listeners.map((listener) => listener.unregister())),
      );
    };
  });

  async function start(mode: RoomPlanCaptureMode) {
    error = null;
    captureMode = mode;
    try {
      await startRoomPlanCapture(false, mode);
      running = true;
      phase = mode === "refine_existing" ? "refining" : "capturing";
    } catch (cause) {
      error = cause instanceof Error ? cause.message : String(cause);
    }
  }

  async function stop() {
    try {
      await stopRoomPlanCapture();
      phase = "building";
    } catch (cause) {
      error = cause instanceof Error ? cause.message : String(cause);
    }
  }

  async function acceptPending() {
    try {
      await acceptPendingRoomPlan();
      draft = pending;
      pending = null;
      diff = null;
      phase = "accepted";
    } catch (cause) {
      error = cause instanceof Error ? cause.message : String(cause);
    }
  }

  async function rejectPending() {
    try {
      await rejectPendingRoomPlan();
      pending = null;
      diff = null;
      phase = "rejected";
    } catch (cause) {
      error = cause instanceof Error ? cause.message : String(cause);
    }
  }

  async function cancel() {
    try {
      await cancelRoomPlanCapture();
    } catch (cause) {
      error = cause instanceof Error ? cause.message : String(cause);
    }
  }
</script>

<section class="capture-panel" aria-labelledby="capture-heading">
  <div class="capture-heading">
    <div>
      <p class="eyebrow">Native RoomPlan</p>
      <h2 id="capture-heading">Capture the room</h2>
    </div>
    <span class:online={available === true} class="availability">
      {available === null ? "checking" : available ? "LiDAR ready" : "iPhone only"}
    </span>
  </div>

  <details class="explain" bind:open={explainOpen}>
    <summary>How capture works</summary>
    <p>
      The iPhone keeps capture offline. Refine existing creates a child revision for comparison;
      Start new scan creates a separate room revision. The native USDZ stays preserved and no
      capture replaces the measured apartment model automatically.
    </p>
  </details>

  <div class="actions">
    {#if !running}
      {#if reference || draft}
        <button disabled={available !== true || pending !== null} onclick={() => start("refine_existing")}>Refine existing</button>
      {/if}
      <button class="secondary" disabled={available !== true || pending !== null} onclick={() => start("new_room")}>
        {reference || draft ? "Start new scan" : "Start capture"}
      </button>
    {:else}
      <button class="secondary" onclick={stop}>Finish and build</button>
      <button class="quiet" onclick={cancel}>Cancel</button>
    {/if}
    <span class="phase">{phase}</span>
    {#if pending}<span class="phase">Resolve the pending revision before another capture.</span>{/if}
  </div>

  {#if error}
    <p class="error" role="alert">{error}</p>
  {/if}

  {#if pending}
    <div class="result review">
      <strong>Refinement ready for review</strong>
      {#if diff}
        <span>{diff.added.length} added · {diff.changed.length} changed · {diff.removed.length} removed</span>
        {#if diff.added.length + diff.changed.length + diff.removed.length > 0}
          <ul class="diff-list">
            {#each [...diff.added, ...diff.changed, ...diff.removed] as change}
              <li><strong>{change.kind}</strong> {change.category} <span class="muted">({change.collection})</span></li>
            {/each}
          </ul>
        {:else}
          <span class="muted">No semantic changes detected within the alignment tolerance.</span>
        {/if}
      {:else}
        <span>No semantic baseline yet; accepting creates the first native revision.</span>
      {/if}
      <div class="review-actions">
        <button onclick={acceptPending}>Accept refinement</button>
        <button class="quiet" onclick={rejectPending}>Reject</button>
      </div>
    </div>
  {:else if draft}
    <div class="result">
      <strong>Draft ready</strong>
      <span>{draft.room.surfaces.length} surfaces · {draft.room.openings.length} openings · {draft.room.objects.length} objects</span>
      <span class="muted">{draft.assets.length} native asset{draft.assets.length === 1 ? "" : "s"} preserved · {draft.provenance.capture_mode === "refine_existing" ? "refinement revision" : "new room revision"}</span>
    </div>
  {:else if reference}
    <div class="result">
      <strong>Imported scan ready for review</strong>
      <span>{reference.observation.export_observation.room_groups} room groups · {reference.observation.export_observation.mesh_assets} mesh assets</span>
      <span class="muted">Raw USDZ preserved · awaiting semantic native revision</span>
      <button class="secondary" onclick={() => previewImportedRoomPlanReference()}>Preview source USDZ (read-only)</button>
    </div>
  {/if}
</section>

<style>
  .capture-panel {
    border: 1px solid var(--line, #d7dedb);
    border-radius: 18px;
    padding: 1.25rem;
    background: var(--surface, #fbfcfb);
  }
  .capture-heading, .actions, .result { display: flex; align-items: center; gap: .75rem; }
  .capture-heading { justify-content: space-between; }
  .eyebrow { margin: 0 0 .2rem; color: var(--muted, #68736f); font: .72rem/1.2 ui-monospace, monospace; text-transform: uppercase; letter-spacing: .08em; }
  h2 { margin: 0; font-size: 1.35rem; }
  .availability { color: var(--muted, #68736f); font: .78rem ui-monospace, monospace; }
  .availability.online { color: #397565; }
  .explain { max-width: 50rem; color: var(--muted, #68736f); line-height: 1.5; }
  .explain p { margin: 1em 0; }
  .explain summary { display: none; }
  @media (width < 38rem) {
    /* Compact on a phone: title and status, the fold, then the action. */
    .capture-panel { padding: var(--space-4); border-radius: var(--radius-lg); }
    .capture-heading { align-items: flex-start; }
    h2 { font-size: var(--text-md); }
    .explain { margin: 0; font-size: var(--text-sm); }
    .explain summary { display: list-item; cursor: pointer; font-size: var(--text-xs); min-height: 2.75rem; line-height: 2.75rem; }
    .explain p { margin: 0 0 var(--space-2); }
    .actions { flex-wrap: wrap; gap: var(--space-3); }
  }
  button { border: 0; border-radius: 999px; padding: .65rem 1rem; background: #397565; color: white; cursor: pointer; }
  button:disabled { cursor: not-allowed; opacity: .45; }
  button.secondary { background: #5f746c; }
  button.quiet { background: transparent; color: var(--muted, #68736f); }
  .phase, .muted { color: var(--muted, #68736f); font: .8rem ui-monospace, monospace; }
  .result { margin-top: 1rem; flex-wrap: wrap; }
  .review-actions { display: flex; gap: .5rem; width: 100%; }
  .diff-list { width: 100%; margin: 0; padding-left: 1.2rem; color: #4d5e58; font-size: .88rem; }
  .diff-list strong { text-transform: capitalize; }
  .error { color: #a33f3f; }
</style>
