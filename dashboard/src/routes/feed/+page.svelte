<script lang="ts">
  import { link } from "$lib/nav";
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
    type DataClass,
    type FeedEntry,
    type FeedEntryDetail,
    type FeedRun,
    type FeedSource,
    type FeedStatus,
    type FeedStream,
    type MailCategory,
    type TriageItem,
    type TriageSweepStatus,
    type VaultLinkCandidate,
  } from "$lib/api";

  type StreamFilter = "all" | FeedStream;
  type Order = "recent" | "relevance";
  type FeedView = "inbox" | "discover" | "mail";
  type MailStatusFilter = "pending" | "archived" | "trashed" | "missing" | "dismissed" | "legacy" | "all";
  type GmailAction = "archive" | "trash";

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
  const MAIL_CATEGORY_LABEL: Record<MailCategory, string> = {
    aktiv: "Active",
    issue: "Action",
    feed: "Feed",
    werbung: "Advertising",
    belege: "Receipts",
    steuern: "Tax",
    sonstiges: "Other",
  };
  const MAIL_CATEGORY_ORDER = [
    "issue",
    "aktiv",
    "steuern",
    "belege",
    "feed",
    "sonstiges",
    "werbung",
  ] as const satisfies readonly MailCategory[];
  const DATA_CLASSES = ["c0", "c1", "c2", "c3"] as const satisfies readonly DataClass[];

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
  let mailCategory = $state<"all" | MailCategory>("all");
  let mailStatus = $state<MailStatusFilter>("pending");
  let mailSearch = $state("");
  let selectedMail = $state<Set<string>>(new Set());
  let classifierOpen = $state(false);
  let mailBusy = $state<string | null>(null);
  let mailJobBusy = $state<string | null>(null);
  let mailActionError = $state<string | null>(null);
  let confirmingBulkAction = $state<GmailAction | null>(null);
  let bulkCategory = $state<MailCategory>("aktiv");
  let bulkDataClass = $state<DataClass>("c1");
  let syncingMail = $state(false);
  let reconcilingMail = $state(false);
  let reconcileNotice = $state<string | null>(null);
  let syncCursor = $state<string | null>(null);
  let syncExhausted = $state(false);
  let syncNotice = $state<string | null>(null);
  let sweepStatus = $state<TriageSweepStatus | null>(null);
  let scoringMail = $state(false);
  let scoringNotice = $state<string | null>(null);
  let classifyingMailData = $state(false);
  let dataClassNotice = $state<string | null>(null);
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
  const view = $derived.by<FeedView>(() => {
    const requested = page.url.searchParams.get("view");
    if (requested === "discover" || requested === "mail") return requested;
    return "inbox";
  });
  const pageTitle = $derived(
    view === "discover" ? "Discover" : view === "mail" ? "Mail proposals" : "Inbox",
  );
  const pageDescription = $derived(
    view === "discover"
      ? "Scan active sources and review relevant opportunities. Scouting evaluates and stores them separately while keeping them in the same Feed workspace."
      : view === "mail"
        ? "Review and classify mail. Archive, Trash, and Restore update Axon and Gmail together."
        : "Only new, unreviewed articles, media, repositories, security reports, and system updates. Processed entries remain available in the library.",
  );

  const pendingMailCount = $derived(
    triage.filter((item) => item.status === "proposed" || item.status === "approved").length,
  );
  const visibleMail = $derived.by(() => {
    const query = mailSearch.trim().toLocaleLowerCase();
    return triage.filter((item) => {
      const statusMatches =
        mailStatus === "all" ||
        (mailStatus === "pending" &&
          (item.status === "proposed" || item.status === "approved")) ||
        (mailStatus === "archived" && item.status === "archived") ||
        (mailStatus === "trashed" && item.status === "trashed") ||
        (mailStatus === "missing" && item.status === "missing") ||
        (mailStatus === "dismissed" && item.status === "dismissed") ||
        (mailStatus === "legacy" && item.status === "executed");
      if (!statusMatches || (mailCategory !== "all" && item.stream !== mailCategory)) return false;
      if (!query) return true;
      return [item.subject, item.from_addr, item.rationale]
        .filter((value): value is string => Boolean(value))
        .some((value) => value.toLocaleLowerCase().includes(query));
    });
  });
  const mailGroups = $derived.by(() =>
    MAIL_CATEGORY_ORDER.filter(
      (category) => mailCategory === "all" || mailCategory === category,
    ).map((category) => ({
      category,
      items: visibleMail
        .filter((item) => item.stream === category)
        .sort((left, right) => {
          const relevanceDelta =
            (right.relevance[0]?.score ?? Number.NEGATIVE_INFINITY) -
            (left.relevance[0]?.score ?? Number.NEGATIVE_INFINITY);
          if (relevanceDelta !== 0) return relevanceDelta;
          return (right.internal_date ?? "").localeCompare(left.internal_date ?? "");
        }),
    })),
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
      if (view === "mail") {
        // Freshness is allowed to fail on its own: an older comms without the
        // status route should still show the board, not an offline page.
        const [proposals, status] = await Promise.all([
          comms.triage(),
          comms.triageSweepStatus().catch(() => null),
        ]);
        triage = proposals;
        sweepStatus = status;
        offline = false;
        return;
      }
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
    if (view === "discover" || ready) return;
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
    if (view !== "discover" && ready) void load();
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

  /// `source_state` stores its timestamps as epoch seconds in a TEXT column,
  /// unlike every other time on this wire, which arrives as the store's
  /// canonical stamp — `2026-08-27 21:23:35.871+00:00`. Both shapes go through
  /// here so a caller never has to know which one a given field is: passing an
  /// epoch to the Date constructor yields an Invalid Date and renders as nothing
  /// at all. The stamp itself is unchanged work for this function — measured
  /// 2026-08-28, `new Date` reads it and Postgres's old `…871000+00` rendering
  /// to the same instant.
  function runTimeLabel(value: string | null): string | null {
    if (!value) return null;
    const epoch = Number(value);
    const date = Number.isFinite(epoch) && value.trim() !== "" ? new Date(epoch * 1000) : new Date(value);
    if (Number.isNaN(date.getTime())) return value;
    return date.toLocaleString("en-GB", {
      day: "numeric",
      month: "short",
      hour: "2-digit",
      minute: "2-digit",
    });
  }

  function sourceRunLabel(value: string | null): string {
    const label = runTimeLabel(value);
    return label ? `last ${label}` : "not scanned yet";
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

  function mailCategoryLabel(category: MailCategory): string {
    return MAIL_CATEGORY_LABEL[category];
  }

  function dataClassLabel(dataClass: DataClass): string {
    if (dataClass === "c3") return "Secret";
    if (dataClass === "c2") return "Others";
    if (dataClass === "c1") return "Mine";
    return "Public";
  }

  function mailDateLabel(value: string | null): string | null {
    if (!value) return null;
    const date = new Date(value);
    if (Number.isNaN(date.getTime())) return null;
    return new Intl.DateTimeFormat("en-GB", {
      day: "numeric",
      month: "short",
      hour: "2-digit",
      minute: "2-digit",
    }).format(date);
  }

  function purgeDateLabel(value: string | null): string | null {
    if (!value) return null;
    const date = new Date(value);
    if (Number.isNaN(date.getTime())) return null;
    return new Intl.DateTimeFormat("en-GB", {
      day: "numeric",
      month: "short",
      year: "numeric",
    }).format(date);
  }

  function toggleMailSelection(id: string): void {
    const next = new Set(selectedMail);
    if (!next.delete(id)) next.add(id);
    selectedMail = next;
  }

  function mailActionSelectable(item: TriageItem): boolean {
    return (
      (item.status === "proposed" || item.status === "approved") &&
      item.gmail_sync_status !== "queued" &&
      item.gmail_sync_status !== "retrying"
    );
  }

  function selectMailGroup(items: TriageItem[]): void {
    const selectable = items.filter(mailActionSelectable);
    const allSelected = selectable.length > 0 && selectable.every((item) => selectedMail.has(item.id));
    const next = new Set(selectedMail);
    for (const item of selectable) {
      if (allSelected) next.delete(item.id);
      else next.add(item.id);
    }
    selectedMail = next;
  }

  function clearMailSelection(): void {
    selectedMail = new Set();
    confirmingBulkAction = null;
  }

  async function syncMoreMail(): Promise<void> {
    if (syncingMail || syncExhausted) return;
    syncingMail = true;
    mailActionError = null;
    syncNotice = null;
    try {
      const result = await comms.sweepTriage(100, syncCursor);
      syncCursor = result.next_cursor;
      syncExhausted = result.exhausted;
      triage = await comms.triage();
      syncNotice = `${result.fetched} inbox threads reviewed · ${result.new_count} new · ${result.total_stored} stored${result.exhausted ? " · inbox exhausted" : ""}.`;
    } catch (cause) {
      mailActionError = cause instanceof Error ? cause.message : "Inbox sync failed.";
    } finally {
      syncingMail = false;
    }
  }

  async function reconcileGmail(): Promise<void> {
    if (reconcilingMail) return;
    reconcilingMail = true;
    mailActionError = null;
    reconcileNotice = null;
    try {
      const result = await comms.reconcileGmail();
      triage = await comms.triage();
      reconcileNotice = `${result.reconciled} locations checked · ${result.changed} Axon states updated · ${result.recovered} queued actions recovered${result.missing > 0 ? ` · ${result.missing} missing in Gmail` : ""}${result.read_failures > 0 ? ` · ${result.read_failures} unavailable` : ""}. Metadata only; no message content fetched.`;
    } catch (cause) {
      mailActionError = cause instanceof Error ? cause.message : "Gmail reconciliation failed.";
    } finally {
      reconcilingMail = false;
    }
  }

  async function decideGmailJob(id: string, decision: "retry" | "cancel"): Promise<void> {
    if (mailJobBusy) return;
    mailJobBusy = id;
    mailActionError = null;
    try {
      await comms.decideGmailJob(id, decision);
    } catch (cause) {
      mailActionError = cause instanceof Error ? cause.message : "Gmail recovery action failed.";
    } finally {
      try {
        triage = await comms.triage();
      } catch (cause) {
        mailActionError ??= cause instanceof Error ? cause.message : "Mail proposals could not be refreshed.";
      }
      mailJobBusy = null;
    }
  }

  async function scoreMailRelevance(): Promise<void> {
    if (scoringMail) return;
    scoringMail = true;
    mailActionError = null;
    scoringNotice = null;
    try {
      const result = await comms.refreshTriageRelevance(500);
      triage = await comms.triage();
      const method = result.mode ?? "unscored";
      scoringNotice = `${result.scored} proposals compared with ${result.profile_count} TELOS lenses · ${method} · local only.`;
    } catch (cause) {
      mailActionError = cause instanceof Error ? cause.message : "TELOS scoring failed.";
    } finally {
      scoringMail = false;
    }
  }

  async function classifyMailData(): Promise<void> {
    if (classifyingMailData) return;
    classifyingMailData = true;
    mailActionError = null;
    dataClassNotice = null;
    try {
      const result = await comms.refreshTriageDataClasses(2_000);
      triage = await comms.triage();
      dataClassNotice = `${result.reviewed} proposals reviewed · ${result.updated} classified · ${result.preserved_human} human choices preserved · local rules only.`;
    } catch (cause) {
      mailActionError = cause instanceof Error ? cause.message : "Data classification failed.";
    } finally {
      classifyingMailData = false;
    }
  }

  async function applyBulkMailAction(
    action: "dismiss" | "categorize" | "set-data-class" | GmailAction | "waiting" | "clear-waiting",
  ): Promise<void> {
    if (mailBusy || selectedMail.size === 0) return;
    mailBusy = "bulk";
    mailActionError = null;
    try {
      const ids = [...selectedMail];
      const result = await comms.bulkTriage(
        ids,
        action,
        action === "categorize" ? bulkCategory : undefined,
        action === "set-data-class" ? bulkDataClass : undefined,
      );
      const succeeded = new Set(result.succeeded);
      triage = await comms.triage();
      selectedMail = new Set(
        [...selectedMail].filter((id) => !succeeded.has(id)),
      );
      confirmingBulkAction = null;
      if (result.failures.length > 0) {
        mailActionError = `${result.succeeded.length} updated; ${result.failures.length} failed.`;
      }
    } catch (cause) {
      mailActionError = cause instanceof Error ? cause.message : "Bulk action failed.";
    } finally {
      mailBusy = null;
    }
  }
</script>

<PageHeader
  badge="Feed"
  title={pageTitle}
  desc={pageDescription}
/>

<FeedNav active={view} mailCount={ready && !loading ? pendingMailCount : undefined} />

{#if view === "discover"}
  <DiscoverView />
{:else if view === "mail"}
  {#if offline}
    <p class="notice">
      <Icon name="wifi-off" />
      Mail proposals could not be loaded. See <a href={link("/capabilities")}>Capabilities</a> for details.
    </p>
  {:else if loading}
    <p class="notice muted"><Icon name="loader" size={13} /> Loading mail proposals…</p>
  {:else}
    <div class="mail-toolbar">
      <label class="mail-search">
        <span class="sr-only">Search mail proposals</span>
        <Icon name="search" size={13} />
        <input bind:value={mailSearch} placeholder="Search sender or subject" />
      </label>
      <label>
        <span class="sr-only">Filter by category</span>
        <select bind:value={mailCategory}>
          <option value="all">All categories</option>
          {#each MAIL_CATEGORY_ORDER as category (category)}
            <option value={category}>{mailCategoryLabel(category)}</option>
          {/each}
        </select>
      </label>
      <div class="segmented" aria-label="Proposal status">
        <button class:active={mailStatus === "pending"} onclick={() => (mailStatus = "pending")}>Pending</button>
        <button class:active={mailStatus === "archived"} onclick={() => (mailStatus = "archived")}>Archive</button>
        <button class:active={mailStatus === "trashed"} onclick={() => (mailStatus = "trashed")}>Trash</button>
        <button class:active={mailStatus === "missing"} onclick={() => (mailStatus = "missing")}>Missing</button>
        <button class:active={mailStatus === "dismissed"} onclick={() => (mailStatus = "dismissed")}>Dismissed</button>
        <button class:active={mailStatus === "legacy"} onclick={() => (mailStatus = "legacy")}>Legacy</button>
        <button class:active={mailStatus === "all"} onclick={() => (mailStatus = "all")}>All</button>
      </div>
      <button class="btn" disabled={syncingMail || syncExhausted} onclick={syncMoreMail}>
        {syncingMail ? "Syncing…" : syncExhausted ? "Inbox synced" : "Sync next 100"}
      </button>
      <button class="btn" disabled={reconcilingMail || triage.length === 0} onclick={reconcileGmail}>
        {reconcilingMail ? "Checking Gmail…" : "Retry + reconcile Gmail"}
      </button>
      <button class="btn" disabled={scoringMail || triage.length === 0} onclick={scoreMailRelevance}>
        {scoringMail ? "Scoring…" : "Score against TELOS"}
      </button>
      <button class="btn" disabled={classifyingMailData || triage.length === 0} onclick={classifyMailData}>
        {classifyingMailData ? "Classifying…" : "Classify data"}
      </button>
      <button class="btn method-button" class:active={classifierOpen} onclick={() => (classifierOpen = !classifierOpen)}>
        How classification works
      </button>
    </div>

    {#if mailActionError}
      <p class="notice"><Icon name="alert" size={13} /> {mailActionError}</p>
    {/if}
    {#if sweepStatus?.enabled}
      <p class="context-note mail-notice" class:sweep-failing={sweepStatus.consecutive_failures > 0}>
        {#if sweepStatus.consecutive_failures > 0}
          <Icon name="alert" size={13} />
          Scheduled sweep failing — {sweepStatus.last_error} error,
          {sweepStatus.consecutive_failures}
          {sweepStatus.consecutive_failures === 1 ? "run" : "runs"} in a row, backing off.
          {#if sweepStatus.last_success_at}
            Last collected {runTimeLabel(sweepStatus.last_success_at)}.
          {:else}
            It has never completed a run.
          {/if}
        {:else if sweepStatus.last_success_at}
          Scheduled sweep every {sweepStatus.every_minutes} min, newest
          {sweepStatus.max_threads}. Last run {runTimeLabel(sweepStatus.last_success_at)} —
          {sweepStatus.considered_count} considered, {sweepStatus.new_count} new.
        {:else}
          Scheduled sweep every {sweepStatus.every_minutes} min, newest
          {sweepStatus.max_threads}. It has not run yet.
        {/if}
        {#if sweepStatus.quiet_hours}
          Quiet {sweepStatus.quiet_hours.start}:00–{sweepStatus.quiet_hours.end}:00.
        {/if}
      </p>
    {/if}
    {#if syncNotice}<p class="context-note mail-notice">{syncNotice}</p>{/if}
    {#if reconcileNotice}<p class="context-note mail-notice">{reconcileNotice}</p>{/if}
    {#if scoringNotice}<p class="context-note mail-notice">{scoringNotice}</p>{/if}
    {#if dataClassNotice}<p class="context-note mail-notice">{dataClassNotice}</p>{/if}

    {#if selectedMail.size > 0}
      <section class="bulk-bar card" aria-label="Bulk mail actions">
        <strong>{selectedMail.size} selected</strong>
        <div class="bulk-category">
          <select bind:value={bulkCategory} aria-label="Bulk category">
            {#each MAIL_CATEGORY_ORDER as category (category)}
              <option value={category}>{mailCategoryLabel(category)}</option>
            {/each}
          </select>
          <button class="btn" disabled={mailBusy === "bulk"} onclick={() => applyBulkMailAction("categorize")}>Apply category</button>
        </div>
        <div class="bulk-category">
          <select bind:value={bulkDataClass} aria-label="Bulk data class">
            {#each DATA_CLASSES as dataClass (dataClass)}
              <option value={dataClass}>{dataClassLabel(dataClass)}</option>
            {/each}
          </select>
          <button class="btn" disabled={mailBusy === "bulk"} onclick={() => applyBulkMailAction("set-data-class")}>Apply data class</button>
        </div>
        <button class="btn" disabled={mailBusy === "bulk"} onclick={() => applyBulkMailAction("dismiss")}>Dismiss from Axon</button>
        <!-- No confirm step, unlike Archive and Trash: this adds one Gmail label
             and removing it is the button beside it. The confirm exists for the
             two actions that take a thread out of the inbox, and putting one on
             a reversible label would teach the habit of clicking through it. -->
        <button class="btn" disabled={mailBusy === "bulk"} onclick={() => applyBulkMailAction("waiting")}>Mark Waiting in Gmail</button>
        <button class="btn" disabled={mailBusy === "bulk"} onclick={() => applyBulkMailAction("clear-waiting")}>Clear Waiting</button>
        <button class="btn" disabled={mailBusy === "bulk"} onclick={() => (confirmingBulkAction = "archive")}>Archive in Axon + Gmail</button>
        <button class="btn danger" disabled={mailBusy === "bulk"} onclick={() => (confirmingBulkAction = "trash")}>Move to Trash</button>
        <button class="btn" disabled={mailBusy === "bulk"} onclick={clearMailSelection}>Clear</button>
        {#if confirmingBulkAction}
          <div class="bulk-confirm" role="alert">
            <span>
              {confirmingBulkAction === "trash"
                ? `Move ${selectedMail.size} selected threads to Gmail Trash?`
                : `Archive ${selectedMail.size} selected threads in Axon and Gmail?`}
            </span>
            <button class="btn" onclick={() => (confirmingBulkAction = null)}>Cancel</button>
            <button
              class="btn"
              class:danger={confirmingBulkAction === "trash"}
              disabled={mailBusy === "bulk"}
              onclick={() => applyBulkMailAction(confirmingBulkAction!)}
            >
              {mailBusy === "bulk" ? "Applying…" : "Confirm"}
            </button>
          </div>
        {/if}
      </section>
    {/if}

    {#if classifierOpen}
      <aside class="classifier card" aria-label="Mail classification method">
        <div>
          <p class="eyebrow mono">Current method</p>
          <h2>Deterministic rules · local · no AI</h2>
        </div>
        <dl>
          <div><dt>Category inputs</dt><dd>Sender, subject, and whether List-Unsubscribe exists.</dd></div>
          <div><dt>Category method</dt><dd>Private rules first, generic heuristics second, then Active as the safe fallback.</dd></div>
          <div><dt>Relevance inputs</dt><dd>Sender, subject, and Gmail snippet compared with configured TELOS lenses.</dd></div>
          <div><dt>Relevance method</dt><dd>Loopback embedding and reranking only; unavailable local models fall back to labelled lexical similarity.</dd></div>
          <div><dt>Never sent</dt><dd>Message bodies and attachments are not fetched. Mail scoring rejects non-loopback model endpoints.</dd></div>
          <div><dt>TELOS boundary</dt><dd>Scoring reads TELOS. Categories and bulk decisions never rewrite TELOS files.</dd></div>
          <div><dt>Corrections</dt><dd>A category you set here becomes a human override and survives later sweeps.</dd></div>
          <div><dt>Data classes</dt><dd>Public may use approved cloud roles; Mine needs a reviewed pseudonymized derivative; Others and Secret never reach a cloud model, refused by the derivative builder, the tier check, the dispatch re-check against the row's current class, and the database constraint alike. Secret is refused local prompts too, by the same gate the labels are derived from — nothing summarizes, diagrams or charts it.</dd></div>
        </dl>
      </aside>
    {/if}

    {#if visibleMail.length === 0}
      <section class="empty-state">
        <h2>{triage.length === 0 ? "No mail proposals" : "No matching proposals"}</h2>
        <p>{triage.length === 0 ? "Run a bounded Gmail sweep to classify new threads for review." : "Change the search, category, or status filter."}</p>
      </section>
    {:else}
      <div class="mail-board" aria-label="Mail category board">
        {#each mailGroups as group (group.category)}
          <section class="mail-column">
            <header>
              <h2>{mailCategoryLabel(group.category)} <span class="count mono">{group.items.length}</span></h2>
              <button
                class="column-select mono"
                disabled={group.items.length === 0}
                onclick={() => selectMailGroup(group.items)}
              >
                {group.items.length > 0 && group.items.every((item) => selectedMail.has(item.id)) ? "Clear" : "Select all"}
              </button>
            </header>
            <ul class="mail-proposals" aria-label={`${mailCategoryLabel(group.category)} mail proposals`}>
              {#each group.items as proposal (proposal.id)}
                {@const dateLabel = mailDateLabel(proposal.internal_date)}
                {@const topMatch = proposal.relevance[0]}
                <li class="card mail-proposal" class:has-job={proposal.gmail_sync_status === "attention"}>
                  <div class="proposal-row">
                    <label class="card-select" aria-label={`Select ${proposal.subject ?? "mail proposal"}`}>
                      <input
                        type="checkbox"
                        checked={selectedMail.has(proposal.id)}
                        disabled={!mailActionSelectable(proposal)}
                        onchange={() => toggleMailSelection(proposal.id)}
                      />
                    </label>
                    <a
                      class="proposal-summary"
                      href={link(`/feed/${encodeURIComponent(proposal.id)}?source=mail`)}
                    >
                      <span class="chevron"><Icon name="arrow-right" size={12} /></span>
                      <span class="proposal-copy">
                        <span class="lead">{proposal.subject ?? "(No subject)"}</span>
                        <span class="meta mono">
                          {proposal.from_addr ?? "Unknown sender"}
                          {#if dateLabel}<span>· {dateLabel}</span>{/if}
                        </span>
                        <span class="snippet">{proposal.snippet ?? "No preview available."}</span>
                        {#if topMatch}
                          <span class="mail-relevance">
                            <span>{topMatch.profile_label}</span>
                            <span class="mono">{topMatch.score.toFixed(2)}</span>
                            <span class="mono method">{topMatch.mode}</span>
                          </span>
                        {:else}
                          <span class="mail-relevance unscored">Not TELOS-scored</span>
                        {/if}
                        <span class="mail-data-class mono" data-class={proposal.data_class}>
                          {dataClassLabel(proposal.data_class)}
                        </span>
                        {#if proposal.waiting}
                          <span class="mail-waiting mono">
                            Waiting{proposal.waiting_since ? ` since ${purgeDateLabel(proposal.waiting_since) ?? proposal.waiting_since}` : ""}
                          </span>
                        {/if}
                        {#if proposal.status === "trashed" && purgeDateLabel(proposal.purge_after)}
                          <span class="mail-purge mono">Axon copy retained until {purgeDateLabel(proposal.purge_after)}</span>
                        {/if}
                        {#if proposal.status === "missing"}
                          <span class="mail-missing">No longer available in Gmail. Axon retained its local record.</span>
                        {/if}
                        {#if proposal.gmail_sync_status && proposal.gmail_sync_status !== "synced"}
                          <span class="mail-sync mono" data-status={proposal.gmail_sync_status}>
                            Gmail sync: {proposal.gmail_sync_status}{proposal.gmail_sync_action ? ` · ${proposal.gmail_sync_action}` : ""}
                          </span>
                        {/if}
                      </span>
                      {#if proposal.status !== "proposed"}
                        <span class="status tag mono">{proposal.status}</span>
                      {/if}
                    </a>
                  </div>
                  {#if proposal.gmail_sync_status === "attention"}
                    <div class="mail-job-actions" aria-label="Gmail action recovery">
                      <span>Automatic retries stopped after five attempts.</span>
                      <button class="btn" disabled={mailJobBusy !== null} onclick={() => decideGmailJob(proposal.id, "retry")}>
                        {mailJobBusy === proposal.id ? "Working…" : "Retry"}
                      </button>
                      <button class="btn" disabled={mailJobBusy !== null} onclick={() => decideGmailJob(proposal.id, "cancel")}>Cancel action</button>
                    </div>
                  {/if}
                </li>
              {/each}
              {#if group.items.length === 0}
                <li class="column-empty">No matching mail</li>
              {/if}
            </ul>
          </section>
        {/each}
      </div>
    {/if}
  {/if}
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
    Feed could not be started. See <a href={link("/capabilities")}>Capabilities</a> for details.
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
              <a class="title" href={link(`/feed/${e.id}`)}>
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
            {:else if e.digest_preview}
              <!-- No summary of its own: past the on-device window, so the enrichment drain left
                   it and the digest drain took it through the cloud instead. Showing the digest's
                   opening rather than an empty card, labelled so the two are not confused. -->
              <p class="preview">{e.digest_preview}</p>
              <p class="muted from-digest">from the digest</p>
            {:else if ingested === e.id}
              <p class="muted pending">
                <Icon name="loader" size={12} /> Summary is running — it will appear after the next load.
              </p>
            {/if}
          </li>
{/snippet}

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

  .mail-toolbar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0.5rem;
    margin-bottom: 1rem;
  }

  .mail-toolbar select,
  .mail-search {
    min-height: 2rem;
    border: 1px solid var(--card-border);
    border-radius: var(--radius-md);
    background: var(--card-bg);
    color: var(--text-primary);
    font: inherit;
    font-size: 0.75rem;
  }

  .mail-toolbar select {
    padding: 0.35rem 0.55rem;
  }

  .mail-search {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    flex: 1;
    min-width: min(15rem, 100%);
    padding: 0 0.6rem;
    color: var(--text-tertiary);
  }

  .mail-search input {
    min-width: 0;
    width: 100%;
    border: 0;
    outline: 0;
    background: transparent;
    color: var(--text-primary);
    font: inherit;
    font-size: 0.75rem;
  }

  .method-button.active {
    color: var(--primary);
  }

  .mail-notice {
    margin: -0.4rem 0 0.85rem;
  }

  /* A schedule that stopped collecting has to read differently from one
     reporting a quiet night, or "last run 3 days ago" gets skimmed as normal. */
  .sweep-failing {
    color: var(--warning, var(--text-primary));
  }

  .bulk-bar {
    position: sticky;
    z-index: 4;
    top: 0.5rem;
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0.5rem;
    padding: 0.65rem;
    margin-bottom: 1rem;
  }

  .bulk-bar strong {
    margin-right: 0.25rem;
    font-size: 0.75rem;
  }

  .bulk-category {
    display: flex;
    gap: 0.35rem;
  }

  .bulk-category select {
    border: 1px solid var(--card-border);
    border-radius: var(--radius-md);
    background: var(--card-bg);
    color: var(--text-primary);
    font: inherit;
    font-size: 0.75rem;
  }

  .bulk-confirm {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    flex-basis: 100%;
    padding-top: 0.55rem;
    border-top: 1px solid var(--card-border);
    color: var(--text-secondary);
    font-size: 0.75rem;
  }

  .bulk-confirm span {
    flex: 1;
  }

  .classifier {
    padding: 1rem;
    margin-bottom: 1rem;
  }

  .classifier h2 {
    margin: 0.15rem 0 0.85rem;
    color: var(--text-primary);
    font-size: 0.9rem;
  }

  .classifier dl {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(13rem, 1fr));
    gap: 0.85rem 1.25rem;
    margin: 0;
  }

  .classifier dt {
    color: var(--text-tertiary);
    font-family: var(--font-mono);
    font-size: 0.625rem;
    text-transform: uppercase;
  }

  .classifier dd {
    margin: 0.2rem 0 0;
    color: var(--text-secondary);
    font-size: 0.75rem;
    line-height: 1.45;
  }

  .mail-board {
    display: grid;
    grid-auto-flow: column;
    grid-auto-columns: minmax(18rem, 21rem);
    align-items: start;
    gap: 0.75rem;
    overflow-x: auto;
    padding: 0 0 0.75rem;
    scroll-snap-type: x proximity;
  }

  .mail-proposals {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
    max-height: min(68vh, 48rem);
    overflow-y: auto;
    padding-right: 0.15rem;
  }

  .mail-column {
    min-width: 0;
    padding: 0.65rem;
    border: 1px solid var(--card-border);
    border-radius: var(--radius-lg, 0.75rem);
    background: var(--surface);
    scroll-snap-align: start;
  }

  .mail-column > header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.5rem;
    margin-bottom: 0.55rem;
  }

  .mail-column h2 {
    margin: 0;
  }

  .column-select {
    padding: 0;
    border: 0;
    background: transparent;
    color: var(--text-tertiary);
    font-size: 0.625rem;
    cursor: pointer;
  }

  .column-select:hover:not(:disabled) {
    color: var(--primary);
  }

  .column-empty {
    padding: 1rem 0.5rem;
    color: var(--text-tertiary);
    font-size: 0.6875rem;
    text-align: center;
  }

  .mail-proposal {
    flex: 0 0 auto;
    height: 11.5rem;
    padding: 0;
    overflow: hidden;
  }

  .proposal-row {
    position: relative;
    display: flex;
  }

  .card-select {
    position: absolute;
    top: 0.9rem;
    left: 0.7rem;
    z-index: 1;
    display: flex;
  }

  .card-select input {
    margin: 0;
    accent-color: var(--primary);
  }

  .from-digest {
    font-size: 0.75rem;
    margin-top: 0.15rem;
  }

  .proposal-summary {
    display: flex;
    align-items: flex-start;
    gap: 0.55rem;
    width: 100%;
    min-height: 11.5rem;
    padding: 0.85rem 0.8rem 0.85rem 2.5rem;
    box-sizing: border-box;
    border: 0;
    background: transparent;
    color: inherit;
    font: inherit;
    text-align: left;
    text-decoration: none;
    cursor: pointer;
  }

  .proposal-summary:hover {
    background: var(--surface-hover, rgba(127, 127, 127, 0.08));
  }

  .proposal-copy {
    display: flex;
    flex: 1;
    min-width: 0;
    flex-direction: column;
  }

  .proposal-copy .lead {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .proposal-copy .meta {
    overflow: hidden;
    margin-top: 0.25rem;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .proposal-copy .snippet {
    display: -webkit-box;
    overflow: hidden;
    margin-top: 0.6rem;
    color: var(--text-secondary);
    font-size: 0.75rem;
    line-height: 1.4;
    -webkit-box-orient: vertical;
    line-clamp: 2;
    -webkit-line-clamp: 2;
  }

  .mail-relevance {
    display: flex;
    align-items: center;
    gap: 0.3rem;
    min-width: 0;
    margin-top: 0.4rem;
    color: var(--primary);
    font-size: 0.65rem;
  }

  .mail-relevance > span:first-child {
    overflow: hidden;
    min-width: 0;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .mail-relevance .method,
  .mail-relevance.unscored {
    color: var(--text-tertiary);
  }

  .mail-data-class {
    align-self: flex-start;
    margin-top: 0.4rem;
    padding: 0.16rem 0.35rem;
    border: 1px solid var(--card-border);
    border-radius: 999px;
    background: var(--surface);
    color: var(--text-tertiary);
    font-size: 0.5625rem;
    text-transform: uppercase;
  }

  .mail-data-class[data-class="c0"] {
    color: var(--success);
  }

  /* c2 and c3 both carry the old vault tier's redact_before_persistence +
     cloud=never restrictions (Q27); both keep its warning colour. */
  .mail-data-class[data-class="c2"],
  .mail-data-class[data-class="c3"] {
    color: var(--warning);
  }

  /* Its own colour rather than the data-class pill's grey: a Waiting thread is a
     thing you are owed, and it has to be findable while scanning a long list. */
  .mail-waiting {
    align-self: flex-start;
    margin-top: 0.4rem;
    padding: 0.16rem 0.35rem;
    border: 1px solid var(--accent, var(--card-border));
    border-radius: 999px;
    background: var(--surface);
    color: var(--accent, var(--text-secondary));
    font-size: 0.5625rem;
    text-transform: uppercase;
  }

  .mail-purge {
    display: block;
    margin-top: 0.4rem;
    color: var(--warning);
    font-size: 0.625rem;
  }

  .mail-missing {
    display: block;
    margin-top: 0.4rem;
    color: var(--text-secondary);
    font-size: 0.625rem;
    line-height: 1.4;
  }

  .mail-sync {
    display: block;
    margin-top: 0.35rem;
    color: var(--warning);
    font-size: 0.625rem;
  }

  .mail-sync[data-status="attention"] {
    color: var(--error, #c33);
  }

  .mail-job-actions {
    display: flex;
    align-items: center;
    gap: 0.45rem;
    padding: 0.65rem 0.75rem;
    border-top: 1px solid var(--card-border);
    color: var(--text-secondary);
    font-size: 0.625rem;
  }

  .mail-proposal.has-job .proposal-summary {
    min-height: 8.5rem;
  }

  .mail-job-actions span {
    flex: 1;
  }

  .danger {
    color: var(--error, #c33);
  }

  .status {
    flex-shrink: 0;
    text-transform: uppercase;
  }

  @media (max-width: 42rem) {
    .mail-board {
      grid-auto-columns: minmax(85vw, 1fr);
    }

    .bulk-confirm {
      align-items: stretch;
      flex-direction: column;
    }
  }

  .empty-state {
    padding: 1.5rem 0;
    color: var(--text-secondary);
  }

  .empty-state h2 {
    margin-bottom: 0.35rem;
    color: var(--text-primary);
    font-size: 0.95rem;
  }

  .empty-state p {
    margin: 0;
    font-size: 0.8125rem;
  }

  .sr-only {
    position: absolute;
    width: 1px;
    height: 1px;
    padding: 0;
    margin: -1px;
    overflow: hidden;
    clip: rect(0, 0, 0, 0);
    white-space: nowrap;
    border: 0;
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
