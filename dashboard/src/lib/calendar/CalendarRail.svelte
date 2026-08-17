<script lang="ts">
  import Rail from "$lib/rail/Rail.svelte";
  import RailGroup from "$lib/rail/RailGroup.svelte";
  import RailSection from "$lib/rail/RailSection.svelte";
  import GoogleDraftInbox from "./GoogleDraftInbox.svelte";
  import GoogleExportReview from "./GoogleExportReview.svelte";
  import GoogleImportReview from "./GoogleImportReview.svelte";
  import CalendarProposalInbox from "./CalendarProposalInbox.svelte";
  import TripDraftInbox from "./TripDraftInbox.svelte";
  import ContextPanel from "./ContextPanel.svelte";
  import RhythmForm from "./RhythmForm.svelte";
  import { kindConfig } from "./types";
  import type {
    CalendarContext,
    CalendarNewContext,
    CalendarNewRhythm,
    CalendarRhythm,
    CalendarUpdateContext,
  } from "$lib/api";

  let {
    refreshes,
    onReviewChanged,
    contexts,
    rhythms,
    rangeLabel,
    defaultFrom,
    defaultUntil,
    contextOpenId = null,
    onCreateContext,
    onUpdateContext,
    onDeleteContext,
    onSaveRhythm,
  }: {
    /** Bumped by the page whenever an edit elsewhere could change what is pending. */
    refreshes: { google: number; proposals: number; trips: number };
    onReviewChanged: () => Promise<void>;
    contexts: CalendarContext[];
    rhythms: CalendarRhythm[];
    rangeLabel: string;
    defaultFrom: string;
    defaultUntil: string;
    contextOpenId?: string | null;
    onCreateContext: (context: CalendarNewContext) => Promise<void>;
    onUpdateContext: (id: string, context: CalendarUpdateContext) => Promise<void>;
    onDeleteContext: (id: string) => Promise<void>;
    onSaveRhythm: (rhythm: CalendarNewRhythm) => Promise<void>;
  } = $props();

  // Counts are reported up by each section rather than fetched here, so a collapsed
  // row can still show what is waiting without the rail duplicating three queries.
  let googleCount = $state(0);
  let proposalCount = $state(0);
  let tripCount = $state(0);
  let reviewCount = $derived(googleCount + proposalCount + tripCount);

  let showGoogleImport = $state(false);
  let showGoogleExport = $state(false);
  let showRhythmForm = $state(false);

  // A deep-linked context has to be reachable, and it opens its own editor from a
  // prop — so the section holding it must not be collapsed when the link lands.
  let contextOpen = $state(false);

  $effect(() => {
    if (contextOpenId) contextOpen = true;
  });

  async function onRhythmSaved(rhythm: CalendarNewRhythm) {
    await onSaveRhythm(rhythm);
    showRhythmForm = false;
  }
</script>

<Rail label="Calendar review and planning">
  <RailGroup title="Review" count={reviewCount}>
    <RailSection label="Google" count={googleCount}>
      <GoogleDraftInbox
        refresh={refreshes.google}
        onChanged={onReviewChanged}
        onCount={(count) => (googleCount = count)}
      />
      <div class="section-actions">
        <button class="btn btn-outline" onclick={() => (showGoogleImport = true)}>Import</button>
        <button class="btn btn-outline" onclick={() => (showGoogleExport = true)}>Sync</button>
      </div>
    </RailSection>

    <RailSection label="Proposals" count={proposalCount}>
      <CalendarProposalInbox
        refresh={refreshes.proposals}
        onChanged={onReviewChanged}
        onCount={(count) => (proposalCount = count)}
      />
    </RailSection>

    <RailSection label="Trips" count={tripCount}>
      <TripDraftInbox refresh={refreshes.trips} onCount={(count) => (tripCount = count)} />
    </RailSection>
  </RailGroup>

  <RailGroup title="Plan">
    <RailSection label="Context" count={contexts.length} bind:open={contextOpen}>
      <ContextPanel
        {contexts}
        {rangeLabel}
        {defaultFrom}
        {defaultUntil}
        openId={contextOpenId}
        onCreate={onCreateContext}
        onUpdate={onUpdateContext}
        onDelete={onDeleteContext}
      />
    </RailSection>

    <RailSection label="Rhythms" count={rhythms.length}>
      {#if rhythms.length === 0}
        <p class="empty">No recurring rhythms yet.</p>
      {:else}
        <ul class="rhythm-list">
          {#each rhythms as rhythm (rhythm.id)}
            <li>
              <span class="dot" style={`background: ${kindConfig(rhythm.kind).color}`}></span>
              <span class="rhythm-title">{rhythm.title}</span>
              <span class="rhythm-when">
                {rhythm.byweekday.map((day: string) => day.slice(0, 2)).join(" ")}
                {#if rhythm.start_time}· {rhythm.start_time}{/if}
              </span>
            </li>
          {/each}
        </ul>
      {/if}
      <button class="btn btn-outline add" onclick={() => (showRhythmForm = true)}>+ Rhythm</button>
    </RailSection>
  </RailGroup>
</Rail>

{#if showGoogleImport}
  <GoogleImportReview onImported={onReviewChanged} onClose={() => (showGoogleImport = false)} />
{/if}

{#if showGoogleExport}
  <GoogleExportReview onSynced={onReviewChanged} onClose={() => (showGoogleExport = false)} />
{/if}

{#if showRhythmForm}
  <RhythmForm onSave={onRhythmSaved} onClose={() => (showRhythmForm = false)} />
{/if}

<style>
  /* Rail chrome — the aside, its groups and their headings — lives in `$lib/rail`,
   * because /travel wears the same one. What stays here is what only this rail has. */
  .section-actions {
    display: flex;
    gap: 0.3rem;
    margin-top: 0.5rem;
  }

  .section-actions .btn,
  .add {
    flex: 1;
    padding: 0.3rem 0.5rem;
    font-size: 0.72rem;
  }

  .add {
    width: 100%;
    margin-top: 0.5rem;
  }

  .empty {
    margin: 0;
    color: var(--text-tertiary);
    font-size: 0.78rem;
  }

  .rhythm-list {
    display: flex;
    flex-direction: column;
    gap: 0.3rem;
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .rhythm-list li {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    min-width: 0;
    font-size: 0.75rem;
  }

  .dot {
    width: 0.4rem;
    height: 0.4rem;
    border-radius: 50%;
    flex-shrink: 0;
  }

  .rhythm-title {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .rhythm-when {
    color: var(--text-tertiary);
    font-size: 0.68rem;
    white-space: nowrap;
  }
</style>
