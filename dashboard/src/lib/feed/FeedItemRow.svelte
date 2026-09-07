<script lang="ts" module>
  /** The kind labels the feed shows, in one place rather than one per surface. */
  export const FEED_KIND_LABEL: Record<string, string> = {
    youtube: "YouTube",
    instagram: "Instagram",
    podcast: "Podcast",
    article: "Article",
    mail: "Mail",
    github: "GitHub",
    arxiv: "arXiv",
    reddit: "Reddit",
  };
</script>

<script lang="ts">
  import type { Snippet } from "svelte";
  import type { FeedEntry } from "../api";
  import { link } from "../nav";
  import Icon from "../Icon.svelte";
  import ListRow from "../ListRow.svelte";
  import RowMeta from "../RowMeta.svelte";
  import FactorBars, { type Factor } from "../FactorBars.svelte";

  /**
   * One feed item, everywhere a feed item is triaged.
   *
   * Home's ladder and /feed's inbox had grown two rows for the same object, with
   * different actions in different places, and the keyboard triage pass needs one shape to
   * bind to. Callback props rather than `createEventDispatcher`, which Svelte 5 keeps only
   * for compatibility.
   *
   * `dataClass` is not passed and is not guessed. `FeedEntry` carries no class field while
   * the `ContentItem` detail shape does, so the chip cannot render honestly until comms
   * publishes a class on the list contract.
   *
   * Two of the props below exist because /feed's inbox moved onto this row on 2026-09-07
   * and had them: a decided row greys in place instead of leaving, and the inbox shows the
   * full evaluation breakdown where Home shows factor bars. Both follow `meta`'s rule —
   * the caller replaces a default only where it knows more.
   */
  let {
    entry,
    id,
    current = false,
    busy = false,
    dense = false,
    tone = "offer",
    decided = null,
    undoHint,
    meta,
    detail,
    onkeep,
    ondismiss,
    onopen,
    onexternal,
    onundo,
  }: {
    entry: FeedEntry;
    /** Stable DOM id for the keyboard cursor. */
    id: string;
    current?: boolean;
    /** This row's own action is in flight. */
    busy?: boolean;
    /** Inbox density: no factor bars, no preview line. */
    dense?: boolean;
    tone?: "alarm" | "now" | "owed" | "offer" | "none";
    /** The row has been decided and is greyed in place. Its actions become one Undo. */
    decided?: "keeper" | "dismissed" | null;
    /** Appended to the verdict, e.g. a keyboard hint. Only where one exists. */
    undoHint?: string;
    /** Replaces the default provenance line — used where the caller knows more. */
    meta?: Snippet;
    /** Replaces the default factor bars — same rule as `meta`. */
    detail?: Snippet;
    onkeep?: () => void;
    ondismiss?: () => void;
    /** Called when the row's title is activated by something other than a click. */
    onopen?: () => void;
    /** Called when the original-source link is activated. */
    onexternal?: () => void;
    onundo?: () => void;
  } = $props();

  const href = $derived(link(`/feed/${encodeURIComponent(entry.id)}`));
  const kindLabel = $derived(FEED_KIND_LABEL[entry.kind] ?? entry.kind);

  const factors = $derived<Factor[]>(
    (entry.evaluation?.factors ?? []).map((factor) => ({
      key: factor.key,
      label: factor.label,
      score: factor.score,
      weight: factor.weight,
      // A glyph, not a second hue: the travel factor used to be painted with the reserved
      // success colour, and its context link only renders in the non-compact form.
      mark: factor.context?.kind === "trip" ? "map-pin" : undefined,
      rationale: factor.rationale,
      context: factor.context
        ? { label: factor.context.label, terms: factor.context.matched_terms }
        : null,
    })),
  );

  const preview = $derived(entry.summary ?? entry.digest_preview);
  const whyHere = $derived(
    entry.evaluation?.explanation ?? entry.relevance?.rationale ?? "",
  );
</script>

<ListRow {id} {current} {tone} dimmed={decided !== null}>
  <!-- Greyed in place, not removed: the cursor stays where the operator left it and the
       decision is one keystroke away from being retracted. -->
  {#snippet mark()}<Icon name="feed" size={15} />{/snippet}

  <span class="row-kind">
    {kindLabel}{#if entry.author} · {entry.author}{/if}{#if entry.relevance} · matches {entry.relevance.profile_label}{/if}
  </span>
  <a class="row-title" {href} onclick={() => onopen?.()}>{entry.title ?? entry.url}</a>

  {#if !dense && preview}
    <p class="row-text">{preview}</p>
    {#if !entry.summary}
      <!-- No summary of its own: past the on-device window, so the enrichment drain left
           it and the digest drain took it through the cloud instead. Labelled so the two
           are not confused. -->
      <span class="from-digest">from the digest</span>
    {/if}
  {/if}

  {#if detail}
    <div class="detail">{@render detail()}</div>
  {:else if !dense && factors.length > 0}
    <div class="factors"><FactorBars {factors} compact weighted /></div>
  {/if}

  {#snippet meta()}
    {#if meta}{@render meta()}{:else if whyHere}<RowMeta {whyHere} candidateStatus="proposed" />{/if}
  {/snippet}

  {#snippet actions()}
    {#if decided}
      <span class="verdict mono">
        {decided === "keeper" ? "kept" : "dismissed"}{undoHint ? ` · ${undoHint}` : ""}
      </span>
      {#if onundo}<button class="btn" type="button" onclick={() => onundo()}>Undo</button>{/if}
    {:else}
    <a class="btn" {href}>Read</a>
    {#if onkeep}
      <button
        class="btn btn-soft"
        type="button"
        disabled={busy}
        onclick={() => onkeep()}
      >
        {#if busy}<Icon name="loader" size={13} />{:else if entry.status === "keeper"}Kept{:else}Keep{/if}
      </button>
    {/if}
    <a
      class="btn"
      href={entry.url}
      target="_blank"
      rel="noreferrer"
      aria-label="Original"
      title="Original"
      onclick={() => onexternal?.()}
    >
      <Icon name="external" size={13} />
    </a>
    {#if ondismiss}
      <button
        class="btn"
        type="button"
        disabled={busy}
        aria-label="Dismiss feed entry"
        title="Dismiss"
        onclick={() => ondismiss()}
      >
        <Icon name="close" size={13} />
      </button>
    {/if}
    {/if}
  {/snippet}
</ListRow>

<style>
  .detail,
  .factors {
    margin-top: var(--space-2);
    max-width: 34rem;
  }

  .from-digest {
    color: var(--text-tertiary);
    font-size: var(--text-2xs);
  }

  .verdict {
    align-self: center;
    color: var(--text-tertiary);
    font-size: var(--text-2xs);
  }
</style>
