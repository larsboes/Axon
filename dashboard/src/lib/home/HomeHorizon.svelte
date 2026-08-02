<script lang="ts">
  import Icon from "$lib/Icon.svelte";
  import { contextLink, entryLink, kindConfig } from "$lib/calendar/types";
  import type { CalendarContext, CalendarEntry } from "$lib/api";

  let {
    contexts,
    entries,
  }: {
    contexts: CalendarContext[];
    entries: CalendarEntry[];
  } = $props();

  const contextLabels: Record<string, string> = {
    uncertainty: "Date window",
    transition: "Transition",
    preference: "Preference",
    planning_gap: "Planning gap",
    note: "Note",
  };

  function shortDate(value: string) {
    return new Date(`${value.slice(0, 10)}T12:00:00`).toLocaleDateString("en-GB", {
      day: "numeric",
      month: "short",
    });
  }

  function entryTime(entry: CalendarEntry) {
    const date = shortDate(entry.starts_at);
    if (entry.all_day) return date;
    return `${date}, ${entry.starts_at.slice(11, 16)}`;
  }
</script>

{#if contexts.length > 0 || entries.length > 0}
  <section class="horizon">
    {#if contexts.length > 0}
      <div class="horizon-section">
        <div class="head">
          <span>Current context</span>
          <a href="/calendar">Edit <Icon name="arrow-right" size={11} /></a>
        </div>
        <div class="context-grid">
          {#each contexts.slice(0, 4) as context (context.id)}
            <a class="context" href={contextLink(context)}>
              <small>{contextLabels[context.kind] ?? "Context"}</small>
              <strong>{context.title}</strong>
              <span>{shortDate(context.valid_from)} – {shortDate(context.valid_until)}</span>
            </a>
          {/each}
        </div>
      </div>
    {/if}

    {#if entries.length > 0}
      <div class="horizon-section schedule">
        <div class="head">
          <span>Coming up</span>
          <a href="/calendar">Calendar <Icon name="arrow-right" size={11} /></a>
        </div>
        <div class="entry-list">
          {#each entries.slice(0, 4) as entry (entry.id)}
            <a href={entryLink(entry)}>
              <i style={`--entry-color: ${kindConfig(entry.kind).color}`}></i>
              <span>
                <strong>{entry.title}</strong>
                <small>
                  {entryTime(entry)}
                  {#if entry.location} · {entry.location}{/if}
                </small>
              </span>
              <em>{entry.commitment === "committed" ? "Committed" : "Planned"}</em>
            </a>
          {/each}
        </div>
      </div>
    {/if}
  </section>
{/if}

<style>
  .horizon {
    display: grid;
    gap: 0.8rem;
    margin-bottom: 1.15rem;
    padding: 0.75rem;
    border: 1px solid var(--card-border);
    border-radius: var(--radius-lg);
    background: var(--card-bg);
  }

  .head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: 0.45rem;
  }

  .head > span {
    color: var(--primary);
    font-size: 0.65rem;
    font-weight: 700;
    letter-spacing: 0.08em;
    text-transform: uppercase;
  }

  .head a {
    display: inline-flex;
    align-items: center;
    gap: 0.25rem;
    color: var(--text-tertiary);
    font-size: 0.65rem;
  }

  .context-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(10rem, 1fr));
    gap: 0.45rem;
  }

  .context {
    display: grid;
    gap: 0.15rem;
    padding: 0.45rem 0.55rem;
    border-radius: var(--radius-sm);
    background: var(--primary-soft);
  }

  .context:hover { box-shadow: inset 0 0 0 1px var(--primary); }
  .context small { color: var(--primary); font-size: 0.58rem; font-weight: 700; text-transform: uppercase; }
  .context strong { font-size: 0.7rem; line-height: 1.25; }
  .context span { color: var(--text-secondary); font-size: 0.58rem; }

  .entry-list {
    border-top: 1px solid var(--card-border);
  }

  .entry-list a {
    display: grid;
    grid-template-columns: auto minmax(0, 1fr) auto;
    align-items: center;
    gap: 0.55rem;
    padding: 0.38rem 0;
    border-bottom: 1px solid var(--card-border);
  }

  .entry-list a:last-child { border-bottom: 0; }
  .entry-list i { width: 0.5rem; height: 0.5rem; border-radius: 50%; background: var(--entry-color); }
  .entry-list span { display: grid; min-width: 0; }
  .entry-list strong { overflow: hidden; font-size: 0.69rem; text-overflow: ellipsis; white-space: nowrap; }
  .entry-list small { color: var(--text-tertiary); font-size: 0.57rem; }
  .entry-list em { color: var(--text-secondary); font-size: 0.55rem; font-style: normal; text-transform: uppercase; }

  @media (width >= 48rem) {
    .horizon { grid-template-columns: minmax(0, 1fr) minmax(0, 1.1fr); }
  }

  @media (width < 38rem) {
    .horizon {
      gap: 1rem;
      padding: 0.85rem;
    }

    .horizon-section {
      min-width: 0;
    }

    .head {
      margin-bottom: 0.65rem;
    }

    .head > span,
    .head a {
      font-size: 0.7rem;
    }

    .head a {
      min-height: 2.5rem;
      margin-block: -0.75rem;
      padding-inline-start: 0.75rem;
    }

    .context-grid {
      grid-template-columns: none;
      grid-auto-flow: column;
      grid-auto-columns: minmax(14.5rem, 88%);
      width: 100%;
      overflow-x: auto;
      overscroll-behavior-inline: contain;
      padding-bottom: 0.25rem;
      scroll-snap-type: inline mandatory;
      scrollbar-width: none;
    }

    .context-grid::-webkit-scrollbar {
      display: none;
    }

    .context {
      min-height: 5rem;
      align-content: center;
      gap: 0.2rem;
      padding: 0.7rem 0.75rem;
      scroll-snap-align: start;
    }

    .context small {
      font-size: 0.65rem;
    }

    .context strong {
      font-size: 0.82rem;
      line-height: 1.3;
    }

    .context span {
      font-size: 0.68rem;
    }

    .entry-list a {
      gap: 0.65rem;
      min-height: 3.25rem;
      padding-block: 0.6rem;
    }

    .entry-list strong {
      font-size: 0.78rem;
    }

    .entry-list small,
    .entry-list em {
      font-size: 0.65rem;
    }
  }
</style>
