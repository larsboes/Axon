<script lang="ts">
  import Icon from "$lib/Icon.svelte";
  import type {
    CalendarContext,
    CalendarEntry,
    FeedEntry,
    ScoutingOpportunity,
    ScoutingSource,
    TripPlan,
  } from "$lib/api";

  let {
    feedEntries,
    opportunities,
    scoutingSources,
    calendarEntries,
    contexts,
    plans,
  }: {
    feedEntries: FeedEntry[];
    opportunities: ScoutingOpportunity[];
    scoutingSources: ScoutingSource[];
    calendarEntries: CalendarEntry[];
    contexts: CalendarContext[];
    plans: TripPlan[];
  } = $props();

  type SourceCard = {
    key: string;
    title: string;
    role: string;
    count: number;
    status: string;
    href: string;
    action: string;
  };

  let cards = $derived.by<SourceCard[]>(() => {
    const configuredObsidian = scoutingSources.filter((source) => source.adapter === "obsidian-markdown");
    const discoveryAdapters = scoutingSources.filter((source) => source.adapter !== "obsidian-markdown");
    const webEntries = calendarEntries.filter((entry) => entry.source === "web" || entry.source === "luma");
    return [
      {
        key: "obsidian",
        title: "Obsidian",
        role: "Profiles and curated personal context",
        count: configuredObsidian.length,
        status: configuredObsidian.every((source) => source.configured) ? "connected" : "check setup",
        href: "/feed?view=discover",
        action: "Use profiles",
      },
      {
        key: "calendar",
        title: "Calendar",
        role: "Commitments, free time, and soft planning contexts",
        count: calendarEntries.length + contexts.length,
        status: "live",
        href: "/calendar",
        action: "Open",
      },
      {
        key: "discovery",
        title: "Luma & Discovery",
        role: "Events, meetups, hackathons, and other opportunities",
        count: opportunities.length,
        status: `${discoveryAdapters.filter((source) => source.enabled).length} active sources`,
        href: "/feed?view=discover",
        action: "Scan",
      },
      {
        key: "web",
        title: "Individual web sources",
        role: "Deliberately added event pages with their original evidence",
        count: webEntries.length,
        status: "curated",
        href: "/calendar",
        action: "View in calendar",
      },
      {
        key: "feed",
        title: "Feed",
        role: "Articles, videos, repositories, and observations",
        count: feedEntries.length,
        status: "30-day window",
        href: "/feed",
        action: "Add a link",
      },
      {
        key: "trips",
        title: "Travel planning",
        role: "Places, stages, and decisions from Trips",
        count: plans.length,
        status: "connected",
        href: "/travel",
        action: "Plan",
      },
    ];
  });
</script>

<section class="sources">
  <!-- Kicker and title come from Home's shared section head; the caveat about
       what the counts mean is this view's own and has to stay. -->
  <p class="intro">
    Each source has a bounded role. Counts refer to the current Home window, not an undisclosed full scan.
  </p>

  <div class="source-grid">
    {#each cards as card (card.key)}
      <a href={card.href}>
        <header>
          <span class="source-icon"><Icon name={card.key === "calendar" ? "calendar" : card.key === "trips" ? "map-pin" : card.key === "feed" ? "feed" : "compass"} size={15} /></span>
          <em>{card.status}</em>
        </header>
        <strong>{card.title}</strong>
        <p>{card.role}</p>
        <footer>
          <span>{card.count} in the current window</span>
          <span>{card.action} <Icon name="arrow-right" size={11} /></span>
        </footer>
      </a>
    {/each}
  </div>

  <p class="boundary">
    New sources are never given blanket access to the vault. Obsidian supplies declared profiles and notes, Discovery supplies opportunities, and Calendar supplies time and commitment.
  </p>
</section>

<style>
  .intro { max-width: 42rem; margin: 0 0 1rem; color: var(--text-secondary); font-size: .72rem; }

  .source-grid { display: grid; gap: .7rem; grid-template-columns: repeat(auto-fit, minmax(13rem, 1fr)); }
  .source-grid > a { display: grid; min-height: 9.5rem; padding: .85rem; border: 1px solid var(--card-border); border-radius: var(--radius-lg); background: var(--card-bg); }
  .source-grid > a:hover { border-color: var(--primary); transform: translateY(-1px); }
  header { display: flex; align-items: center; justify-content: space-between; }
  .source-icon { display: grid; place-items: center; width: 2rem; height: 2rem; border-radius: var(--radius-sm); background: var(--primary-soft); color: var(--primary); }
  header em { color: var(--text-tertiary); font-size: .58rem; font-style: normal; text-transform: uppercase; }
  .source-grid strong { margin-top: .65rem; font-size: .8rem; }
  .source-grid p { margin: .25rem 0 .7rem; color: var(--text-secondary); font-size: .66rem; line-height: 1.4; }
  footer { display: flex; align-items: center; justify-content: space-between; gap: .5rem; margin-top: auto; padding-top: .55rem; border-top: 1px solid var(--card-border); color: var(--text-tertiary); font-size: .58rem; }
  footer span:last-child { display: inline-flex; align-items: center; gap: .2rem; color: var(--primary); font-weight: 650; }
  .boundary { margin: 1rem 0 0; padding: .8rem; border-left: 2px solid var(--primary); background: var(--primary-soft); color: var(--text-secondary); font-size: .68rem; line-height: 1.45; }
</style>
