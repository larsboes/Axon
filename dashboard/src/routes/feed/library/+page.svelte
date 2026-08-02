<script lang="ts">
  import { onMount } from "svelte";
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
    type FeedKind,
    type FeedStatus,
  } from "$lib/api";

  type KindFilter = "all" | FeedKind;
  type StatusFilter = "all" | FeedStatus;
  type TripFilter = "all" | "matched" | string;
  type Order = "smart" | "recent" | "oldest";

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
  const KIND_MARK: Record<string, string> = {
    youtube: "YT",
    instagram: "IG",
    podcast: "PO",
    article: "AR",
    mail: "MA",
    github: "GH",
    arxiv: "AX",
    reddit: "RE",
  };
  const STATUS_LABEL: Record<FeedStatus, string> = {
    new: "New",
    keeper: "Kept",
    dismissed: "Dismissed",
  };

  let entries = $state<FeedEntry[]>([]);
  let loading = $state(true);
  let offline = $state(false);
  let search = $state("");
  let kind = $state<KindFilter>("all");
  let status = $state<StatusFilter>("all");
  let lens = $state("all");
  let trip = $state<TripFilter>("all");
  let order = $state<Order>("smart");
  let busy = $state<string | null>(null);
  let modelStatus = $state<CommsEvaluationStatus | null>(null);

  const kinds = $derived(
    [...new Set(entries.map((entry) => entry.kind))].sort((a, b) =>
      (KIND_LABEL[a] ?? a).localeCompare(KIND_LABEL[b] ?? b, "en"),
    ),
  );
  const lenses = $derived(
    [...new Set(entries.flatMap((entry) => entry.relevance?.profile_label ?? []))].sort((a, b) =>
      a.localeCompare(b, "en"),
    ),
  );
  const filtered = $derived.by(() => {
    const term = search.trim().toLocaleLowerCase("en-GB");
    const result = entries.filter((entry) => {
      if (kind !== "all" && entry.kind !== kind) return false;
      if (status !== "all" && entry.status !== status) return false;
      if (lens !== "all" && entry.relevance?.profile_label !== lens) return false;
      const travel = travelFactor(entry);
      if (trip === "matched" && !travel?.context) return false;
      if (trip !== "all" && trip !== "matched" && travel?.context?.id !== trip) return false;
      if (!term) return true;
      return [
        entry.title,
        entry.author,
        entry.summary,
        entry.url,
        entry.relevance?.profile_label,
        travel?.context?.label,
        travel?.context?.matched_terms.join(" "),
      ]
        .filter(Boolean)
        .some((value) => value?.toLocaleLowerCase("en-GB").includes(term));
    });
    return result.sort(compareEntries);
  });
  const groups = $derived.by(() => {
    const grouped = new Map<string, FeedEntry[]>();
    for (const entry of filtered) {
      const group = entry.relevance?.profile_label ?? KIND_LABEL[entry.kind] ?? entry.kind;
      const items = grouped.get(group);
      if (items) items.push(entry);
      else grouped.set(group, [entry]);
    }
    return [...grouped.entries()].sort(([, left], [, right]) => {
      if (order === "smart") {
        const relevanceDelta = averageRelevance(right) - averageRelevance(left);
        if (Math.abs(relevanceDelta) > 0.001) return relevanceDelta;
      }
      if (order === "oldest") return dateValue(left[0]) - dateValue(right[0]);
      return dateValue(right[0]) - dateValue(left[0]);
    });
  });
  const typeCounts = $derived.by(() => {
    const counts = new Map<string, number>();
    for (const entry of entries) counts.set(entry.kind, (counts.get(entry.kind) ?? 0) + 1);
    return [...counts.entries()].sort((a, b) => b[1] - a[1]);
  });
  const keepers = $derived(entries.filter((entry) => entry.status === "keeper").length);
  const newCount = $derived(entries.filter((entry) => entry.status === "new").length);
  const evaluationCount = $derived(entries.filter((entry) => entry.evaluation).length);
  const travelMatchCount = $derived(entries.filter((entry) => travelFactor(entry)?.context).length);
  const recentCount = $derived(
    entries.filter((entry) => Date.now() - dateValue(entry) <= 30 * 86_400_000).length,
  );
  const travelLanes = $derived.by(() => {
    const lanes = new Map<
      string,
      {
        id: string;
        label: string;
        dateStart: string | null;
        dateEnd: string | null;
        entries: FeedEntry[];
      }
    >();
    for (const plan of modelStatus?.travel_context?.plans ?? []) {
      lanes.set(plan.id, {
        id: plan.id,
        label: plan.label,
        dateStart: plan.date_start,
        dateEnd: plan.date_end,
        entries: [],
      });
    }
    for (const entry of entries) {
      const factor = travelFactor(entry);
      const context = factor?.context;
      if (!context || factor.score <= 0) continue;
      const lane = lanes.get(context.id);
      if (lane) lane.entries.push(entry);
      else {
        lanes.set(context.id, {
          id: context.id,
          label: context.label,
          dateStart: context.date_start,
          dateEnd: context.date_end,
          entries: [entry],
        });
      }
    }
    return [...lanes.values()]
      .map((lane) => ({
        ...lane,
        entries: [...lane.entries].sort(compareEntries),
        average:
          lane.entries.length === 0
            ? 0
            : lane.entries.reduce(
                (sum, entry) => sum + (travelFactor(entry)?.score ?? 0),
                0,
              ) / lane.entries.length,
      }))
      .sort((left, right) => (left.dateStart ?? "9999").localeCompare(right.dateStart ?? "9999"));
  });

  onMount(() => {
    void (async () => {
      try {
        await axonStatus.start("comms").catch(() => undefined);
        const [feed, status] = await Promise.all([
          comms.feed({ days: 3650, includeDismissed: true }),
          comms.evaluationStatus().catch(() => null),
        ]);
        entries = feed;
        modelStatus = status;
      } catch {
        offline = true;
      } finally {
        loading = false;
      }
    })();
  });

  function dateValue(entry: FeedEntry): number {
    const value = new Date(entry.created_at).getTime();
    return Number.isNaN(value) ? 0 : value;
  }

  function compareEntries(left: FeedEntry, right: FeedEntry): number {
    if (order === "oldest") return dateValue(left) - dateValue(right);
    if (order === "smart") {
      const relevanceDelta =
        (right.evaluation?.overall_score ?? right.relevance?.score ?? -1) -
        (left.evaluation?.overall_score ?? left.relevance?.score ?? -1);
      if (Math.abs(relevanceDelta) > 0.001) return relevanceDelta;
    }
    return dateValue(right) - dateValue(left);
  }

  function averageRelevance(items: FeedEntry[]): number {
    const scores = items.flatMap(
      (item) => item.evaluation?.overall_score ?? item.relevance?.score ?? [],
    );
    if (scores.length === 0) return -1;
    return scores.reduce((sum, score) => sum + score, 0) / scores.length;
  }

  function travelFactor(entry: FeedEntry) {
    return entry.evaluation?.factors.find((factor) => factor.key === "travel") ?? null;
  }

  function shortRange(start: string | null, end: string | null): string {
    const format = (value: string | null) =>
      value
        ? new Intl.DateTimeFormat("en-GB", { day: "2-digit", month: "short" }).format(
            new Date(`${value}T12:00:00`),
          )
        : "open";
    return `${format(start)} – ${format(end)}`;
  }

  function preview(summary: string | null): string | null {
    if (!summary) return null;
    return summary
      .replace(/^\s*[-*]\s+/gm, "")
      .replace(/\s+/g, " ")
      .trim();
  }

  function formatDate(entry: FeedEntry): string {
    const date = new Date(entry.created_at);
    if (Number.isNaN(date.getTime())) return entry.day;
    return date.toLocaleDateString("en-GB", { day: "numeric", month: "short", year: "numeric" });
  }

  function share(count: number): string {
    return `${entries.length === 0 ? 0 : (count / entries.length) * 100}%`;
  }

  async function setStatus(id: string, next: FeedStatus): Promise<void> {
    if (busy) return;
    busy = id;
    try {
      await comms.setStatus(id, next);
      entries = entries.map((entry) => (entry.id === id ? { ...entry, status: next } : entry));
    } finally {
      busy = null;
    }
  }
</script>

<PageHeader
  badge="Feed"
  title="Library"
  desc="The entire Feed collection remains searchable. TELOS lenses form the shelves, while type, status, and time remain independent filters."
/>

<FeedNav active="library" />

{#if offline}
  <p class="notice">
    <Icon name="wifi-off" />
    The library could not be loaded.
  </p>
{:else if loading}
  <p class="notice muted"><Icon name="loader" size={13} /> Loading collection…</p>
{:else}
  {#if modelStatus}
    <ModelStatus status={modelStatus} />
  {/if}
  <section class="collection-summary" aria-labelledby="bestand-title">
    <div class="summary-copy">
      <p class="eyebrow mono">Collection</p>
      <h2 id="bestand-title">{entries.length} entries in the collection</h2>
      <p>
        New and processed content belongs to the same collection. These figures describe the
        collection; they are not a model-generated success score.
      </p>
    </div>
    <dl>
      <div><dt>New</dt><dd>{newCount}</dd></div>
      <div><dt>Last 30 days</dt><dd>{recentCount}</dd></div>
      <div><dt>Kept</dt><dd>{keepers}</dd></div>
      <div><dt>With ranking</dt><dd>{evaluationCount}</dd></div>
      <div><dt>Travel relevance</dt><dd>{travelMatchCount}</dd></div>
    </dl>
  </section>

  {#if typeCounts.length > 0}
    <section class="distribution" aria-labelledby="verteilung-title">
      <div class="distribution-head">
        <h2 id="verteilung-title">Sources in the collection</h2>
        <span class="mono">{typeCounts.length} types</span>
      </div>
      <div class="bar" aria-hidden="true">
        {#each typeCounts as [entryKind, count] (entryKind)}
          <span class={`kind-${entryKind}`} style:width={share(count)}></span>
        {/each}
      </div>
      <ul>
        {#each typeCounts as [entryKind, count] (entryKind)}
          <li>
            <span class={`dot kind-${entryKind}`}></span>
            <span>{KIND_LABEL[entryKind] ?? entryKind}</span>
            <strong class="mono">{count}</strong>
          </li>
        {/each}
      </ul>
    </section>
  {/if}

  {#if travelLanes.length > 0}
    <section class="travel-context" aria-labelledby="reise-kontext-title">
      <header>
        <div>
          <p class="eyebrow mono">Planned trips</p>
          <h2 id="reise-kontext-title">What may become relevant while travelling</h2>
        </div>
        <a href="/travel">Open Travel <Icon name="arrow-right" size={13} /></a>
      </header>
      <div class="travel-timeline">
        {#each travelLanes as lane (lane.id)}
          <div class="travel-lane">
            <button type="button" onclick={() => (trip = lane.id)}>
              <span class="timeline-date mono">{shortRange(lane.dateStart, lane.dateEnd)}</span>
              <strong>{lane.label}</strong>
              <small>
                {lane.entries.length === 0
                  ? "No matching Feed entry yet"
                  : `${lane.entries.length} matches · average ${Math.round(lane.average * 100)} travel fit`}
              </small>
            </button>
            {#if lane.entries.length > 0}
              <ol>
                {#each lane.entries.slice(0, 3) as entry (entry.id)}
                  <li>
                    <a href={`/feed/${entry.id}`}>{entry.title ?? entry.url}</a>
                    <span class="mono">{Math.round((travelFactor(entry)?.score ?? 0) * 100)}</span>
                  </li>
                {/each}
              </ol>
            {/if}
          </div>
        {/each}
      </div>
    </section>
  {/if}

  <section class="controls" aria-label="Filter library">
    <label class="search">
      <Icon name="search" size={14} />
      <input bind:value={search} type="search" placeholder="Title, author, content, or lens" />
    </label>
    <select bind:value={kind} aria-label="Source type">
      <option value="all">All types</option>
      {#each kinds as entryKind (entryKind)}
        <option value={entryKind}>{KIND_LABEL[entryKind] ?? entryKind}</option>
      {/each}
    </select>
    <select bind:value={status} aria-label="Status">
      <option value="all">Any status</option>
      <option value="new">New</option>
      <option value="keeper">Kept</option>
      <option value="dismissed">Dismissed</option>
    </select>
    <select bind:value={lens} aria-label="TELOS lens">
      <option value="all">All lenses</option>
      {#each lenses as profile (profile)}
        <option value={profile}>{profile}</option>
      {/each}
    </select>
    <select bind:value={trip} aria-label="Travel relevance">
      <option value="all">All travel relevance</option>
      <option value="matched">Only travel-related</option>
      {#each travelLanes as lane (lane.id)}
        <option value={lane.id}>{lane.label}</option>
      {/each}
    </select>
    <select bind:value={order} aria-label="Sort order">
      <option value="smart">Best match first</option>
      <option value="recent">Newest first</option>
      <option value="oldest">Oldest first</option>
    </select>
  </section>

  <p class="result-count mono">{filtered.length} of {entries.length} visible</p>

  {#if groups.length === 0}
    <p class="empty">No entries match these filters.</p>
  {:else}
    {#each groups as [group, items] (group)}
      <section class="shelf">
        <header>
          <div>
            <p class="shelf-label mono">{items[0].relevance ? "TELOS lens" : "Source type"}</p>
            <h2>{group}</h2>
          </div>
          <p>
            <span class="mono">{items.length}</span>
            {items.length === 1 ? "entry" : "entries"}
          </p>
        </header>
        <div class="gallery">
          {#each items as entry (entry.id)}
            <article class:dismissed={entry.status === "dismissed"}>
              <div class:media={entry.stream === "media"} class="tile-mark" aria-hidden="true">
                {KIND_MARK[entry.kind] ?? entry.kind.slice(0, 2).toUpperCase()}
              </div>
              <div class="tile-body">
                <div class="tile-meta">
                  <span>{KIND_LABEL[entry.kind] ?? entry.kind}</span>
                  <span>{formatDate(entry)}</span>
                  <span class={`status status-${entry.status}`}>{STATUS_LABEL[entry.status]}</span>
                </div>
                <h3><a href={`/feed/${entry.id}`}>{entry.title ?? entry.url}</a></h3>
                {#if entry.author}<p class="author">{entry.author}</p>{/if}
                {#if preview(entry.summary)}
                  <p class="preview">{preview(entry.summary)}</p>
                {:else}
                  <p class="preview missing">No summary yet.</p>
                {/if}
                {#if entry.evaluation}
                  <div class="tile-evaluation">
                    <EvaluationBreakdown evaluation={entry.evaluation} compact />
                  </div>
                {/if}
                <footer>
                  {#if entry.relevance}
                    <p class="fit">
                      <span>{entry.relevance.profile_label}</span>
                      <span class="mono">{entry.relevance.score.toFixed(2)}</span>
                    </p>
                  {:else}
                    <span></span>
                  {/if}
                  <div class="actions">
                    <a
                      href={entry.url}
                      target="_blank"
                      rel="noreferrer"
                      aria-label="Open original"
                    >
                      <Icon name="external" size={13} />
                    </a>
                    {#if busy === entry.id}
                      <span><Icon name="loader" size={13} /></span>
                    {:else if entry.status === "new"}
                      <button onclick={() => setStatus(entry.id, "keeper")}>Keep</button>
                      <button onclick={() => setStatus(entry.id, "dismissed")}>Dismiss</button>
                    {:else}
                      <button onclick={() => setStatus(entry.id, "new")}>Mark new</button>
                    {/if}
                  </div>
                </footer>
              </div>
            </article>
          {/each}
        </div>
      </section>
    {/each}
  {/if}
{/if}

<style>
  .notice {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    color: var(--warning);
  }

  .notice.muted {
    color: var(--text-tertiary);
  }

  .collection-summary {
    display: grid;
    grid-template-columns: minmax(16rem, 1.35fr) minmax(22rem, 1fr);
    align-items: end;
    gap: clamp(2rem, 5vw, 5rem);
    padding: 1.75rem 0 1.5rem;
    border-bottom: 1px solid var(--card-border);
  }

  .eyebrow,
  .shelf-label {
    margin: 0 0 0.25rem;
    color: var(--primary);
    font-size: 0.625rem;
    text-transform: uppercase;
    letter-spacing: 0.06em;
  }

  .summary-copy h2 {
    margin: 0;
    font-size: clamp(1.55rem, 3vw, 2.4rem);
    font-weight: 580;
    letter-spacing: -0.035em;
  }

  .summary-copy > p:last-child {
    max-width: 58ch;
    margin: 0.65rem 0 0;
    color: var(--text-secondary);
    font-size: 0.8125rem;
    line-height: 1.55;
  }

  dl {
    display: grid;
    grid-template-columns: repeat(5, minmax(0, 1fr));
    margin: 0;
  }

  dl div {
    padding-left: 0.8rem;
    border-left: 1px solid var(--card-border);
  }

  dt {
    color: var(--text-tertiary);
    font-size: 0.625rem;
  }

  dd {
    margin: 0.3rem 0 0;
    font-family: var(--font-mono);
    font-size: 1.35rem;
  }

  .distribution {
    padding: 1.25rem 0;
    border-bottom: 1px solid var(--card-border);
  }

  .distribution-head {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
  }

  .distribution h2,
  .shelf h2 {
    margin: 0;
    font-size: 0.875rem;
    font-weight: 600;
  }

  .distribution-head > span,
  .result-count {
    color: var(--text-tertiary);
    font-size: 0.625rem;
  }

  .bar {
    display: flex;
    height: 0.35rem;
    margin-top: 0.7rem;
    overflow: hidden;
    background: var(--surface);
  }

  .bar span {
    min-width: 2px;
  }

  .distribution ul {
    display: flex;
    flex-wrap: wrap;
    gap: 0.4rem 1rem;
    margin: 0.65rem 0 0;
    padding: 0;
    list-style: none;
  }

  .distribution li {
    display: grid;
    grid-template-columns: 0.45rem auto auto;
    align-items: center;
    gap: 0.35rem;
    color: var(--text-secondary);
    font-size: 0.6875rem;
  }

  .distribution li strong {
    color: var(--text-tertiary);
    font-weight: 500;
  }

  .dot {
    width: 0.4rem;
    height: 0.4rem;
  }

  .kind-youtube,
  .kind-instagram,
  .kind-podcast {
    background: var(--accent);
  }

  .kind-github,
  .kind-arxiv,
  .kind-reddit {
    background: var(--primary);
  }

  .kind-article,
  .kind-mail {
    background: var(--text-tertiary);
  }

  .travel-context {
    padding: 1.25rem 0;
    border-bottom: 1px solid var(--card-border);
  }

  .travel-context > header,
  .travel-context > header a,
  .travel-lane > button,
  .travel-lane li {
    display: flex;
    align-items: center;
  }

  .travel-context > header {
    justify-content: space-between;
    gap: 1rem;
    margin-bottom: 0.8rem;
  }

  .travel-context h2 {
    margin: 0;
    font-size: 1rem;
    font-weight: 600;
  }

  .travel-context > header a {
    gap: 0.35rem;
    color: var(--text-secondary);
    font-size: 0.6875rem;
  }

  .travel-timeline {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(16rem, 1fr));
    border-top: 1px solid var(--card-border);
    border-left: 1px solid var(--card-border);
  }

  .travel-lane {
    min-width: 0;
    padding: 0.8rem;
    border-right: 1px solid var(--card-border);
    border-bottom: 1px solid var(--card-border);
    background: var(--card-bg);
  }

  .travel-lane > button {
    width: 100%;
    flex-direction: column;
    align-items: flex-start;
    gap: 0.15rem;
    padding: 0;
    border: 0;
    background: none;
    color: inherit;
    font: inherit;
    text-align: left;
    cursor: pointer;
  }

  .travel-lane > button:hover strong {
    color: var(--success);
  }

  .timeline-date,
  .travel-lane small {
    color: var(--text-tertiary);
    font-size: 0.5625rem;
  }

  .travel-lane strong {
    font-size: 0.8125rem;
  }

  .travel-lane ol {
    display: grid;
    gap: 0.35rem;
    margin: 0.7rem 0 0;
    padding: 0.65rem 0 0;
    border-top: 1px solid var(--card-border);
    list-style: none;
  }

  .travel-lane li {
    justify-content: space-between;
    gap: 0.5rem;
    min-width: 0;
    font-size: 0.625rem;
  }

  .travel-lane li a {
    overflow: hidden;
    color: var(--text-secondary);
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .travel-lane li span {
    color: var(--success);
  }

  .controls {
    display: grid;
    grid-template-columns: minmax(14rem, 1fr) repeat(5, auto);
    gap: 0.5rem;
    padding: 1rem 0 0.5rem;
  }

  .search {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    min-width: 0;
    padding: 0 0.65rem;
    border: 1px solid var(--input-border);
    border-radius: var(--radius-md);
    background: var(--input-bg);
    color: var(--text-tertiary);
  }

  .search input,
  select {
    min-width: 0;
    border: 1px solid var(--input-border);
    border-radius: var(--radius-md);
    background: var(--input-bg);
    color: var(--text-primary);
    font: inherit;
    font-size: 0.75rem;
  }

  .search input {
    width: 100%;
    padding: 0.55rem 0;
    border: 0;
    outline: 0;
    background: transparent;
  }

  select {
    padding: 0.5rem 1.8rem 0.5rem 0.6rem;
  }

  .result-count {
    margin: 0.15rem 0 1.5rem;
  }

  .empty {
    padding: 3rem 0;
    color: var(--text-tertiary);
  }

  .shelf {
    margin-bottom: 2.5rem;
  }

  .shelf > header {
    display: flex;
    align-items: end;
    justify-content: space-between;
    gap: 1rem;
    margin-bottom: 0.65rem;
    padding-bottom: 0.55rem;
    border-bottom: 1px solid var(--card-border);
  }

  .shelf > header > p {
    margin: 0;
    color: var(--text-tertiary);
    font-size: 0.6875rem;
  }

  .gallery {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(18rem, 1fr));
    gap: 0;
    border-top: 1px solid var(--card-border);
    border-left: 1px solid var(--card-border);
  }

  article {
    display: grid;
    grid-template-columns: 3.25rem minmax(0, 1fr);
    min-height: 14rem;
    border-right: 1px solid var(--card-border);
    border-bottom: 1px solid var(--card-border);
    background: var(--card-bg);
  }

  article.dismissed {
    background: color-mix(in srgb, var(--card-bg) 82%, var(--surface));
  }

  .tile-mark {
    display: flex;
    justify-content: center;
    padding-top: 1rem;
    border-right: 1px solid var(--card-border);
    background: var(--primary-soft);
    color: var(--primary);
    font-family: var(--font-mono);
    font-size: 0.625rem;
    font-weight: 700;
    letter-spacing: 0.08em;
  }

  .tile-mark.media {
    background: var(--accent-soft);
    color: var(--accent);
  }

  .tile-body {
    display: flex;
    min-width: 0;
    flex-direction: column;
    padding: 0.9rem 1rem 0.8rem;
  }

  .tile-meta {
    display: flex;
    flex-wrap: wrap;
    gap: 0.3rem 0.6rem;
    color: var(--text-tertiary);
    font-size: 0.625rem;
  }

  .status {
    color: var(--text-secondary);
  }

  .status-keeper {
    color: var(--success);
  }

  .status-dismissed {
    color: var(--text-tertiary);
  }

  h3 {
    margin: 0.65rem 0 0;
    font-size: 1rem;
    font-weight: 580;
    line-height: 1.25;
    letter-spacing: -0.015em;
  }

  h3 a {
    color: inherit;
    text-decoration: none;
  }

  h3 a:hover {
    color: var(--primary);
  }

  .author {
    margin: 0.35rem 0 0;
    color: var(--text-tertiary);
    font-size: 0.6875rem;
  }

  .preview {
    display: -webkit-box;
    overflow: hidden;
    margin: 0.75rem 0 1rem;
    color: var(--text-secondary);
    font-size: 0.75rem;
    line-height: 1.55;
    -webkit-box-orient: vertical;
    -webkit-line-clamp: 4;
    line-clamp: 4;
  }

  .preview.missing {
    color: var(--text-tertiary);
  }

  .tile-evaluation {
    margin: 0 0 0.8rem;
    padding-top: 0.65rem;
    border-top: 1px solid var(--card-border);
  }

  article footer {
    display: flex;
    align-items: end;
    justify-content: space-between;
    gap: 0.75rem;
    margin-top: auto;
    padding-top: 0.65rem;
    border-top: 1px solid var(--card-border);
  }

  .fit {
    display: flex;
    gap: 0.4rem;
    margin: 0;
    color: var(--primary);
    font-size: 0.625rem;
  }

  .actions {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: 0.4rem;
  }

  .actions a,
  .actions button,
  .actions > span {
    padding: 0;
    border: 0;
    background: transparent;
    color: var(--text-tertiary);
    font: inherit;
    font-size: 0.625rem;
    cursor: pointer;
  }

  .actions a:hover,
  .actions button:hover {
    color: var(--primary);
  }

  @media (max-width: 68rem) {
    .collection-summary {
      grid-template-columns: 1fr;
    }

    .controls {
      grid-template-columns: repeat(2, minmax(0, 1fr));
    }

    .search {
      grid-column: 1 / -1;
    }
  }

  @media (max-width: 40rem) {
    dl {
      grid-template-columns: repeat(2, 1fr);
      gap: 0.9rem 0;
    }

    .controls {
      grid-template-columns: 1fr;
    }

    .search {
      grid-column: auto;
    }

    .gallery {
      grid-template-columns: 1fr;
    }

  }
</style>
