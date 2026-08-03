<script lang="ts">
  import { page } from "$app/state";
  import DiscoverView from "$lib/feed/DiscoverView.svelte";
  import EvaluationBreakdown from "$lib/feed/EvaluationBreakdown.svelte";
  import FeedNav from "$lib/feed/FeedNav.svelte";
  import ModelStatus from "$lib/feed/ModelStatus.svelte";
  import Icon from "$lib/Icon.svelte";
  import PageHeader from "$lib/PageHeader.svelte";
  import {
    axonStatus,
    comms,
    type CommsEvaluationStatus,
    type FeedEntry,
    type FeedEntryDetail,
    type FeedRun,
    type FeedSource,
    type FeedStatus,
    type FeedStream,
    type TriageItem,
    type VaultLinkCandidate,
  } from "$lib/api";

  type StreamFilter = "all" | FeedStream;
  type Order = "recent" | "relevance";
  type FeedView = "inbox" | "discover";

  const STREAMS: { value: StreamFilter; label: string }[] = [
    { value: "all", label: "All" },
    { value: "news", label: "News" },
    { value: "media", label: "Media" },
  ];
  const RANGES = [7, 30, 90];
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

  let stream = $state<StreamFilter>("all");
  let days = $state(7);
  let order = $state<Order>("recent");

  let pasted = $state("");
  let ingesting = $state(false);
  let ingestError = $state<string | null>(null);
  let ingested = $state<string | null>(null);

  let entries = $state<FeedEntry[]>([]);
  let runs = $state<FeedRun[]>([]);
  let expandedRuns = $state<Set<string>>(new Set());
  let triage = $state<TriageItem[]>([]);
  let loading = $state(true);
  let offline = $state(false);
  let busy = $state<string | null>(null);
  let ready = $state(false);
  let relevanceBusy = $state(false);
  let relevanceNotice = $state<string | null>(null);
  let modelStatus = $state<CommsEvaluationStatus | null>(null);
  let vaultOpen = $state(false);
  let vaultBusy = $state(false);
  let vaultLinks = $state<VaultLinkCandidate[]>([]);
  let vaultError = $state<string | null>(null);
  let sourcesOpen = $state(false);
  let sourcesBusy = $state(false);
  let feedSources = $state<FeedSource[]>([]);
  let sourceNotice = $state<string | null>(null);
  const view = $derived<FeedView>(
    page.url.searchParams.get("view") === "discover" ? "discover" : "inbox",
  );

  // Grouped by the day the item belongs to, newest first. The server already returns a
  // `day` key per entry, so this stays presentation and never re-derives dates.
  const grouped = $derived.by(() => {
    if (order === "relevance") {
      return [
        [
          "relevance",
          [...entries].sort(
            (a, b) =>
              (b.evaluation?.overall_score ?? b.relevance?.score ?? -1) -
              (a.evaluation?.overall_score ?? a.relevance?.score ?? -1),
          ),
        ],
      ] as [string, FeedEntry[]][];
    }
    const byDay = new Map<string, FeedEntry[]>();
    for (const e of entries) {
      const list = byDay.get(e.day);
      if (list) list.push(e);
      else byDay.set(e.day, [e]);
    }
    return [...byDay.entries()].sort((a, b) => b[0].localeCompare(a[0]));
  });

  // A collector run that contributed a dozen papers should read as one thing to
  // triage, not twelve. The server derives which items arrived together; this
  // decides only how they are shown.
  const runOf = $derived(new Map(runs.map((r) => [r.feed_id, r])));

  type Row =
    | { kind: "single"; id: string; entry: FeedEntry }
    | { kind: "run"; id: string; label: string; entries: FeedEntry[] };

  // A run of one is just an item: collapsing it would hide a row behind a click
  // and tell the reader nothing they could not already see.
  function rowsFor(items: FeedEntry[]): Row[] {
    const groups = new Map<string, FeedEntry[]>();
    for (const e of items) {
      const key = runOf.get(e.id)?.run_key;
      if (!key) continue;
      const list = groups.get(key);
      if (list) list.push(e);
      else groups.set(key, [e]);
    }

    const rows: Row[] = [];
    const grouped = new Set<string>();
    for (const [key, group] of groups) {
      if (group.length < 2) continue;
      for (const e of group) grouped.add(e.id);
      const run = runOf.get(group[0].id);
      rows.push({
        kind: "run",
        id: key,
        label: run?.label ?? run?.source_id ?? "Collection run",
        entries: group,
      });
    }
    for (const e of items) {
      if (!grouped.has(e.id)) rows.push({ kind: "single", id: e.id, entry: e });
    }
    return rows;
  }

  function toggleRun(key: string): void {
    const next = new Set(expandedRuns);
    if (!next.delete(key)) next.add(key);
    expandedRuns = next;
  }

  async function load(): Promise<void> {
    loading = true;
    try {
      const [feed, proposals, feedRuns] = await Promise.all([
        comms.feed({
          stream: stream === "all" ? undefined : stream,
          days,
        }),
        comms.triage().catch(() => [] as TriageItem[]),
        comms.runs(days).catch(() => [] as FeedRun[]),
      ]);
      entries = feed.filter((entry) => entry.status === "new");
      triage = proposals;
      runs = feedRuns;
      offline = false;
    } catch {
      offline = true;
    } finally {
      loading = false;
    }
  }

  async function loadModelStatus(): Promise<void> {
    modelStatus = await comms.evaluationStatus().catch(() => null);
  }

  $effect(() => {
    if (view !== "inbox" || ready) return;
    void axonStatus
      .start("comms")
      .catch(() => undefined)
      .finally(() => {
        ready = true;
        void loadModelStatus();
      });
  });

  // Re-fetch when a filter changes. Reading the three pieces of state is what registers
  // the dependency; load() itself is deliberately untracked.
  $effect(() => {
    void stream;
    void days;
    if (view === "inbox" && ready) void load();
  });

  async function ingest(): Promise<void> {
    const url = pasted.trim();
    if (!url || ingesting) return;
    ingesting = true;
    ingestError = null;
    ingested = null;
    try {
      const entry = await comms.ingest(url);
      // The id is the hash of the canonical URL, so re-pasting a known link updates
      // the row it already has instead of adding a second one.
      const known = entries.some((e) => e.id === entry.id);
      const listEntry = toListEntry(entry);
      if (entry.status === "new") {
        entries = known
          ? entries.map((e) => (e.id === entry.id ? listEntry : e))
          : [listEntry, ...entries];
      }
      pasted = "";
      ingested = entry.id;
      offline = false;
    } catch (e) {
      ingestError = e instanceof Error ? e.message : String(e);
    } finally {
      ingesting = false;
    }
  }

  function toListEntry(entry: FeedEntryDetail): FeedEntry {
    return { ...entry, relevance: entry.relevance[0] ?? null };
  }

  async function refreshRelevance(): Promise<void> {
    if (relevanceBusy) return;
    relevanceBusy = true;
    relevanceNotice = null;
    try {
      const result = await comms.refreshRelevance(Math.max(days, 90));
      const method =
        result.mode === "reranked"
          ? "reranked"
          : result.mode === "semantic"
            ? "semantic"
          : result.mode === "lexical"
            ? "lexical (local embedding unavailable)"
            : "without profiles";
      relevanceNotice =
        result.evaluated === 0
          ? `${result.skipped_current} entries are already current — no reevaluation needed.`
          : `${result.evaluated} of ${result.considered} entries reevaluated, ${result.skipped_current} unchanged — ${method}.`;
      await Promise.all([load(), loadModelStatus()]);
    } catch (e) {
      relevanceNotice = e instanceof Error ? e.message : String(e);
    } finally {
      relevanceBusy = false;
    }
  }

  async function scanVault(): Promise<void> {
    vaultOpen = true;
    vaultBusy = true;
    vaultError = null;
    try {
      vaultLinks = await comms.scanVaultLinks();
    } catch (e) {
      vaultError = e instanceof Error ? e.message : String(e);
    } finally {
      vaultBusy = false;
    }
  }

  async function openSources(): Promise<void> {
    sourcesOpen = true;
    sourcesBusy = true;
    sourceNotice = null;
    try {
      const response = await comms.sources();
      feedSources = response.sources.filter((source) => source.enabled);
    } catch (cause) {
      sourceNotice = cause instanceof Error ? cause.message : String(cause);
    } finally {
      sourcesBusy = false;
    }
  }

  async function scanSources(sourceId?: string): Promise<void> {
    if (sourcesBusy) return;
    sourcesBusy = true;
    sourceNotice = null;
    try {
      const result = await comms.scanSources(sourceId);
      sourceNotice = `${result.fetched} found · ${result.new_count} new. Summaries and ranking continue in the background.`;
      const [sourceResponse] = await Promise.all([comms.sources(), load()]);
      feedSources = sourceResponse.sources.filter((source) => source.enabled);
    } catch (cause) {
      sourceNotice = cause instanceof Error ? cause.message : String(cause);
    } finally {
      sourcesBusy = false;
    }
  }

  function sourceLabel(source: FeedSource): string {
    if (source.adapter === "github-trending") return "GitHub Trending";
    if (source.adapter === "arxiv") return "New arXiv papers";
    return source.id;
  }

  function sourceRunLabel(value: string | null): string {
    if (!value) return "not scanned yet";
    const epoch = Number(value);
    const date = Number.isFinite(epoch) ? new Date(epoch * 1000) : new Date(value);
    if (Number.isNaN(date.getTime())) return value;
    return `last ${date.toLocaleString("en-GB", { day: "numeric", month: "short", hour: "2-digit", minute: "2-digit" })}`;
  }

  async function importVault(candidate: VaultLinkCandidate): Promise<void> {
    busy = candidate.id;
    vaultError = null;
    try {
      const entry = await comms.importVaultLink(candidate.source_id, candidate.url);
      const listEntry = toListEntry(entry);
      entries = entries.some((item) => item.id === entry.id)
        ? entries.map((item) => (item.id === entry.id ? listEntry : item))
        : [listEntry, ...entries];
      vaultLinks = vaultLinks.map((item) =>
        item.source_id === candidate.source_id && item.url === candidate.url
          ? { ...item, imported: true }
          : item,
      );
    } catch (e) {
      vaultError = e instanceof Error ? e.message : String(e);
    } finally {
      busy = null;
    }
  }

  async function setStatus(id: string, status: FeedStatus): Promise<void> {
    busy = id;
    try {
      await comms.setStatus(id, status);
      entries = entries.filter((entry) => entry.id !== id);
    } finally {
      busy = null;
    }
  }

  function dayLabel(day: string): string {
    if (day === "relevance") return "For you";
    const d = new Date(`${day}T00:00:00`);
    if (Number.isNaN(d.getTime())) return day;
    const today = new Date();
    today.setHours(0, 0, 0, 0);
    const diff = Math.round((today.getTime() - d.getTime()) / 86_400_000);
    if (diff === 0) return "Today";
    if (diff === 1) return "Yesterday";
    return d.toLocaleDateString("en-GB", { weekday: "short", day: "numeric", month: "long" });
  }
</script>

<PageHeader
  badge="Feed"
  title={view === "discover" ? "Discover" : "Inbox"}
  desc={view === "discover"
    ? "Scan active sources and review relevant opportunities. Scouting evaluates and stores them separately while keeping them in the same Feed workspace."
    : "Only new, unreviewed articles, media, repositories, security reports, and system updates. Processed entries remain available in the library."}
/>

<FeedNav active={view} />

{#if view === "discover"}
  <DiscoverView />
{:else}
{#if modelStatus}
  <ModelStatus status={modelStatus} />
{/if}
<form
  class="paste"
  onsubmit={(e) => {
    e.preventDefault();
    void ingest();
  }}
>
  <input
    class="input"
    type="url"
    bind:value={pasted}
    placeholder="Add a link — YouTube, GitHub, arXiv, Reddit, article"
    disabled={ingesting}
  />
  <button class="btn btn-primary" type="submit" disabled={ingesting || pasted.trim() === ""}>
    {#if ingesting}<Icon name="loader" size={14} /> reading…{:else}<Icon name="plus" size={14} /> Ingest{/if}
  </button>
</form>

{#if ingestError}
  <p class="notice">
    <Icon name="wifi-off" />
    {ingestError}
  </p>
{/if}

<div class="filters">
  <div class="segmented">
    {#each STREAMS as s (s.value)}
      <button class:active={stream === s.value} onclick={() => (stream = s.value)}>
        {s.label}
      </button>
    {/each}
  </div>
  <div class="segmented">
    {#each RANGES as r (r)}
      <button class:active={days === r} onclick={() => (days = r)}>{r}d</button>
    {/each}
  </div>
  <div class="segmented">
    <button class:active={order === "recent"} onclick={() => (order = "recent")}>New</button>
    <button class:active={order === "relevance"} onclick={() => (order = "relevance")}>
      For you
    </button>
  </div>
  <button class="btn" onclick={refreshRelevance} disabled={relevanceBusy}>
    {#if relevanceBusy}<Icon name="loader" size={13} /> Comparing…{:else}Compare with TELOS{/if}
  </button>
  <button class="btn" onclick={scanVault} disabled={vaultBusy}>
    <Icon name="database" size={13} /> Vault-Links
  </button>
  <button class="btn" onclick={openSources} disabled={sourcesBusy}>
    <Icon name="refresh" size={13} /> Sources
  </button>
</div>

{#if relevanceNotice}
  <p class="context-note">{relevanceNotice}</p>
{/if}

{#if sourcesOpen}
  <section class="source-panel card">
    <div class="vault-head">
      <div>
        <p class="eyebrow mono">General feed</p>
        <h2>Watched sources</h2>
      </div>
      <div class="panel-actions">
        <button
          class="btn"
          disabled={sourcesBusy || feedSources.length === 0}
          onclick={() => scanSources()}
        >
          {sourcesBusy ? "Scanning…" : "Scan all"}
        </button>
        <button class="btn" onclick={() => (sourcesOpen = false)} aria-label="Close sources">
          <Icon name="close" size={13} />
        </button>
      </div>
    </div>
    <p class="vault-copy">
      Public awareness sources become regular Feed entries. Ranking only evaluates new or changed
      content.
    </p>
    {#if sourceNotice}<p class="context-note source-notice">{sourceNotice}</p>{/if}
    {#if sourcesBusy && feedSources.length === 0}
      <p class="muted"><Icon name="loader" size={12} /> Loading sources…</p>
    {:else if feedSources.length === 0}
      <p class="muted">No general Feed sources are enabled.</p>
    {:else}
      <ul class="source-list">
        {#each feedSources as source (source.id)}
          <li>
            <div>
              <p class="lead">{sourceLabel(source)}</p>
              <p class="meta mono">
                {sourceRunLabel(source.last_run_at)} · max. {source.limit}
              </p>
            </div>
            <a class="btn" href={source.source_url} target="_blank" rel="noreferrer">
              Source <Icon name="external" size={12} />
            </a>
            <button class="btn" disabled={sourcesBusy} onclick={() => scanSources(source.id)}>
              Scan
            </button>
          </li>
        {/each}
      </ul>
    {/if}
  </section>
{/if}

{#if vaultOpen}
  <section class="vault card">
    <div class="vault-head">
      <div>
        <p class="eyebrow mono">Explicit sources</p>
        <h2>Links from Obsidian</h2>
      </div>
      <button class="btn" onclick={() => (vaultOpen = false)} aria-label="Close vault list">
        <Icon name="close" size={13} />
      </button>
    </div>
    <p class="vault-copy">
      Axon reads only configured notes or headings. A link is fetched and imported only after you
      select it.
    </p>
    {#if vaultBusy}
      <p class="muted"><Icon name="loader" size={12} /> Scanning allowed sources…</p>
    {:else if vaultError}
      <p class="notice">{vaultError}</p>
    {:else if vaultLinks.length === 0}
      <p class="muted">No new or allowed links found.</p>
    {:else}
      <ul class="vault-list">
        {#each vaultLinks as candidate (`${candidate.source_id}:${candidate.url}`)}
          <li>
            <div>
              <p class="lead">{candidate.label ?? candidate.url}</p>
              <p class="meta mono">{candidate.source_ref}</p>
            </div>
            <button
              class="btn"
              disabled={candidate.imported || busy === candidate.id}
              onclick={() => importVault(candidate)}
            >
              {candidate.imported ? "In Feed" : busy === candidate.id ? "Reading…" : "Import"}
            </button>
          </li>
        {/each}
      </ul>
    {/if}
  </section>
{/if}

{#if offline}
  <p class="notice">
    <Icon name="wifi-off" />
    Feed could not be started. See <a href="/capabilities">Capabilities</a> for details.
  </p>
{:else if loading && entries.length === 0}
  <p class="notice muted">Loading…</p>
{:else if grouped.length === 0}
  <p class="notice muted">Nothing in this period.</p>
{:else}
  {#each grouped as [day, items] (day)}
    <section class="day">
      <h2>{dayLabel(day)} <span class="count mono">{items.length}</span></h2>
      <ul>
        {#each rowsFor(items) as row (row.id)}
          {#if row.kind === "run"}
            <li class="card run" class:open={expandedRuns.has(row.id)}>
              <button
                class="run-head"
                onclick={() => toggleRun(row.id)}
                aria-expanded={expandedRuns.has(row.id)}
              >
                <span class="chevron"><Icon name="arrow-right" size={13} /></span>
                <span class="text">{row.label}</span>
                <span class="count mono">{row.entries.length}</span>
              </button>
              {#if expandedRuns.has(row.id)}
                <ul class="run-items">
                  {#each row.entries as e (e.id)}
                    {@render entryCard(e)}
                  {/each}
                </ul>
              {/if}
            </li>
          {:else}
            {@render entryCard(row.entry)}
          {/if}
        {/each}
      </ul>
    </section>
  {/each}
{/if}

{#snippet entryCard(e: FeedEntry)}
          <li class="card entry">
            <div class="row">
              <a class="title" href={`/feed/${e.id}`}>
                <span class="kind tag mono">{KIND_LABEL[e.kind] ?? e.kind}</span>
                <span class="text">{e.title ?? e.url}</span>
              </a>
              <div class="acts">
                <a class="btn" href={e.url} target="_blank" rel="noreferrer" aria-label="Original">
                  <Icon name="external" size={13} />
                </a>
                {#if busy === e.id}
                  <span class="btn"><Icon name="loader" size={13} /></span>
                {:else}
                  <button
                    class="btn"
                    class:kept={e.status === "keeper"}
                    onclick={() => setStatus(e.id, e.status === "keeper" ? "new" : "keeper")}
                    aria-label="Keep"
                  >
                    <Icon name="check" size={13} />
                  </button>
                  <button
                    class="btn"
                    onclick={() => setStatus(e.id, "dismissed")}
                    aria-label="Dismiss"
                  >
                    <Icon name="close" size={13} />
                  </button>
                {/if}
              </div>
            </div>

            {#if e.author}<p class="meta mono">{e.author}</p>{/if}
            {#if e.evaluation}
              <div class="evaluation-compact">
                <EvaluationBreakdown evaluation={e.evaluation} compact />
              </div>
            {:else if e.relevance}
              <p class="relevance">
                <span>{e.relevance.profile_label}</span>
                <span class="mono">{e.relevance.score.toFixed(2)}</span>
                <span class="method">{e.relevance.mode}</span>
              </p>
            {/if}
            {#if e.summary}
              <p class="preview">{e.summary}</p>
            {:else if ingested === e.id}
              <p class="muted pending">
                <Icon name="loader" size={12} /> Summary is running — it will appear after the next load.
              </p>
            {/if}
          </li>
{/snippet}

{#if triage.length > 0}
  <section class="day">
    <h2>Inbox proposals <span class="count mono">{triage.length}</span></h2>
    <ul>
      {#each triage as t (t.id)}
        <li class="card entry">
          <p class="lead">{t.subject}</p>
          <p class="meta mono">{t.from_addr} → {t.stream}</p>
          <p class="muted">{t.rationale}</p>
        </li>
      {/each}
    </ul>
  </section>
{/if}
{/if}

<style>
  .paste {
    display: flex;
    gap: 0.5rem;
    margin-bottom: 0.75rem;
  }

  .paste .input {
    flex: 1;
    min-width: 0;
  }

  .paste .btn {
    flex-shrink: 0;
  }

  .filters {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0.5rem;
    margin-bottom: 1.25rem;
  }

  .context-note {
    margin: -0.55rem 0 1rem;
    color: var(--text-secondary);
    font-size: 0.75rem;
  }

  .vault {
    padding: 1rem;
    margin-bottom: 1.25rem;
  }

  .source-panel {
    padding: 1rem;
    margin-bottom: 1.25rem;
  }

  .vault-head,
  .vault-list li,
  .source-list li {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 1rem;
  }

  .panel-actions,
  .source-list li {
    display: flex;
    align-items: center;
    gap: 0.5rem;
  }

  .vault-head h2 {
    margin: 0.15rem 0 0;
    color: var(--text-primary);
    font-size: 1rem;
  }

  .eyebrow {
    margin: 0;
    color: var(--primary);
    font-size: 0.625rem;
    text-transform: uppercase;
    letter-spacing: 0.06em;
  }

  .vault-copy {
    max-width: 62ch;
    margin: 0.6rem 0 0.9rem;
    color: var(--text-secondary);
    font-size: 0.75rem;
  }

  .vault-list {
    gap: 0;
  }

  .source-list {
    gap: 0;
  }

  .source-list li {
    padding: 0.7rem 0;
    border-top: 1px solid var(--card-border);
  }

  .source-list li > div:first-child {
    min-width: 0;
    flex: 1;
  }

  .source-list .btn {
    flex-shrink: 0;
  }

  .source-notice {
    margin: 0 0 0.75rem;
  }

  .vault-list li {
    align-items: center;
    padding: 0.65rem 0;
    border-top: 1px solid var(--card-border);
  }

  .vault-list .lead {
    overflow-wrap: anywhere;
  }

  .segmented {
    display: inline-flex;
    gap: 0.125rem;
    padding: 0.125rem;
    border-radius: var(--radius-md);
    background-color: var(--surface);
  }

  .segmented button {
    font: inherit;
    font-size: 0.75rem;
    font-weight: 500;
    padding: 0.3rem 0.6rem;
    border: 0;
    border-radius: var(--radius-sm);
    background-color: transparent;
    color: var(--text-secondary);
    cursor: pointer;
  }

  .segmented button.active {
    background-color: var(--card-bg);
    color: var(--primary);
    box-shadow: var(--card-shadow);
  }

  .day {
    margin-bottom: 1.75rem;
  }

  h2 {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    margin: 0 0 0.6rem;
    font-size: 0.8125rem;
    font-weight: 600;
    color: var(--text-secondary);
  }

  .count {
    font-size: 0.625rem;
    color: var(--text-tertiary);
  }

  ul {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }

  .entry {
    padding: 0.75rem;
  }

  /* A collector run, collapsed to one row until asked to open. */
  .run {
    padding: 0;
    overflow: hidden;
  }

  .run-head {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    width: 100%;
    padding: 0.625rem 0.75rem;
    background: none;
    border: 0;
    color: inherit;
    font: inherit;
    text-align: left;
    cursor: pointer;
  }

  .run-head:hover {
    background: var(--surface-hover, rgba(127, 127, 127, 0.08));
  }

  .run-head .text {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .chevron {
    display: flex;
    color: var(--text-tertiary);
    transition: transform 120ms ease;
  }

  .run.open .chevron {
    transform: rotate(90deg);
  }

  .run-items {
    padding: 0 0.5rem 0.5rem;
  }

  .row {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 0.75rem;
  }

  .title {
    display: flex;
    align-items: baseline;
    gap: 0.5rem;
    flex: 1;
    min-width: 0;
    padding: 0;
    border: 0;
    background: none;
    font: inherit;
    text-align: left;
    color: inherit;
    cursor: pointer;
    text-decoration: none;
  }

  .title:hover .text {
    color: var(--primary);
  }

  .title .text {
    font-size: 0.875rem;
    font-weight: 500;
  }

  .kind {
    flex-shrink: 0;
    font-size: 0.5625rem;
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }

  .acts {
    display: flex;
    gap: 0.15rem;
    flex-shrink: 0;
  }

  .acts .btn {
    padding: 0.3rem;
  }

  .acts .kept {
    color: var(--success);
  }

  .meta {
    margin: 0.3rem 0 0;
    font-size: 0.625rem;
    color: var(--text-tertiary);
  }

  .relevance {
    display: flex;
    flex-wrap: wrap;
    gap: 0.35rem;
    margin: 0.45rem 0 0;
    color: var(--primary);
    font-size: 0.6875rem;
  }

  .evaluation-compact {
    max-width: 32rem;
    margin-top: 0.55rem;
    padding-top: 0.5rem;
    border-top: 1px solid var(--card-border);
  }

  .relevance .method {
    color: var(--text-tertiary);
  }

  .preview {
    display: -webkit-box;
    overflow: hidden;
    margin: 0.45rem 0 0;
    color: var(--text-secondary);
    font-size: 0.75rem;
    line-height: 1.45;
    -webkit-box-orient: vertical;
    -webkit-line-clamp: 2;
    line-clamp: 2;
  }

  .lead {
    margin: 0;
    font-size: 0.875rem;
    font-weight: 500;
  }

  .muted {
    color: var(--text-tertiary);
    font-size: 0.75rem;
    margin: 0.25rem 0 0;
  }

  /* The shared Icon renders as a block, so an inline one needs a flex line. */
  .pending {
    display: flex;
    align-items: center;
    gap: 0.4rem;
  }

  .notice {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.75rem;
    border-radius: var(--radius-md);
    background-color: var(--warning-soft);
    color: var(--warning);
    font-size: 0.8125rem;
  }

  .notice.muted {
    background-color: transparent;
    color: var(--text-tertiary);
  }

  .notice a {
    color: inherit;
    text-decoration: underline;
  }
</style>
