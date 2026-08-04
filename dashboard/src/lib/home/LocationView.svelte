<script lang="ts">
  import Icon from "$lib/Icon.svelte";
  import { entryLink } from "$lib/calendar/types";
  import type { CalendarEntry, ScoutingOpportunity, TripPlan } from "$lib/api";

  let {
    entries,
    opportunities,
    plans,
  }: {
    entries: CalendarEntry[];
    opportunities: ScoutingOpportunity[];
    plans: TripPlan[];
  } = $props();

  type Item = {
    id: string;
    location: string;
    title: string;
    date: string;
    meta: string;
    href: string;
    external?: boolean;
    rank: number;
  };

  type Group = { location: string; items: Item[]; nextDate: string; rank: number };
  const todayKey = [
    new Date().getFullYear(),
    String(new Date().getMonth() + 1).padStart(2, "0"),
    String(new Date().getDate()).padStart(2, "0"),
  ].join("-");

  let groups = $derived.by<Group[]>(() => {
    const byLocation = new Map<string, Item[]>();
    const push = (item: Item) => {
      const items = byLocation.get(item.location) ?? [];
      items.push(item);
      byLocation.set(item.location, items);
    };

    for (const entry of entries) {
      if (entry.ends_at.slice(0, 10) < todayKey) continue;
      push({
        id: `calendar:${entry.id}`,
        location: entryCity(entry),
        title: entry.title,
        date: entry.starts_at,
        meta: `${entry.commitment === "committed"
          ? "Committed"
          : entry.commitment === "planned"
            ? "Planned"
            : "Possible"} · Calendar`,
        href: entryLink(entry),
        rank: entry.commitment === "committed" ? 1_000 : entry.commitment === "planned" ? 800 : 250,
      });
    }
    for (const opportunity of opportunities) {
      if (opportunity.ends_at && opportunity.ends_at.slice(0, 10) < todayKey) continue;
      if (!opportunity.ends_at && opportunity.starts_at && opportunity.starts_at.slice(0, 10) < todayKey) continue;
      push({
        id: `opportunity:${opportunity.id}`,
        location: cleanLocation(opportunity.city || opportunity.location),
        title: opportunity.title,
        date: opportunity.starts_at || "9999-12-31",
        meta: staleScore(opportunity)
          ? `${opportunity.source} · score out of date`
          : `${opportunity.source} · ${opportunity.matched_focus || "not yet scored"}`,
        href: opportunity.url,
        external: true,
        rank: staleScore(opportunity) ? 50 : 300 + opportunity.score * 100,
      });
    }
    for (const plan of plans) {
      if (plan.date_end < todayKey) continue;
      const destinations = plan.destinations.length > 0 ? plan.destinations : [plan.origin];
      for (const destination of destinations) {
        push({
          id: `trip:${plan.id}:${destination.id}`,
          location: cleanLocation(destination.name),
          title: plan.title,
          date: plan.date_start,
          meta: "Travel planning",
          href: "/travel",
          rank: 700,
        });
      }
    }

    return [...byLocation.entries()]
      .map(([location, items]) => {
        items.sort((a, b) => b.rank - a.rank || a.date.localeCompare(b.date));
        return {
          location,
          items,
          nextDate: items.reduce(
            (earliest, item) => item.date < earliest ? item.date : earliest,
            "9999-12-31",
          ),
          rank: items[0]?.rank ?? 0,
        };
      })
      .sort((a, b) => b.rank - a.rank || a.nextDate.localeCompare(b.nextDate) || a.location.localeCompare(b.location));
  });

  function staleScore(opportunity: ScoutingOpportunity) {
    return opportunity.opportunity_type === "event"
      && /scholarship/i.test(opportunity.matched_focus);
  }

  function entryCity(entry: CalendarEntry) {
    if (entry.payload && typeof entry.payload === "object" && "city" in entry.payload) {
      const city = (entry.payload as { city?: unknown }).city;
      if (typeof city === "string" && city.trim()) return cleanLocation(city);
    }
    return cleanLocation(entry.location ?? "");
  }

  function cleanLocation(value: string) {
    const text = value.trim();
    if (!text || !/[\p{L}\p{N}]/u.test(text)) return "Location undecided";
    if (/online|remote|virtual/i.test(text)) return "Online";
    const parts = text.split(",").map((part) => part.trim()).filter(Boolean);
    const last = parts.at(-1) ?? text;
    return last.replace(/^\d{5}\s+/, "") || text;
  }

  function dateLabel(value: string) {
    if (value.startsWith("9999")) return "Date undecided";
    return new Date(`${value.slice(0, 10)}T12:00:00`).toLocaleDateString("en-GB", {
      weekday: "short",
      day: "numeric",
      month: "short",
    });
  }
</script>

<section class="locations">
  <!-- The kicker and title live in Home's shared section head, which owns the
       view switcher too. Only the sentence that explains the grouping is
       this view's own. -->
  <p class="intro">
    Committed events, open opportunities, and trips grouped by location, ordered by date.
  </p>

  {#if groups.length === 0}
    <p class="empty">No events, opportunities, or trips with a location yet.</p>
  {:else}
    <div class="groups">
      {#each groups as group (group.location)}
        <article>
          <header>
            <span class="pin"><Icon name="map-pin" size={14} /></span>
            <div>
              <h3>{group.location}</h3>
              <small>{group.items.length} {group.items.length === 1 ? "item" : "items"} · next {dateLabel(group.nextDate)}</small>
            </div>
          </header>
          <div class="items">
            {#each group.items.slice(0, 5) as item (item.id)}
              <a href={item.href} target={item.external ? "_blank" : undefined} rel={item.external ? "noreferrer" : undefined}>
                <time>{dateLabel(item.date)}</time>
                <span>
                  <strong>{item.title}</strong>
                  <small>{item.meta}</small>
                </span>
                <Icon name={item.external ? "external" : "arrow-right"} size={11} />
              </a>
            {/each}
          </div>
        </article>
      {/each}
    </div>
  {/if}
</section>

<style>
  .intro { max-width: 42rem; margin: 0 0 1rem; color: var(--text-secondary); font-size: .72rem; }

  .groups { display: grid; gap: .8rem; }
  article { overflow: hidden; border: 1px solid var(--card-border); border-radius: var(--radius-lg); background: var(--card-bg); }
  article header { display: flex; align-items: center; gap: .7rem; padding: .8rem .9rem; background: var(--surface); }
  .pin { display: grid; place-items: center; width: 2rem; height: 2rem; border-radius: 50%; background: var(--primary-soft); color: var(--primary); }
  header div { display: grid; }
  h3 { margin: 0; font-size: .82rem; }
  header small { color: var(--text-tertiary); font-size: .61rem; }

  .items a { display: grid; grid-template-columns: 5.5rem minmax(0, 1fr) auto; align-items: center; gap: .65rem; padding: .7rem .9rem; border-top: 1px solid var(--card-border); }
  .items a:hover { background: var(--primary-soft); }
  time { color: var(--text-tertiary); font: 600 .6rem var(--font-mono); }
  .items span { display: grid; min-width: 0; }
  .items strong { overflow: hidden; font-size: .73rem; text-overflow: ellipsis; white-space: nowrap; }
  .items small { overflow: hidden; color: var(--text-tertiary); font-size: .6rem; text-overflow: ellipsis; white-space: nowrap; }
  .empty { padding: 2rem; border: 1px dashed var(--card-border); color: var(--text-secondary); font-size: .75rem; text-align: center; }

  @media (width >= 62rem) {
    .groups { grid-template-columns: repeat(2, minmax(0, 1fr)); }
  }
</style>
