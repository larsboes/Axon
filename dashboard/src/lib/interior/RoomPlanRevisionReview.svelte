<script lang="ts">
  import { interior } from "$lib/api";
  import { compareRoomPlanDrafts, type RoomPlanDraft } from "$lib/roomplan";

  let { revisions = [] }: { revisions?: RoomPlanDraft[] } = $props();
  let reviewState = $state<"idle" | "accept" | "reject">("idle");
  let reviewError = $state<string | null>(null);

  async function review(decision: "accept" | "reject") {
    if (!latest) return;
    reviewError = null;
    try {
      await interior.reviewRoomplanRevision(latest.draft_id, decision);
      reviewState = decision;
    } catch (cause) {
      reviewError = cause instanceof Error ? cause.message : String(cause);
    }
  }
  const latest = $derived(revisions.at(-1) ?? null);
  const parent = $derived(revisions.length > 1 ? revisions.at(-2) ?? null : null);
  const diff = $derived(latest && parent ? compareRoomPlanDrafts(parent, latest) : null);
</script>

{#if revisions.length > 0}
  <section class="revision-review" aria-labelledby="revision-review-heading">
    <div class="heading">
      <div>
        <p class="eyebrow">Semantic revision review</p>
        <h3 id="revision-review-heading">What changed since the previous capture</h3>
      </div>
      <span class="mono">{revisions.length} revision{revisions.length === 1 ? "" : "s"}</span>
    </div>
    {#if diff}
      <p class="summary">
        {diff.added.length} added · {diff.changed.length} changed · {diff.removed.length} removed
      </p>
      {#if diff.added.length + diff.changed.length + diff.removed.length > 0}
        <ul>
          {#each [...diff.added, ...diff.changed, ...diff.removed] as change}
            <li><strong>{change.kind}</strong> {change.category} <span class="muted">({change.collection})</span></li>
          {/each}
        </ul>
      {:else}
        <p class="muted">No semantic changes detected within the alignment tolerance.</p>
      {/if}
    {:else}
      <p class="muted">One semantic revision is available. A second revision is required for comparison.</p>
    {/if}
    <div class="actions">
      <button onclick={() => review("accept")} disabled={reviewState !== "idle"}>Approve revision</button>
      <button class="quiet" onclick={() => review("reject")} disabled={reviewState !== "idle"}>Reject</button>
      {#if reviewState !== "idle"}<span class="muted">Decision recorded: {reviewState}</span>{/if}
      {#if reviewError}<span class="error">{reviewError}</span>{/if}
    </div>
    <p class="note">Approval records the review decision only. Reconciliation into room.toml remains a separate explicit action; raw USDZ assets are never overwritten.</p>
  </section>
{/if}

<style>
  .revision-review { margin-top: 1rem; padding-top: 1rem; border-top: 1px solid var(--line, #d7dedb); }
  .heading { display: flex; justify-content: space-between; align-items: start; gap: 1rem; }
  .eyebrow { margin: 0 0 .2rem; color: var(--muted, #68736f); font: .72rem/1.2 ui-monospace, monospace; text-transform: uppercase; letter-spacing: .08em; }
  h3 { margin: 0; font-size: 1rem; }
  .summary { margin-bottom: .4rem; font-weight: 600; }
  ul { margin: .4rem 0; padding-left: 1.2rem; color: #4d5e58; font-size: .88rem; }
  .muted, .note, .mono { color: var(--muted, #68736f); font: .8rem ui-monospace, monospace; }
  .note { line-height: 1.5; }
  .actions { display: flex; align-items: center; gap: .5rem; flex-wrap: wrap; margin-top: .75rem; }
  button { border: 0; border-radius: 999px; padding: .55rem .85rem; background: #397565; color: white; cursor: pointer; }
  button:disabled { opacity: .5; cursor: not-allowed; }
  button.quiet { background: transparent; color: var(--muted, #68736f); }
  .error { color: #a33f3f; font-size: .8rem; }
</style>
