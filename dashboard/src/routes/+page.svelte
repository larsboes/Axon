<script lang="ts">
  import { link } from "$lib/nav";
  import { onMount } from "svelte";
  import Icon from "$lib/Icon.svelte";
  import {
    axonStatus,
    calendar,
    comms,
    macmon,
    panelUrl,
    scouting,
    trips,
    type AxonStatusHealth,
    type MacmonSample,
    type CalendarContext,
    type CalendarEntry,
    type CapabilityView,
    type FeedEntry,
    type FeedStatus,
    type OpportunityStatus,
    type ScoutingOpportunity,
    type ScoutingSource,
    tasks,
    type Task,
    type TriageItem,
    type TripPlan,
  } from "$lib/api";
  import { entryLink } from "$lib/calendar/types";
  import { capabilities } from "$lib/capabilities.svelte";
  import RepoStatusCard from "$lib/RepoStatusCard.svelte";
  import HomeHorizon from "$lib/home/HomeHorizon.svelte";
  import LocationView from "$lib/home/LocationView.svelte";
  import SourcesView from "$lib/home/SourcesView.svelte";

  type Decision =
    | { key: string; kind: "system"; priority: number }
    | { key: string; kind: "feed"; priority: number; entry: FeedEntry }
    | { key: string; kind: "calendar"; priority: number; entry: CalendarEntry }
    | { key: string; kind: "opportunity"; priority: number; opportunity: ScoutingOpportunity }
    | { key: string; kind: "trip"; priority: number; plan: TripPlan }
    | { key: string; kind: "mail"; priority: number; item: TriageItem }
    | { key: string; kind: "task"; priority: number; task: Task };
  type HomeView = "now" | "locations" | "sources";

  const today = new Date();
  const todayKey = localDateKey(today);
  const horizonEndKey = localDateKey(new Date(today.getFullYear(), today.getMonth() + 4, today.getDate()));
  const todayLabel = today.toLocaleDateString("en-GB", {
    weekday: "long",
    day: "numeric",
    month: "long",
  });
  const feedKinds: Record<string, string> = {
    youtube: "YouTube",
    instagram: "Instagram",
    podcast: "Podcast",
    article: "Article",
    mail: "Mail",
    github: "GitHub",
    arxiv: "arXiv",
    reddit: "Reddit",
  };

  let health = $state<AxonStatusHealth | null>(null);
  let macmonSample = $state<MacmonSample | null>(null);
  let macmonErr = $state(false);
  let feedEntries = $state<FeedEntry[]>([]);
  let mailProposals = $state<TriageItem[]>([]);
  let openTasks = $state<Task[]>([]);
  let opportunities = $state<ScoutingOpportunity[]>([]);
  let scoutingSources = $state<ScoutingSource[]>([]);
  let plans = $state<TripPlan[]>([]);
  let calendarEntries = $state<CalendarEntry[]>([]);
  let calendarContexts = $state<CalendarContext[]>([]);
  let unavailable = $state<string[]>([]);
  let loading = $state(true);
  let busy = $state<string | null>(null);
  let busyProject = $state<string | null>(null);
  let actionError = $state<string | null>(null);
  let showAll = $state(false);
  let showReading = $state(false);
  let selectedIndex = $state(0);
  let homeView = $state<HomeView>("now");

  /// How much reading is worth showing before it becomes a scroll. Past this
  /// the Feed page is the better surface, so the list links out instead.
  const READING_PREVIEW = 6;

  const viewHeadings: Record<HomeView, { kicker: string; title: string }> = {
    now: { kicker: "Focus", title: "Up next" },
    locations: { kicker: "Places", title: "By location" },
    sources: { kicker: "Inputs", title: "Sources" },
  };

  const upcomingEntries = $derived(
    calendarEntries
      .filter((entry) => entry.starts_at.slice(0, 10) >= todayKey && entry.commitment !== "possible")
      .sort((a, b) => a.starts_at.localeCompare(b.starts_at)),
  );

  const decisions = $derived.by<Decision[]>(() => {
    const items: Decision[] = [];

    if (health && !health.ok) {
      items.push({ key: "system", kind: "system", priority: 10_000 });
    }

    for (const plan of plans) {
      if (plan.date_end < todayKey || !tripNeedsPlanning(plan)) continue;
      const days = daysUntil(plan.date_start);
      const urgency = days <= 14 ? Math.max(0, 400 - days * 20) : 0;
      items.push({
        key: `trip:${plan.id}`,
        kind: "trip",
        plan,
        priority: 800 + urgency,
      });
    }

    for (const entry of calendarEntries) {
      if (entry.commitment !== "possible" || entry.ends_at.slice(0, 10) < todayKey) continue;
      const days = daysUntil(entry.starts_at);
      const urgency = days >= 0 && days <= 30 ? 240 - days * 5 : 0;
      items.push({
        key: `calendar:${entry.id}`,
        kind: "calendar",
        entry,
        priority: 700 + urgency + (entry.source === "web" ? 50 : 0),
      });
    }

    for (const opportunity of opportunities) {
      if (opportunity.status !== "new" || !opportunityIsPersonalized(opportunity)) continue;
      const days = opportunity.starts_at ? daysUntil(opportunity.starts_at) : 90;
      const urgency = days >= 0 && days <= 30 ? 200 - days * 4 : 0;
      items.push({
        key: `opportunity:${opportunity.id}`,
        kind: "opportunity",
        opportunity,
        priority: 600 + safeOpportunityScore(opportunity) * 100 + urgency
          + calendarRankAdjustment(opportunity)
          + contextRankAdjustment(opportunity),
      });
    }

    /// A task outranks the mail it came from: someone already decided this is
    /// owed, where a proposal is still awaiting that decision. Overdue and
    /// due-soon lift it; an undated task sits at the base of the band.
    for (const task of openTasks) {
      const days = task.due ? daysUntil(task.due) : null;
      const urgency = days === null ? 0 : days < 0 ? 260 : Math.max(0, 240 - days * 8);
      items.push({
        key: `task:${task.id}`,
        kind: "task",
        task,
        priority: 620 + urgency,
      });
    }

    /// Mail sits between opportunities and reading, and only `aktiv` mail gets
    /// here at all. The other six categories are the ones the rules already
    /// decided about — a receipt or a newsletter has no call left to make, and
    /// putting the whole inbox in this list would repeat the mistake the note
    /// below the loop describes, with 25 proposals instead of 53 articles.
    for (const item of mailProposals) {
      if (item.status !== "proposed" || item.stream !== "aktiv") continue;
      const recent = item.internal_date
        ? Date.now() - new Date(item.internal_date).getTime() < 172_800_000
        : false;
      items.push({
        key: `mail:${item.id}`,
        kind: "mail",
        item,
        priority: 550 + (recent ? 30 : 0),
      });
    }

    for (const entry of feedEntries) {
      if (entry.status !== "new") continue;
      const recent = Date.now() - new Date(entry.created_at).getTime() < 86_400_000 ? 40 : 0;
      items.push({
        key: `feed:${entry.id}`,
        kind: "feed",
        entry,
        priority: 500 + (entry.relevance?.score ?? 0) * 100 + recent,
      });
    }

    return items.sort((a, b) => b.priority - a.priority);
  });

  /// Two lists, not one ranked pile. A trip stage and a calendar proposal are
  /// dated commitments — they expire whether or not you look at them. Feed is
  /// optional reading that never expires. Interleaving them by score put 53
  /// articles between three decisions that actually needed a call.
  const commitments = $derived(decisions.filter((decision) => decision.kind !== "feed"));
  const reading = $derived(
    decisions.filter((decision): decision is Extract<Decision, { kind: "feed" }> =>
      decision.kind === "feed",
    ),
  );

  /// Reading opens collapsed, so J/K/Enter walk the commitments first and only
  /// reach the articles once they are on screen.
  const visibleReading = $derived(showAll ? reading : reading.slice(0, READING_PREVIEW));
  const visibleDecisions = $derived<Decision[]>(
    showReading ? [...commitments, ...visibleReading] : commitments,
  );

  const readingToday = $derived(
    reading.filter(
      (decision) => Date.now() - new Date(decision.entry.created_at).getTime() < 86_400_000,
    ).length,
  );

  /// What the page is actually for, in one sentence: the nearest thing that
  /// expires. A count of the backlog ("57 open items") reads as debt and names
  /// nothing you can act on — and 53 of those 57 were unread articles.
  const brief = $derived.by(() => {
    if (loading) return "Bringing together saved information, opportunities, and travel plans.";
    const next = commitments[0];
    if (!next) {
      return reading.length === 0
        ? "There are no open decisions right now. You can start something new."
        : `Nothing is waiting on a decision. ${countLabel(reading.length, "unread item")} below.`;
    }
    if (next.kind === "system") return "A service that should be running is not responding.";
    if (next.kind === "trip") {
      const where = next.plan.destinations.map((place) => place.name).join(" → ")
        || next.plan.title;
      return `${sentenceCase(whenLabel(next.plan.date_start))}: ${where}, ${tripGap(next.plan)}.`;
    }
    if (next.kind === "calendar") return `${next.entry.title} on ${dateLabel(next.entry.starts_at)} is still undecided.`;
    if (next.kind === "task") {
      return next.task.due
        ? `${next.task.title} is due ${dateLabel(next.task.due)}.`
        : `${next.task.title} is still open.`;
    }
    if (next.kind === "mail") {
      return `${next.item.subject ?? "A mail"} from ${next.item.from_addr ?? "an unknown sender"} is waiting for a call.`;
    }
    return `${next.opportunity.title} is waiting for a yes or no.`;
  });

  function countLabel(count: number, noun: string): string {
    return `${count} ${noun}${count === 1 ? "" : "s"}`;
  }

  function whenLabel(day: string): string {
    const days = daysUntil(day);
    if (days < 0) return "already under way";
    if (days === 0) return "today";
    if (days === 1) return "tomorrow";
    return `in ${days} days`;
  }

  function sentenceCase(value: string): string {
    return value.charAt(0).toLocaleUpperCase("en-GB") + value.slice(1);
  }

  $effect(() => {
    if (selectedIndex >= visibleDecisions.length) {
      selectedIndex = Math.max(0, visibleDecisions.length - 1);
    }
  });

  onMount(() => {
    const stop = capabilities.subscribe();
    void loadHome();

    // One-shot macmon sample for the sidebar compact card; no aggressive polling
    // since /systems is the real live dashboard. Refresh every 30s.
    const pollMac = () => {
      macmon.json().then((d) => { macmonSample = d; macmonErr = false; }).catch(() => { macmonErr = true; });
    };
    pollMac();
    const macTimer = setInterval(pollMac, 30_000);

    return () => {
      clearInterval(macTimer);
      stop();
    };
  });

  async function loadHome(): Promise<void> {
    loading = true;
    await capabilities.refresh();

    const [healthResult, feedResult, taskResult, scoutingResult, tripResult, calendarResult] = await Promise.allSettled([
      axonStatus.health(),
      // Both come from comms, so they share one capability gate and one
      // failure path — a reachable comms that returns no mail is a different
      // fact from an unreachable comms, and merging them would hide it.
      readCapability("comms", async () => {
        const [entries, proposals] = await Promise.all([
          comms.feed({ days: 30 }),
          comms.triage("proposed"),
        ]);
        return { entries, proposals };
      }),
      readCapability("tasks", () => tasks.list("open")),
      readCapability("scouting", async () => {
        const [opportunityResult, sourceResult] = await Promise.all([
          scouting.opportunities(false),
          scouting.sources(),
        ]);
        return { opportunityResult, sourceResult };
      }),
      readCapability("trips", () => trips.list()),
      readCapability("calendar", async () => {
        const [entries, contexts] = await Promise.all([
          calendar.entries.list(todayKey, horizonEndKey),
          calendar.contexts.list(todayKey, horizonEndKey),
        ]);
        return { entries, contexts };
      }),
    ]);

    const missing: string[] = [];
    if (healthResult.status === "fulfilled") health = healthResult.value;
    else missing.push("System status");

    if (feedResult.status === "fulfilled") {
      feedEntries = feedResult.value.entries.filter((entry) => entry.status === "new");
      mailProposals = feedResult.value.proposals;
    } else {
      missing.push("Feed");
    }

    if (taskResult.status === "fulfilled") {
      openTasks = taskResult.value;
    } else {
      missing.push("Tasks");
    }

    if (scoutingResult.status === "fulfilled") {
      opportunities = scoutingResult.value.opportunityResult.opportunities;
      scoutingSources = scoutingResult.value.sourceResult.sources;
    } else {
      missing.push("Scouting");
    }

    if (tripResult.status === "fulfilled") {
      plans = tripResult.value;
    } else {
      missing.push("Travel");
    }

    if (calendarResult.status === "fulfilled") {
      calendarEntries = calendarResult.value.entries;
      calendarContexts = calendarResult.value.contexts;
    } else {
      missing.push("Calendar");
    }

    unavailable = missing;
    loading = false;
  }

  async function readCapability<T>(name: string, read: () => Promise<T>): Promise<T> {
    const capability = capabilities.byName(name);
    if (capability && capability.up !== true) {
      await axonStatus.start(name);
    }
    return read();
  }

  async function setFeedStatus(id: string, status: FeedStatus): Promise<void> {
    if (busy) return;
    busy = `feed:${id}`;
    actionError = null;
    try {
      await comms.setStatus(id, status);
      feedEntries = feedEntries.filter((entry) => entry.id !== id);
    } catch (caught) {
      actionError = message(caught);
    } finally {
      busy = null;
    }
  }

  async function completeTask(id: string): Promise<void> {
    if (busy) return;
    busy = `task:${id}`;
    actionError = null;
    try {
      await tasks.patch(id, { status: "done" });
      openTasks = openTasks.filter((task) => task.id !== id);
    } catch (caught) {
      actionError = message(caught);
    } finally {
      busy = null;
    }
  }

  function mailLink(item: TriageItem): string {
    return link(`/feed/${encodeURIComponent(item.id)}?source=mail`);
  }

  /// Local only. Dismissing drops the proposal from this list and changes
  /// nothing in Gmail — the archive and trash actions live on the entry page,
  /// behind their own confirmation, because they leave Axon.
  async function dismissMail(id: string): Promise<void> {
    if (busy) return;
    busy = `mail:${id}`;
    actionError = null;
    try {
      await comms.setTriageStatus(id, "dismissed");
      mailProposals = mailProposals.filter((item) => item.id !== id);
    } catch (caught) {
      actionError = message(caught);
    } finally {
      busy = null;
    }
  }

  async function setOpportunityStatus(
    id: string,
    status: OpportunityStatus,
  ): Promise<void> {
    if (busy) return;
    busy = `opportunity:${id}`;
    actionError = null;
    try {
      await scouting.setStatus(id, status);
      opportunities = opportunities.filter((opportunity) => opportunity.id !== id);
    } catch (caught) {
      actionError = message(caught);
    } finally {
      busy = null;
    }
  }

  async function planCalendarEntry(entry: CalendarEntry): Promise<void> {
    if (busy) return;
    busy = `calendar:${entry.id}`;
    actionError = null;
    try {
      const updated = await calendar.entries.update(entry.id, { commitment: "planned" });
      calendarEntries = calendarEntries.map((current) => current.id === entry.id ? updated : current);
    } catch (caught) {
      actionError = message(caught);
    } finally {
      busy = null;
    }
  }

  async function startProject(project: CapabilityView): Promise<void> {
    if (busyProject) return;
    busyProject = project.name;
    actionError = null;
    try {
      await axonStatus.start(project.name);
      await capabilities.refresh();
    } catch (caught) {
      actionError = message(caught);
    } finally {
      busyProject = null;
    }
  }

  function handleKeyboard(event: KeyboardEvent): void {
    const target = event.target instanceof Element ? event.target : null;
    if (
      event.metaKey ||
      event.ctrlKey ||
      event.altKey ||
      target?.closest("a, button, input, select, textarea")
    ) {
      return;
    }

    if (event.key.toLowerCase() === "j" && visibleDecisions.length > 0) {
      event.preventDefault();
      selectedIndex = Math.min(selectedIndex + 1, visibleDecisions.length - 1);
    } else if (event.key.toLowerCase() === "k" && visibleDecisions.length > 0) {
      event.preventDefault();
      selectedIndex = Math.max(selectedIndex - 1, 0);
    } else if (event.key === "Enter" && visibleDecisions[selectedIndex]) {
      event.preventDefault();
      openDecision(visibleDecisions[selectedIndex]);
    }
  }

  function openDecision(decision: Decision): void {
    if (decision.kind === "feed") {
      location.href = `/feed/${encodeURIComponent(decision.entry.id)}`;
    } else if (decision.kind === "task") {
      // A task's own page does not exist yet, so Enter goes to the mail it came
      // from — the thing you actually need to read to act on it.
      if (decision.task.source_url) location.href = decision.task.source_url;
    } else if (decision.kind === "mail") {
      // The entry route resolves an item from its source alone, so mail opens
      // the same reader feed does — with the Gmail actions its extension adds.
      location.href = `/feed/${encodeURIComponent(decision.item.id)}?source=mail`;
    } else if (decision.kind === "calendar") {
      location.href = "/calendar";
    } else if (decision.kind === "opportunity") {
      window.open(decision.opportunity.url, "_blank", "noopener,noreferrer");
    } else if (decision.kind === "trip") {
      location.href = "/travel";
    } else {
      location.href = "/capabilities";
    }
  }

  function localDateKey(date: Date): string {
    return [
      date.getFullYear(),
      String(date.getMonth() + 1).padStart(2, "0"),
      String(date.getDate()).padStart(2, "0"),
    ].join("-");
  }

  function daysUntil(value: string): number {
    const date = new Date(`${value.slice(0, 10)}T12:00:00`);
    if (Number.isNaN(date.getTime())) return 365;
    const start = new Date(today);
    start.setHours(12, 0, 0, 0);
    return Math.ceil((date.getTime() - start.getTime()) / 86_400_000);
  }

  function safeOpportunityScore(opportunity: ScoutingOpportunity): number {
    return staleOpportunityScore(opportunity) ? 0 : opportunity.score;
  }

  function staleOpportunityScore(opportunity: ScoutingOpportunity): boolean {
    return opportunity.opportunity_type === "event"
      && /scholarship/i.test(opportunity.matched_focus);
  }

  function opportunityIsPersonalized(opportunity: ScoutingOpportunity): boolean {
    const lastDay = (opportunity.ends_at || opportunity.starts_at || "").slice(0, 10);
    if (lastDay && lastDay < todayKey) return false;
    if (staleOpportunityScore(opportunity)) return false;
    if (calendarEntries.some((entry) => sameOpportunity(entry, opportunity))) return false;

    const linkedToPlan = calendarRankAdjustment(opportunity) > 0
      || contextRankAdjustment(opportunity) > 0;
    if (linkedToPlan) return true;

    const focus = opportunity.matched_focus.trim();
    const genericFocus = /^(events?|scholarships?) profile$/i.test(focus);
    return !genericFocus && focus.length > 0 && safeOpportunityScore(opportunity) >= 0.22;
  }

  function sameOpportunity(entry: CalendarEntry, opportunity: ScoutingOpportunity): boolean {
    if (!entry.payload || typeof entry.payload !== "object") return false;
    const payload = entry.payload as { opportunity_id?: unknown; url?: unknown };
    return payload.opportunity_id === opportunity.id || payload.url === opportunity.url;
  }

  function calendarRankAdjustment(opportunity: ScoutingOpportunity): number {
    if (!opportunity.starts_at) return 0;
    const day = opportunity.starts_at.slice(0, 10);
    let adjustment = 0;
    for (const entry of calendarEntries) {
      if (sameOpportunity(entry, opportunity)) continue;
      if (entry.commitment === "possible") continue;
      const entryDay = entry.starts_at.slice(0, 10);
      const distance = Math.abs(daysBetween(day, entryDay));
      const city = opportunity.city.trim().toLocaleLowerCase("en-GB");
      const samePlace = city.length > 0
        && (entry.location ?? "").toLocaleLowerCase("en-GB").includes(city);
      if (samePlace && distance <= 3) adjustment += 65;
      if (entry.commitment === "committed" && entryDay === day) adjustment -= 220;
    }
    return Math.max(-220, Math.min(90, adjustment));
  }

  function contextRankAdjustment(opportunity: ScoutingOpportunity): number {
    if (!opportunity.starts_at) return 0;
    const day = opportunity.starts_at.slice(0, 10);
    const city = opportunity.city.trim().toLocaleLowerCase("en-GB");
    let adjustment = 0;
    for (const context of calendarContexts) {
      if (day < context.valid_from || day > context.valid_until) continue;
      const text = `${context.title} ${context.details}`.toLocaleLowerCase("en-GB");
      const nearPlanningDeadline = context.kind !== "planning_gap"
        || Math.abs(daysBetween(day, context.valid_until)) <= 3;
      if (city && text.includes(city) && nearPlanningDeadline) adjustment += 45;
      if (context.kind === "uncertainty") adjustment -= 15;
    }
    return adjustment;
  }

  function daysBetween(a: string, b: string): number {
    const first = new Date(`${a.slice(0, 10)}T12:00:00`);
    const second = new Date(`${b.slice(0, 10)}T12:00:00`);
    return Math.round((first.getTime() - second.getTime()) / 86_400_000);
  }

  function relativeDate(value: string): string {
    const date = new Date(value);
    if (Number.isNaN(date.getTime())) return "";
    const diff = Math.round((today.getTime() - date.getTime()) / 86_400_000);
    if (diff <= 0) return "today";
    if (diff === 1) return "yesterday";
    if (diff < 7) return `${diff} days ago`;
    return date.toLocaleDateString("en-GB", { day: "numeric", month: "short" });
  }

  function dateLabel(value: string): string {
    const date = new Date(`${value.slice(0, 10)}T12:00:00`);
    if (Number.isNaN(date.getTime())) return value;
    return date.toLocaleDateString("en-GB", { day: "numeric", month: "short" });
  }

  function calendarContext(entry: CalendarEntry): string {
    return [
      dateLabel(entry.starts_at),
      entry.location,
      entry.source === "web" ? "added deliberately" : "",
    ].filter(Boolean).join(" · ");
  }

  function tripNeedsPlanning(plan: TripPlan): boolean {
    return (
      plan.stages.length === 0 ||
      plan.stages.some((stage) => stage.status === "planning" || stage.status === "option_selected")
    );
  }

  /// Names the leg that is actually open, not how many are.
  ///
  /// Counting produced the byte-identical sentence "One stage still needs a
  /// decision." under every trip on the page, three in a row, which told the
  /// operator nothing and read as boilerplate. The leg and its state are both
  /// already in the data.
  function tripGap(plan: TripPlan): string {
    if (plan.stages.length === 0) return "no route planned yet";
    const open = plan.stages.filter(
      (stage) => stage.status === "planning" || stage.status === "option_selected",
    );
    const first = open[0];
    if (!first) return "everything booked";
    // `option_selected` is a real distinction: a connection is chosen, so the
    // remaining act is booking it, not deciding it.
    const state = first.status === "option_selected" ? "chosen but not booked" : "no connection chosen";
    const rest = open.length > 1 ? `, +${open.length - 1} more` : "";
    // The leg only earns its words on a multi-stage trip. On a single-stage one
    // it just repeats the destination the row already shows above it.
    if (plan.stages.length < 2) return `${state}${rest}`;
    const leg = [first.origin?.name, first.destination?.name].filter(Boolean).join(" → ");
    return leg ? `${leg}, ${state}${rest}` : `${state}${rest}`;
  }

  function opportunityContext(opportunity: ScoutingOpportunity): string {
    return [
      opportunity.starts_at ? dateLabel(opportunity.starts_at) : "",
      opportunity.city,
      opportunityRankHint(opportunity),
    ]
      .filter(Boolean)
      .join(" · ");
  }

  function opportunityRankHint(opportunity: ScoutingOpportunity): string {
    if (staleOpportunityScore(opportunity)) return "score out of date";
    const calendarAdjustment = calendarRankAdjustment(opportunity);
    const contextAdjustment = contextRankAdjustment(opportunity);
    if (calendarAdjustment < 0) return "conflicts with a committed event";
    if (calendarAdjustment > 0) return "fits a planned location";
    if (contextAdjustment > 0) return "fits the current planning context";
    return opportunity.matched_focus ? `matches ${opportunity.matched_focus}` : "";
  }

  function cleanRationale(value: string): string {
    if (!value || /(cosine=|hash-fallback|matched focus)/i.test(value)) return "";
    return value
      .split("\n")
      .filter((line) => !line.includes("vault link:"))
      .join(" ")
      .trim();
  }

  function message(caught: unknown): string {
    return caught instanceof Error ? caught.message : String(caught);
  }

  function projectTitle(project: CapabilityView): string {
    if (project.name === "server") return "Home-Server & Local AI";
    return project.name.charAt(0).toUpperCase() + project.name.slice(1);
  }
</script>

<svelte:window onkeydown={handleKeyboard} />

<div class="home">
  <header class="briefing">
    <div>
      <p class="date">{todayLabel}</p>
      <h1>
        {#if loading}
          Axon is organising the day.
        {:else if commitments.length === 0}
          Nothing to decide.
        {:else}
          Here is what to do next.
        {/if}
      </h1>
      <p class="brief">{brief}</p>
    </div>
    <a class="library-link" href={link("/feed/library")}>
      Library <Icon name="arrow-right" size={13} />
    </a>
  </header>

  {#if actionError}
    <div class="notice error">
      <Icon name="alert" size={15} />
      <span>{actionError}</span>
      <button type="button" aria-label="Dismiss error" onclick={() => (actionError = null)}>
        <Icon name="close" size={13} />
      </button>
    </div>
  {/if}

  <div class="workspace">
    <section class="next">
      {#if homeView === "now"}
        <HomeHorizon contexts={calendarContexts} entries={upcomingEntries} />
      {/if}

      <!-- One header for the whole main column. The view switcher lives here
           rather than above the page, because these are three readings of the
           same column, not three modes of the page. -->
      <div class="section-head">
        <div>
          <span class="section-kicker">{viewHeadings[homeView].kicker}</span>
          <h2>{viewHeadings[homeView].title}</h2>
        </div>
        <nav class="home-views" aria-label="Home view">
          <button class:active={homeView === "now"} onclick={() => (homeView = "now")}>Now</button>
          <button class:active={homeView === "locations"} onclick={() => (homeView = "locations")}>
            Locations
          </button>
          <button class:active={homeView === "sources"} onclick={() => (homeView = "sources")}>
            Sources
          </button>
        </nav>
      </div>

      {#if homeView === "now"}
      {#if visibleDecisions.length > 1}
        <p class="key-hint"><kbd>J</kbd><kbd>K</kbd> select · <kbd>Enter</kbd> open</p>
      {/if}

      <div class="queue" aria-busy={loading}>
        {#if loading}
          <p class="queue-state"><Icon name="loader" size={14} /> Reading current work…</p>
        {:else if commitments.length === 0}
          <div class="queue-state complete">
            <span class="complete-mark"><Icon name="check" size={18} /></span>
            <span>
              <strong>Nothing is waiting on a decision.</strong>
              Unfinished travel stages, undecided calendar events, and services that stopped appear here.
            </span>
          </div>
        {:else}
          {#each commitments as decision, index (decision.key)}
            {@render decisionRow(decision, index)}
          {/each}
        {/if}
      </div>

      {#snippet decisionRow(decision: Decision, index: number)}
            <article class="decision" class:selected={selectedIndex === index}>
              {#if decision.kind === "system"}
                <div class="decision-mark warning"><Icon name="alert" size={16} /></div>
                <div class="decision-copy">
                  <span class="kind">System</span>
                  <a class="decision-title" href={link("/capabilities")}>Check autostart</a>
                  <p>At least one service that should be running is not responding.</p>
                </div>
                <div class="decision-actions">
                  <a class="btn action-primary" href={link("/capabilities")}>
                    Check <Icon name="arrow-right" size={13} />
                  </a>
                </div>
              {:else if decision.kind === "feed"}
                <div class="decision-mark"><Icon name="feed" size={16} /></div>
                <div class="decision-copy">
                  <span class="kind">
                    {feedKinds[decision.entry.kind] ?? decision.entry.kind}
                    · {relativeDate(decision.entry.created_at)}
                    {#if decision.entry.relevance}
                      · matches {decision.entry.relevance.profile_label}
                    {/if}
                  </span>
                  <a class="decision-title" href={link(`/feed/${encodeURIComponent(decision.entry.id)}`)}>
                    {decision.entry.title ?? decision.entry.url}
                  </a>
                  {#if decision.entry.summary || cleanRationale(decision.entry.relevance?.rationale ?? "")}
                    <p>
                      {decision.entry.summary ??
                        cleanRationale(decision.entry.relevance?.rationale ?? "")}
                    </p>
                  {/if}
                </div>
                <div class="decision-actions">
                  <a class="btn" href={link(`/feed/${encodeURIComponent(decision.entry.id)}`)}>Read</a>
                  <button
                    class="btn action-primary"
                    type="button"
                    disabled={busy === decision.key}
                    onclick={() => void setFeedStatus(decision.entry.id, "keeper")}
                  >
                    {#if busy === decision.key}<Icon name="loader" size={13} />{:else}Keep{/if}
                  </button>
                  <button
                    class="btn icon-action"
                    type="button"
                    disabled={busy === decision.key}
                    aria-label="Dismiss feed entry"
                    title="Dismiss"
                    onclick={() => void setFeedStatus(decision.entry.id, "dismissed")}
                  >
                    <Icon name="close" size={13} />
                  </button>
                </div>
              {:else if decision.kind === "task"}
                <div class="decision-mark"><Icon name="check" size={16} /></div>
                <div class="decision-copy">
                  <span class="kind">
                    Task
                    {#if decision.task.due}
                      · due {dateLabel(decision.task.due)}
                      {#if daysUntil(decision.task.due) < 0}
                        · <span class="private-mark">overdue</span>
                      {/if}
                    {/if}
                    {#if decision.task.source_capability}
                      · from {decision.task.source_capability}
                    {/if}
                  </span>
                  {#if decision.task.source_url}
                    <a class="decision-title" href={decision.task.source_url}>
                      {decision.task.title}
                    </a>
                  {:else}
                    <span class="decision-title">{decision.task.title}</span>
                  {/if}
                  {#if decision.task.note}<p>{decision.task.note}</p>{/if}
                </div>
                <div class="decision-actions">
                  {#if decision.task.source_url}
                    <a class="btn" href={decision.task.source_url}>Source</a>
                  {/if}
                  <button
                    class="btn action-primary"
                    type="button"
                    disabled={busy === decision.key}
                    onclick={() => void completeTask(decision.task.id)}
                  >
                    {#if busy === decision.key}<Icon name="loader" size={13} />{:else}Done{/if}
                  </button>
                </div>
              {:else if decision.kind === "mail"}
                <div class="decision-mark"><Icon name="mail" size={16} /></div>
                <div class="decision-copy">
                  <span class="kind">
                    Mail · {decision.item.from_addr ?? "unknown sender"}
                    {#if decision.item.internal_date}
                      · {relativeDate(decision.item.internal_date)}
                    {/if}
                    {#if decision.item.data_class === "vault"}
                      · <span class="private-mark">Private, redacted</span>
                    {/if}
                  </span>
                  <a class="decision-title" href={mailLink(decision.item)}>
                    {decision.item.subject ?? "(no subject)"}
                  </a>
                  {#if decision.item.snippet}<p>{decision.item.snippet}</p>{/if}
                </div>
                <div class="decision-actions">
                  <a class="btn" href={mailLink(decision.item)}>Open</a>
                  <button
                    class="btn icon-action"
                    type="button"
                    disabled={busy === decision.key}
                    aria-label="Dismiss mail proposal"
                    title="Dismiss"
                    onclick={() => void dismissMail(decision.item.id)}
                  >
                    {#if busy === decision.key}
                      <Icon name="loader" size={13} />
                    {:else}
                      <Icon name="close" size={13} />
                    {/if}
                  </button>
                </div>
              {:else if decision.kind === "calendar"}
                <div class="decision-mark nightlife"><Icon name="ticket" size={16} /></div>
                <div class="decision-copy">
                  <span class="kind">Calendar opportunity · {calendarContext(decision.entry)}</span>
                  <a class="decision-title" href={entryLink(decision.entry)}>{decision.entry.title}</a>
                  {#if decision.entry.notes}<p>{decision.entry.notes}</p>{/if}
                </div>
                <div class="decision-actions">
                  <a class="btn" href={entryLink(decision.entry)}>Calendar</a>
                  <button
                    class="btn action-primary"
                    type="button"
                    disabled={busy === decision.key}
                    onclick={() => void planCalendarEntry(decision.entry)}
                  >
                    {#if busy === decision.key}<Icon name="loader" size={13} />{:else}Plan{/if}
                  </button>
                </div>
              {:else if decision.kind === "opportunity"}
                <div class="decision-mark"><Icon name="compass" size={16} /></div>
                <div class="decision-copy">
                  <span class="kind">Opportunity · {opportunityContext(decision.opportunity)}</span>
                  <a
                    class="decision-title"
                    href={decision.opportunity.url}
                    target="_blank"
                    rel="noreferrer"
                  >
                    {decision.opportunity.title}
                  </a>
                  {#if cleanRationale(decision.opportunity.rationale)}
                    <p>{cleanRationale(decision.opportunity.rationale)}</p>
                  {/if}
                </div>
                <div class="decision-actions">
                  <a class="btn" href={decision.opportunity.url} target="_blank" rel="noreferrer">
                    Open
                  </a>
                  <button
                    class="btn action-primary"
                    type="button"
                    disabled={busy === decision.key}
                    onclick={() => void setOpportunityStatus(decision.opportunity.id, "saved")}
                  >
                    {#if busy === decision.key}<Icon name="loader" size={13} />{:else}Save{/if}
                  </button>
                  <button
                    class="btn icon-action"
                    type="button"
                    disabled={busy === decision.key}
                    aria-label="Dismiss opportunity"
                    title="Dismiss"
                    onclick={() => void setOpportunityStatus(decision.opportunity.id, "dismissed")}
                  >
                    <Icon name="close" size={13} />
                  </button>
                </div>
              {:else}
                <div class="decision-mark"><Icon name="map-pin" size={16} /></div>
                <div class="decision-copy">
                  <span class="kind">
                    Travel · {dateLabel(decision.plan.date_start)}
                    {#if decision.plan.destinations[0]}
                      · {decision.plan.destinations.map((place) => place.name).join(" → ")}
                    {/if}
                  </span>
                  <a class="decision-title" href={link("/travel")}>{decision.plan.title}</a>
                  <p>{tripGap(decision.plan)}</p>
                </div>
                <div class="decision-actions">
                  <a class="btn action-primary" href={link("/travel")}>
                    Continue planning <Icon name="arrow-right" size={13} />
                  </a>
                </div>
              {/if}
            </article>
      {/snippet}

      <!-- Reading is the other 93% of what used to be one queue, and none of it
           expires. It gets a count and a disclosure, not a rank. -->
      {#if !loading && reading.length > 0}
        <div class="reading">
          <button
            class="reading-toggle"
            type="button"
            aria-expanded={showReading}
            onclick={() => (showReading = !showReading)}
          >
            <Icon name="feed" size={13} />
            <span>
              <strong>{countLabel(reading.length, "unread item")}</strong>
              {#if readingToday > 0}<small>{readingToday} today</small>{/if}
            </span>
            <em>{showReading ? "Hide" : "Read"}</em>
          </button>

          {#if showReading}
            <div class="queue">
              {#each visibleReading as decision, index (decision.key)}
                {@render decisionRow(decision, commitments.length + index)}
              {/each}
            </div>
            {#if reading.length > READING_PREVIEW}
              <button class="show-all" type="button" onclick={() => (showAll = !showAll)}>
                {showAll
                  ? `Show ${READING_PREVIEW} at a time`
                  : `Show ${reading.length - READING_PREVIEW} more`}
                <Icon name={showAll ? "close" : "plus"} size={12} />
              </button>
            {/if}
          {/if}
        </div>
      {/if}

      {#if unavailable.length > 0}
        <p class="unavailable">
          <Icon name="wifi-off" size={12} />
          Unavailable: {unavailable.join(", ")}
          <button type="button" onclick={() => void loadHome()}>Try again</button>
        </p>
      {/if}
      {:else if homeView === "locations"}
        <LocationView
          entries={calendarEntries}
          opportunities={opportunities.filter((opportunity) => opportunity.status === "new")}
          plans={plans.filter((plan) => plan.status !== "archived")}
        />
      {:else}
        <SourcesView
          {feedEntries}
          {opportunities}
          {scoutingSources}
          {calendarEntries}
          contexts={calendarContexts}
          {plans}
        />
      {/if}
    </section>

    <aside>
      <section class="side-section">
        <div class="section-head compact">
          <div>
            <span class="section-kicker">Start</span>
            <h2>Quick actions</h2>
          </div>
        </div>
        <nav class="quick-list" aria-label="Quick actions">
          <a href={link("/feed")}>
            <Icon name="plus" size={15} />
            <span><strong>Add a link</strong><small>Article, video, or repository</small></span>
            <Icon name="arrow-right" size={13} />
          </a>
          <a href={link("/travel")}>
            <Icon name="map-pin" size={15} />
            <span><strong>Plan travel</strong><small>Places, connections, and dates</small></span>
            <Icon name="arrow-right" size={13} />
          </a>
          <a href={link("/feed?view=discover")}>
            <Icon name="compass" size={15} />
            <span><strong>Scan sources</strong><small>Look deliberately for new opportunities</small></span>
            <Icon name="arrow-right" size={13} />
          </a>
        </nav>
      </section>

      {#if capabilities.panels.length > 0}
        <section class="side-section continue">
          <div class="section-head compact">
            <div>
              <span class="section-kicker">Projects</span>
              <h2>Continue working</h2>
            </div>
            <a class="small-link" href={link("/projects")}>All</a>
          </div>
          <ul>
            {#each capabilities.panels as project (project.name)}
              <li>
                <span class="project-mark">
                  <Icon name={project.name === "server" ? "server" : "graduation"} size={15} />
                </span>
                <span class="project-copy">
                  <strong>{projectTitle(project)}</strong>
                  <small>{project.up === true ? "running" : "starts on demand"}</small>
                </span>
                {#if project.up}
                  <a
                    class="btn icon-action"
                    href={panelUrl(project)}
                    target="_blank"
                    rel="noreferrer"
                    aria-label={`Open ${projectTitle(project)}`}
                    title="Open"
                  >
                    <Icon name="external" size={13} />
                  </a>
                {:else}
                  <button
                    class="btn project-start"
                    type="button"
                    disabled={busyProject === project.name}
                    onclick={() => void startProject(project)}
                  >
                    {#if busyProject === project.name}
                      <Icon name="loader" size={13} />
                    {:else}
                      Start
                    {/if}
                  </button>
                {/if}
              </li>
            {/each}
          </ul>
        </section>
      {/if}

      <!-- Machine status, not work. It stays one line until asked: health,
           temperature and memory are things you check, not things you do, and
           three sections of them outweighed the queue they sat beside. A
           <details> keeps the disclosure in CSS with no state to track. -->
      <details class="status">
        <summary>
          <span class="status-dot" class:ok={health?.ok} class:problem={health !== null && !health.ok}></span>
          <span class="status-line">
            {health === null ? "Status unknown" : health.ok ? "Systems healthy" : "Needs attention"}
            {#if macmonSample}
              <span class="mono">
                · {macmonSample.temp.cpu_temp_avg.toFixed(0)}°
                · {(macmonSample.memory.ram_usage / 1073741824).toFixed(1)} GB
              </span>
            {/if}
          </span>
          <Icon name="chevron" size={12} />
        </summary>

        <div class="status-body">
          {#if macmonErr}
            <p class="mc-offline">
              <Icon name="alert" size={12} />
              macmon is off — <a href={link("/systems")}>Details</a>
            </p>
          {:else if macmonSample}
            <div class="macmon-compact">
              <div class="mc-temps">
                <span class="mc-temp" class:warm={macmonSample.temp.cpu_temp_avg >= 60} class:hot={macmonSample.temp.cpu_temp_avg >= 80}>
                  {macmonSample.temp.cpu_temp_avg.toFixed(0)}° CPU
                </span>
                <span class="mc-temp">
                  {macmonSample.temp.gpu_temp_avg.toFixed(0)}° GPU
                </span>
                <span class="mc-power">{macmonSample.all_power.toFixed(1)} W</span>
              </div>
              <div class="mc-mem">
                <span class="mc-mem-label">RAM</span>
                <div class="mc-bar">
                  <div class="mc-fill" style="width:{(macmonSample.memory.ram_usage / macmonSample.memory.ram_total * 100).toFixed(0)}%"></div>
                </div>
                <span class="mc-mem-num mono">{(macmonSample.memory.ram_usage / 1073741824).toFixed(1)} GB</span>
              </div>
              <a class="mc-detail" href={link("/systems")}>Details <Icon name="arrow-right" size={11} /></a>
            </div>
          {/if}

          <RepoStatusCard />

          <a class="capabilities-link" href={link("/capabilities")}>
            Capabilities <Icon name="arrow-right" size={12} />
          </a>
        </div>
      </details>
    </aside>
  </div>
</div>

<style>
  .home {
    animation: fade-up 0.2s ease-out both;
  }

  .briefing {
    display: flex;
    align-items: end;
    justify-content: space-between;
    gap: 2rem;
    padding: 0.25rem 0 1.35rem;
    border-bottom: 1px solid var(--card-border);
  }

  .date,
  .section-kicker {
    margin: 0 0 0.35rem;
    color: var(--primary);
    font-size: 0.6875rem;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.08em;
  }

  h1 {
    max-width: 48rem;
    margin: 0;
    font-size: clamp(1.65rem, 3vw, 2.6rem);
    font-weight: 620;
    line-height: 1.08;
    letter-spacing: -0.035em;
  }

  .brief {
    max-width: 42rem;
    margin: 0.45rem 0 0;
    color: var(--text-secondary);
    font-size: 0.875rem;
  }

  .library-link,
  .small-link {
    display: inline-flex;
    align-items: center;
    gap: 0.35rem;
    color: var(--text-secondary);
    font-size: 0.75rem;
    font-weight: 600;
    white-space: nowrap;
  }

  .library-link:hover,
  .small-link:hover {
    color: var(--primary);
  }

  .notice {
    display: flex;
    align-items: center;
    gap: 0.55rem;
    margin-top: 1rem;
    padding: 0.7rem 0.85rem;
    border: 1px solid var(--card-border);
    border-radius: var(--radius-md);
    font-size: 0.75rem;
  }

  .notice.error {
    color: var(--danger);
    border-color: var(--danger);
    background: var(--danger-soft);
  }

  .notice span {
    flex: 1;
  }

  .notice button {
    display: grid;
    place-items: center;
    padding: 0.2rem;
    border: 0;
    background: transparent;
    color: inherit;
    cursor: pointer;
  }

  /* Three readings of one column, not three modes of the page — so text links
     that sit beside the heading, rather than a filled control above it that
     implied the page had three top-level states. */
  .home-views {
    display: inline-flex;
    gap: 0.85rem;
  }

  .home-views button {
    padding: 0 0 0.2rem;
    border: 0;
    border-bottom: 1.5px solid transparent;
    background: transparent;
    color: var(--text-tertiary);
    font: 600 0.7rem var(--font-sans);
    cursor: pointer;
  }

  .home-views button:hover {
    color: var(--text-secondary);
  }

  .home-views button.active {
    border-bottom-color: var(--primary);
    color: var(--text-primary);
  }

  .workspace {
    display: grid;
    gap: clamp(2rem, 4vw, 4rem);
    padding-top: 1.2rem;
  }

  .section-head {
    display: flex;
    align-items: end;
    justify-content: space-between;
    gap: 1rem;
    margin-bottom: 0.8rem;
  }

  .section-head.compact {
    align-items: center;
  }

  .section-head .section-kicker {
    margin-bottom: 0.15rem;
  }

  h2 {
    margin: 0;
    font-size: 1rem;
    font-weight: 650;
    letter-spacing: -0.015em;
  }

  .key-hint {
    margin: 0 0 0.5rem;
    color: var(--text-tertiary);
    font-size: 0.625rem;
  }

  /* The reading band. Deliberately quieter than a decision row: one line with
     a count, and the articles only when asked for. */
  .reading {
    margin-top: 0.9rem;
  }

  .reading-toggle {
    display: flex;
    align-items: center;
    gap: 0.55rem;
    width: 100%;
    padding: 0.6rem 0.1rem;
    border: 0;
    border-top: 1px solid var(--card-border);
    border-bottom: 1px solid var(--card-border);
    background: transparent;
    color: var(--text-secondary);
    font: inherit;
    text-align: left;
    cursor: pointer;
  }

  .reading-toggle:hover {
    color: var(--text-primary);
  }

  .reading-toggle span {
    display: flex;
    flex: 1;
    align-items: baseline;
    gap: 0.45rem;
    min-width: 0;
  }

  .reading-toggle strong {
    font-size: 0.8125rem;
    font-weight: 600;
  }

  .reading-toggle small {
    color: var(--text-tertiary);
    font-size: 0.6875rem;
  }

  .reading-toggle em {
    color: var(--primary);
    font-size: 0.6875rem;
    font-style: normal;
    font-weight: 600;
  }

  .reading .queue {
    border-top: 0;
  }

  kbd {
    min-width: 1.25rem;
    padding: 0.1rem 0.25rem;
    border: 1px solid var(--card-border);
    border-bottom-color: var(--card-border-hover);
    border-radius: 3px;
    background: var(--surface);
    color: var(--text-secondary);
    font: 600 0.5625rem var(--font-mono);
    text-align: center;
  }

  .queue {
    border-top: 1px solid var(--card-border);
    border-bottom: 1px solid var(--card-border);
  }

  .decision {
    position: relative;
    display: grid;
    grid-template-columns: auto minmax(0, 1fr);
    gap: 0.8rem;
    padding: 1rem 0.65rem;
    border-bottom: 1px solid var(--card-border);
    transition: background-color 0.15s ease;
  }

  .decision:last-child {
    border-bottom: 0;
  }

  .decision:hover,
  .decision.selected {
    background: var(--primary-soft);
  }

  .decision.selected::before {
    content: "";
    position: absolute;
    inset: 0 auto 0 0;
    width: 2px;
    background: var(--primary);
  }

  .decision-mark,
  .complete-mark,
  .project-mark {
    display: grid;
    place-items: center;
    width: 2rem;
    height: 2rem;
    border-radius: var(--radius-sm);
    background: var(--primary-soft);
    color: var(--primary);
  }

  .decision-mark.warning {
    background: var(--warning-soft);
    color: var(--warning);
  }

  .decision-mark.nightlife {
    background: color-mix(in srgb, #db2777 12%, transparent);
    color: #db2777;
  }

  .decision-copy {
    min-width: 0;
  }

  .kind {
    display: block;
    overflow: hidden;
    margin-bottom: 0.15rem;
    color: var(--text-tertiary);
    font: 600 0.625rem/1.4 var(--font-mono);
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  /* Says why the subject reads oddly, so a row of [number] markers looks like
     a decision the system made rather than a rendering fault. */
  .private-mark {
    color: var(--text-secondary);
  }

  .decision-title {
    display: block;
    overflow-wrap: anywhere;
    font-size: 0.9rem;
    font-weight: 620;
    line-height: 1.35;
  }

  .decision-title:hover {
    color: var(--primary);
  }

  .decision-copy p {
    display: -webkit-box;
    overflow: hidden;
    margin: 0.3rem 0 0;
    color: var(--text-secondary);
    font-size: 0.75rem;
    -webkit-box-orient: vertical;
    -webkit-line-clamp: 2;
    line-clamp: 2;
  }

  .decision-actions {
    grid-column: 2;
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0.25rem;
    margin-top: 0.15rem;
  }

  .decision-actions .btn,
  .project-start {
    padding: 0.3rem 0.55rem;
    font-size: 0.6875rem;
  }

  .action-primary {
    color: var(--primary);
    background: var(--primary-soft);
  }

  .action-primary:hover {
    color: var(--text-inverse);
    background: var(--primary);
  }

  .icon-action {
    width: 1.8rem;
    height: 1.8rem;
    padding: 0;
  }

  .queue-state {
    display: flex;
    align-items: center;
    gap: 0.55rem;
    min-height: 5rem;
    margin: 0;
    padding: 1rem 0.65rem;
    color: var(--text-secondary);
    font-size: 0.75rem;
  }

  .queue-state.complete {
    align-items: flex-start;
    padding-block: 1.25rem;
  }

  .queue-state.complete span:last-child {
    display: grid;
    gap: 0.15rem;
  }

  .complete-mark {
    flex: 0 0 auto;
    background: var(--success-soft);
    color: var(--success);
  }

  .show-all {
    display: flex;
    align-items: center;
    gap: 0.35rem;
    margin: 0.65rem 0 0 auto;
    padding: 0.3rem 0;
    border: 0;
    background: transparent;
    color: var(--text-secondary);
    font: 600 0.6875rem var(--font-sans);
    cursor: pointer;
  }

  .show-all:hover {
    color: var(--primary);
  }

  .unavailable {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 0.35rem;
    margin: 0.8rem 0 0;
    color: var(--text-tertiary);
    font-size: 0.6875rem;
  }

  .unavailable button {
    padding: 0;
    border: 0;
    background: transparent;
    color: var(--primary);
    font: inherit;
    cursor: pointer;
  }

  aside {
    display: flex;
    flex-direction: column;
    gap: 2rem;
  }

  .side-section {
    min-width: 0;
  }

  .quick-list {
    border-top: 1px solid var(--card-border);
  }

  .quick-list a {
    display: grid;
    grid-template-columns: auto minmax(0, 1fr) auto;
    align-items: center;
    gap: 0.7rem;
    padding: 0.8rem 0.1rem;
    border-bottom: 1px solid var(--card-border);
    color: var(--text-secondary);
  }

  .quick-list a > :global(svg):first-child {
    color: var(--primary);
  }

  .quick-list a > :global(svg):last-child {
    color: var(--text-tertiary);
  }

  .quick-list a:hover {
    color: var(--primary);
  }

  .quick-list span,
  .project-copy {
    display: grid;
    min-width: 0;
  }

  .quick-list strong,
  .project-copy strong {
    overflow: hidden;
    color: var(--text-primary);
    font-size: 0.75rem;
    font-weight: 620;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .quick-list small,
  .project-copy small {
    overflow: hidden;
    color: var(--text-tertiary);
    font-size: 0.625rem;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .continue ul {
    margin: 0;
    padding: 0;
    border-top: 1px solid var(--card-border);
    list-style: none;
  }

  .continue li {
    display: grid;
    grid-template-columns: auto minmax(0, 1fr) auto;
    align-items: center;
    gap: 0.65rem;
    padding: 0.65rem 0;
    border-bottom: 1px solid var(--card-border);
  }

  .project-mark {
    width: 1.8rem;
    height: 1.8rem;
  }

  /* ── Compact macmon sidebar card ──────────────────────────── */
  .macmon-compact {
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
    border-top: 1px solid var(--card-border);
    padding: 0.6rem 0.1rem 0.2rem;
    font-size: 0.72rem;
  }

  .mc-temps {
    display: flex;
    align-items: center;
    gap: 0.65rem;
  }

  .mc-temp {
    font-variant-numeric: tabular-nums;
    font-weight: 500;
  }

  .mc-temp.warm {
    color: var(--warning);
  }

  .mc-temp.hot {
    color: var(--danger);
  }

  .mc-power {
    margin-left: auto;
    color: var(--text-tertiary);
    font-size: 0.6875rem;
  }

  .mc-mem {
    display: flex;
    align-items: center;
    gap: 0.45rem;
  }

  .mc-mem-label {
    flex-shrink: 0;
    color: var(--text-tertiary);
    font-size: 0.65rem;
    font-weight: 600;
    text-transform: uppercase;
  }

  .mc-bar {
    flex: 1;
    height: 0.3rem;
    border-radius: 999px;
    background: var(--surface);
    overflow: hidden;
  }

  .mc-fill {
    height: 100%;
    border-radius: 999px;
    background: var(--primary);
    transition: width 0.5s ease;
  }

  .mc-mem-num {
    flex-shrink: 0;
    color: var(--text-secondary);
    font-size: 0.6875rem;
  }

  .mc-detail {
    display: inline-flex;
    align-items: center;
    gap: 0.3rem;
    color: var(--text-tertiary);
    font-size: 0.65rem;
    font-weight: 600;
    margin-top: 0.15rem;
  }

  .mc-detail:hover {
    color: var(--primary);
  }

  .mc-offline {
    display: flex;
    align-items: center;
    gap: 0.35rem;
    margin: 0;
    font-size: 0.6875rem;
    color: var(--text-tertiary);
    border-top: 1px solid var(--card-border);
    padding: 0.6rem 0.1rem 0;
  }

  .mc-offline a {
    color: var(--primary);
    font-weight: 600;
  }

  .status {
    margin-top: auto;
    padding-top: 0.8rem;
    border-top: 1px solid var(--card-border);
  }

  .status summary {
    display: flex;
    align-items: center;
    gap: 0.45rem;
    color: var(--text-tertiary);
    font-size: 0.6875rem;
    cursor: pointer;
    list-style: none;
  }

  .status summary::-webkit-details-marker {
    display: none;
  }

  .status-line {
    flex: 1;
  }

  .status-line .mono {
    color: var(--text-tertiary);
    font-family: var(--font-mono);
    font-size: 0.625rem;
  }

  .status summary > :global(svg) {
    transition: transform 0.15s ease;
  }

  .status[open] summary > :global(svg) {
    transform: rotate(90deg);
  }

  .status-body {
    display: grid;
    gap: 0.6rem;
    padding-top: 0.6rem;
  }

  .capabilities-link {
    display: inline-flex;
    align-items: center;
    gap: 0.3rem;
    color: var(--text-tertiary);
    font-size: 0.6875rem;
  }

  .status-dot {
    width: 0.4rem;
    height: 0.4rem;
    border-radius: 50%;
    background: var(--text-tertiary);
  }

  .status-dot.ok {
    background: var(--success);
  }

  .status-dot.problem {
    background: var(--warning);
  }

  @media (width >= 50rem) {
    .workspace {
      grid-template-columns: minmax(0, 2.2fr) minmax(15rem, 0.8fr);
    }

    .decision {
      grid-template-columns: auto minmax(0, 1fr) auto;
      align-items: center;
      min-height: 5.5rem;
      padding: 1rem 0.8rem;
    }

    .decision-actions {
      grid-column: auto;
      justify-content: flex-end;
      margin-top: 0;
    }
  }

  @media (width < 38rem) {
    .briefing {
      align-items: flex-start;
      flex-direction: column;
      gap: 0.65rem;
      padding-block: 0.15rem 1rem;
    }

    h1 {
      font-size: clamp(1.7rem, 8vw, 2.05rem);
      line-height: 1.05;
    }

    .brief {
      font-size: 0.9rem;
      line-height: 1.45;
    }

    .library-link {
      min-height: 2.5rem;
    }

    .home-views {
      display: grid;
      grid-template-columns: repeat(3, minmax(0, 1fr));
      width: 100%;
      margin-top: 0.85rem;
    }

    .home-views button {
      min-height: 2.75rem;
      padding: 0.5rem 0.35rem;
      font-size: 0.75rem;
    }

    .workspace {
      gap: 2.75rem;
      padding-top: 1rem;
    }

    .decision {
      grid-template-columns: 2.25rem minmax(0, 1fr);
      gap: 0.7rem;
      padding: 1rem 0.25rem;
    }

    .decision-mark {
      width: 2.25rem;
      height: 2.25rem;
    }

    .kind {
      overflow: visible;
      font-size: 0.68rem;
      line-height: 1.45;
      text-overflow: clip;
      white-space: normal;
    }

    .decision-title {
      font-size: 0.95rem;
      line-height: 1.35;
    }

    .decision-copy p {
      font-size: 0.8rem;
      line-height: 1.45;
    }

    .decision-actions {
      grid-column: 1 / -1;
      gap: 0.4rem;
      margin-top: 0.35rem;
    }

    .decision-actions .btn,
    .project-start {
      min-height: 2.5rem;
      padding: 0.55rem 0.75rem;
      font-size: 0.75rem;
    }

    .icon-action {
      width: 2.5rem;
      height: 2.5rem;
    }

    .quick-list a {
      min-height: 3.75rem;
    }

    .quick-list strong,
    .project-copy strong {
      font-size: 0.82rem;
    }

    .quick-list small,
    .project-copy small {
      font-size: 0.7rem;
    }

    .key-hint {
      display: none;
    }
  }
</style>
