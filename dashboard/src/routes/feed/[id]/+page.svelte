<script lang="ts">
  import { onMount } from "svelte";
  import { page } from "$app/state";
  import EvaluationBreakdown from "$lib/feed/EvaluationBreakdown.svelte";
  import MarkdownDocument from "$lib/feed/MarkdownDocument.svelte";
  import Icon from "$lib/Icon.svelte";
  import { axonStatus, comms, type FeedEntryDetail, type FeedStatus } from "$lib/api";

  const KIND_LABEL: Record<string, string> = {
    youtube: "YouTube",
    instagram: "Instagram",
    podcast: "Podcast",
    article: "Article",
    mail: "Mail",
    github: "GitHub",
    arxiv: "arXiv",
    reddit: "Reddit",
  };

  let entry = $state<FeedEntryDetail | null>(null);
  let loading = $state(true);
  let error = $state<string | null>(null);
  let busy = $state(false);

  const title = $derived(entry?.title ?? entry?.url ?? "Feed entry");
  const bodyLabel = $derived(
    entry?.kind === "github"
      ? "README"
      : entry?.kind === "arxiv"
        ? "Abstract"
        : entry?.kind === "youtube" || entry?.kind === "podcast"
          ? "Transcript"
          : "Article content",
  );
  const readMinutes = $derived(
    Math.max(
      1,
      Math.ceil(
        `${entry?.summary ?? ""} ${entry?.transcript ?? ""}`.trim().split(/\s+/).filter(Boolean)
          .length / 220,
      ),
    ),
  );
  const transcriptCollapsible = $derived(
    Boolean(entry?.summary) &&
      (entry?.kind === "youtube" ||
        entry?.kind === "podcast" ||
        entry?.kind === "instagram"),
  );

  onMount(() => {
    void (async () => {
      try {
        await axonStatus.start("comms").catch(() => undefined);
        entry = await comms.entry(page.params.id ?? "");
      } catch (cause) {
        error = cause instanceof Error ? cause.message : String(cause);
      } finally {
        loading = false;
      }
    })();
  });

  async function setStatus(status: FeedStatus): Promise<void> {
    if (!entry || busy) return;
    busy = true;
    try {
      await comms.setStatus(entry.id, status);
      entry.status = status;
    } catch (cause) {
      error = cause instanceof Error ? cause.message : String(cause);
    } finally {
      busy = false;
    }
  }
</script>

<svelte:head>
  <title>{title} · Axon Feed</title>
</svelte:head>

<a class="back" href="/feed">← Back to Feed</a>

{#if loading}
  <p class="state" aria-live="polite"><Icon name="loader" size={14} /> Loading entry…</p>
{:else if error && !entry}
  <p class="state error" aria-live="polite">{error}</p>
{:else if entry}
  <article>
    <header class="article-head">
      <div class="overline">
        <span class="tag mono">{KIND_LABEL[entry.kind] ?? entry.kind}</span>
        <span>{new Date(entry.created_at).toLocaleDateString("en-GB")}</span>
        <span>{readMinutes} min read</span>
      </div>
      <h1>{title}</h1>
      {#if entry.author}<p class="byline">{entry.author}</p>{/if}
      <div class="actions">
        <a class="btn btn-primary" href={entry.url} target="_blank" rel="noreferrer">
          Open original <Icon name="external" size={13} />
        </a>
        <button
          class="btn"
          class:kept={entry.status === "keeper"}
          disabled={busy}
          onclick={() => setStatus(entry?.status === "keeper" ? "new" : "keeper")}
        >
          <Icon name="check" size={13} />
          {entry.status === "keeper" ? "Remove from saved" : "Keep"}
        </button>
        <button class="btn" disabled={busy} onclick={() => setStatus("dismissed")}>
          <Icon name="close" size={13} /> Dismiss
        </button>
      </div>
      {#if error}<p class="inline-error" aria-live="polite">{error}</p>{/if}
    </header>

    <div class="reader-grid">
      <div class="reader">
        {#if entry.summary}
          <section class="note">
            <p class="section-label">Summary</p>
            <MarkdownDocument content={entry.summary} compact />
          </section>
        {/if}

        {#if entry.transcript && transcriptCollapsible}
          <details class="transcript-disclosure">
            <summary>
              <span>
                <span class="section-label">Source</span>
                <strong>{bodyLabel}</strong>
              </span>
              <span class="disclosure-meta">{readMinutes} min · open</span>
            </summary>
            <div class="transcript-body">
              <a class="source-link" href={entry.url} target="_blank" rel="noreferrer">
                Read original <Icon name="external" size={12} />
              </a>
              <MarkdownDocument content={entry.transcript} />
            </div>
          </details>
        {:else if entry.transcript}
          <section class="source-document">
            <div class="document-head">
              <div>
                <p class="section-label">Source</p>
                <h2>{bodyLabel}</h2>
              </div>
              <a href={entry.url} target="_blank" rel="noreferrer">
                Read original <Icon name="external" size={12} />
              </a>
            </div>
            <MarkdownDocument content={entry.transcript} />
          </section>
        {:else}
          <section class="source-document empty">
            No readable source text is available for this entry. The original remains linked.
          </section>
        {/if}
      </div>

      <aside class="context">
        <section>
          <p class="section-label">Entry</p>
          <dl>
            <div><dt>Type</dt><dd>{KIND_LABEL[entry.kind] ?? entry.kind}</dd></div>
            <div>
              <dt>Captured</dt>
              <dd>{new Date(entry.created_at).toLocaleDateString("en-GB")}</dd>
            </div>
            <div><dt>Reading time</dt><dd>{readMinutes} min</dd></div>
            <div>
              <dt>Processing</dt>
              <dd>{entry.summary ? "Summary + source" : "Formatted source"}</dd>
            </div>
          </dl>
        </section>

        {#if entry.evaluation}
          <section aria-labelledby="evaluation-title">
            <p class="section-label" id="evaluation-title">Why is this here?</p>
            <EvaluationBreakdown evaluation={entry.evaluation} />
          </section>
        {/if}

        {#if entry.relevance.length > 0}
          <section aria-labelledby="relevance-title">
            <div class="aside-title">
              <p class="section-label" id="relevance-title">Matches</p>
              <span>{entry.relevance[0].mode === "semantic" ? "semantic" : "lexical"}</span>
            </div>
            {#each entry.relevance as match (match.profile_key)}
              <div class="match">
                <div class="match-head">
                  <strong>{match.profile_label}</strong>
                  <span class="mono">{match.score.toFixed(2)}</span>
                </div>
                <details>
                  <summary>Classification</summary>
                  <p>{match.rationale}</p>
                </details>
              </div>
            {/each}
          </section>
        {/if}

        {#if entry.origins.length > 0}
          <section>
            <p class="section-label">Found via</p>
            {#each entry.origins as origin (`${origin.source_id}:${origin.source_ref}`)}
              <p class="origin">
                {origin.label ?? origin.source_id}
                <span class="mono">{origin.source_ref}</span>
              </p>
            {/each}
          </section>
        {/if}
      </aside>
    </div>
  </article>
{/if}

<style>
  .back {
    display: inline-flex;
    align-items: center;
    gap: 0.35rem;
    margin: 0 0 2rem;
    color: var(--text-secondary);
    font-size: 0.75rem;
    text-decoration: none;
  }

  article {
    width: 100%;
    max-width: 88rem;
    margin: 0 auto;
  }

  .article-head {
    max-width: 76rem;
    margin: 0 auto 3.25rem;
  }

  .overline,
  .actions {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0.55rem;
  }

  .overline {
    color: var(--text-tertiary);
    font-size: 0.6875rem;
  }

  h1 {
    max-width: 24ch;
    margin: 0.9rem 0 0;
    font-size: clamp(2.1rem, 4.2vw, 3.8rem);
    font-weight: 560;
    line-height: 1.04;
    letter-spacing: -0.04em;
  }

  .byline {
    margin: 0.65rem 0 0;
    color: var(--text-secondary);
  }

  .actions {
    margin-top: 1.25rem;
  }

  .kept {
    color: var(--success);
  }

  .reader-grid {
    display: grid;
    grid-template-columns: minmax(0, 52rem) minmax(17rem, 21rem);
    align-items: start;
    justify-content: center;
    gap: clamp(2.5rem, 5vw, 5rem);
  }

  .section-label {
    margin: 0 0 0.8rem;
    color: var(--primary);
    font-family: var(--font-mono);
    font-size: 0.625rem;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.07em;
  }

  .reader {
    min-width: 0;
  }

  .note {
    margin-bottom: 3rem;
    padding: 1.25rem 0 1.25rem 1.4rem;
    border-left: 2px solid var(--primary);
  }

  .source-document {
    min-width: 0;
  }

  .transcript-disclosure {
    border-top: 1px solid var(--card-border);
    border-bottom: 1px solid var(--card-border);
  }

  .transcript-disclosure > summary {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 1rem;
    padding: 1.1rem 0;
    cursor: pointer;
    list-style: none;
  }

  .transcript-disclosure > summary::-webkit-details-marker {
    display: none;
  }

  .transcript-disclosure > summary::after {
    content: "+";
    flex: 0 0 auto;
    color: var(--primary);
    font-family: var(--font-mono);
    font-size: 1rem;
  }

  .transcript-disclosure[open] > summary::after {
    content: "−";
  }

  .transcript-disclosure .section-label {
    display: block;
    margin-bottom: 0.15rem;
  }

  .transcript-disclosure strong {
    font-size: 1rem;
    font-weight: 560;
  }

  .disclosure-meta {
    margin-left: auto;
    color: var(--text-tertiary);
    font-size: 0.6875rem;
  }

  .transcript-body {
    position: relative;
    padding: 1.25rem 0 2rem;
    border-top: 1px solid var(--card-border);
  }

  .source-link {
    display: inline-flex;
    align-items: center;
    gap: 0.3rem;
    margin-bottom: 1.25rem;
    color: var(--text-tertiary);
    font-size: 0.6875rem;
  }

  .document-head {
    display: flex;
    align-items: end;
    justify-content: space-between;
    gap: 1rem;
    margin-bottom: 1.4rem;
    padding-bottom: 0.75rem;
    border-bottom: 1px solid var(--card-border);
  }

  .document-head .section-label {
    margin-bottom: 0.15rem;
  }

  .document-head h2 {
    margin: 0;
    font-size: 1.25rem;
  }

  .document-head a {
    display: inline-flex;
    align-items: center;
    gap: 0.3rem;
    flex-shrink: 0;
    color: var(--text-tertiary);
    font-size: 0.6875rem;
  }

  .empty {
    padding: 1rem 0;
    color: var(--text-tertiary);
    font-size: 0.875rem;
  }

  .context {
    position: sticky;
    top: 8rem;
  }

  .context section {
    padding: 1.1rem 0;
    border-top: 1px solid var(--card-border);
  }

  .context section:first-child {
    padding-top: 0;
    border-top: 0;
  }

  dl {
    margin: 0;
  }

  dl div {
    display: flex;
    justify-content: space-between;
    gap: 1rem;
    padding: 0.25rem 0;
    font-size: 0.75rem;
  }

  dt {
    color: var(--text-tertiary);
  }

  dd {
    margin: 0;
    color: var(--text-secondary);
    text-align: right;
  }

  .aside-title,
  .match-head {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 0.75rem;
  }

  .aside-title > span {
    color: var(--text-tertiary);
    font-size: 0.625rem;
  }

  .match {
    padding: 0.7rem 0;
    border-top: 1px solid var(--card-border);
  }

  .match-head {
    font-size: 0.8125rem;
  }

  .match-head span {
    color: var(--primary);
  }

  .match details {
    margin-top: 0.25rem;
  }

  .match summary {
    cursor: pointer;
    color: var(--text-tertiary);
    font-size: 0.625rem;
  }

  .match p,
  .origin {
    margin: 0.35rem 0 0;
    color: var(--text-secondary);
    font-size: 0.6875rem;
    line-height: 1.5;
  }

  .origin span {
    display: block;
    margin-top: 0.15rem;
    color: var(--text-tertiary);
  }

  .state {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    color: var(--text-secondary);
  }

  .error,
  .inline-error {
    color: var(--warning);
  }

  .inline-error {
    font-size: 0.75rem;
  }

  @media (max-width: 62rem) {
    .article-head {
      margin-bottom: 2.5rem;
    }

    .reader-grid {
      grid-template-columns: minmax(0, 48rem);
    }

    .context {
      position: static;
      display: grid;
      grid-template-columns: repeat(auto-fit, minmax(14rem, 1fr));
      gap: 1.5rem;
      margin-top: 3rem;
      padding-top: 1rem;
      border-top: 1px solid var(--card-border);
    }

    .context section,
    .context section:first-child {
      padding: 0;
      border: 0;
    }
  }

  @media (max-width: 40rem) {
    .article-head {
      margin-bottom: 2rem;
    }

    h1 {
      font-size: 2.15rem;
    }

    .document-head {
      align-items: flex-start;
      flex-direction: column;
    }

    .transcript-disclosure > summary {
      align-items: flex-start;
    }

    .disclosure-meta {
      display: none;
    }

    .note {
      padding-left: 1rem;
    }
  }
</style>
