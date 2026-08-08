<script lang="ts">
  import { onMount } from "svelte";
  import Icon from "$lib/Icon.svelte";
  import PageHeader from "$lib/PageHeader.svelte";
  import { axonStatus, type UpstreamAudit } from "$lib/api";

  // The M2 gate (tools/upstream-checker) is the source of truth. This page is a compact,
  // read-only lens over its offline result; it neither changes pins nor runs the network audit.
  const PAGE_SIZE = 24;
  type Filter = "all" | "ok" | "na" | "warn" | "fail";
  const FILTERS: { value: Filter; label: string }[] = [
    { value: "all", label: "All" },
    { value: "ok", label: "OK" },
    // Its own filter, not folded into OK: these are the entries release-drift checking cannot
    // describe, so they are the ones whose freshness depends on the stated tracked_by instead.
    { value: "na", label: "Not release-checked" },
    { value: "warn", label: "Warnings" },
    { value: "fail", label: "Failures" },
  ];

  let data = $state<UpstreamAudit | null>(null);
  let error = $state<string | null>(null);
  let filter = $state<Filter>("all");
  let query = $state("");
  let limit = $state(PAGE_SIZE);

  onMount(() => {
    axonStatus
      .upstreams()
      .then((result) => (data = result))
      .catch((cause) => (error = cause instanceof Error ? cause.message : String(cause)));
  });

  const entries = $derived(data?.entries ?? []);
  const shown = $derived(
    entries.filter(
      (entry) =>
        (filter === "all" || entry.status === filter) &&
        (query === "" ||
          entry.name.toLowerCase().includes(query.toLowerCase()) ||
          entry.verdict.toLowerCase().includes(query.toLowerCase())),
    ),
  );
  const visible = $derived(shown.slice(0, limit));
  const remaining = $derived(Math.max(0, shown.length - visible.length));

  $effect(() => {
    filter;
    query;
    limit = PAGE_SIZE;
  });

  function count(value: Filter): number {
    return value === "all" ? entries.length : entries.filter((entry) => entry.status === value).length;
  }

  function isLink(url: string): boolean {
    return url.startsWith("http://") || url.startsWith("https://");
  }
</script>

<PageHeader
  badge="Dependencies"
  title="Upstreams"
  desc="The auditable inventory of external dependencies: verdict, pin, and notes from the shared gate."
/>

{#if error}
  <p class="err"><Icon name="alert" size={14} /> {error}</p>
{:else if !data}
  <p class="loading"><Icon name="loader" size={14} /> Checking dependencies…</p>
{:else}
  <section class="overview" aria-label="Audit result">
    <div>
      <strong>{data.totals.count} dependencies</strong>
      <span>{data.totals.ok} clear</span>
      {#if data.totals.na > 0}
        <span>{data.totals.na} not release-checked</span>
      {/if}
    </div>
    <p class:attention={data.totals.warn > 0 || data.totals.fail > 0}>
      {#if data.totals.fail > 0}
        <Icon name="alert" size={13} /> {data.totals.fail} failures
      {:else if data.totals.warn > 0}
        <Icon name="alert" size={13} /> {data.totals.warn} warnings
      {:else}
        <Icon name="check" size={13} /> Complete: every dependency has a verdict and pin
      {/if}
    </p>
  </section>

  {#if data.offline}
    <p class="offline"><Icon name="wifi-off" size={13} /> Offline snapshot: GitHub drift and cooldown are not included in this view.</p>
  {/if}

  <div class="toolbar">
    <div class="filters" aria-label="Filter audit result">
      {#each FILTERS as option (option.value)}
        <button
          class:active={filter === option.value}
          onclick={() => (filter = option.value)}
          aria-pressed={filter === option.value}
        >
          {option.label}<span>{count(option.value)}</span>
        </button>
      {/each}
    </div>
    <label class="search">
      <span class="sr-only">Filter by name or verdict</span>
      <Icon name="search" size={14} />
      <input type="search" placeholder="Filter…" bind:value={query} />
    </label>
  </div>

  {#if shown.length === 0}
    <p class="empty">No dependencies match this selection.</p>
  {:else}
    <div class="table-wrap">
      <table>
        <thead>
          <tr><th>Dependency</th><th>Verdict</th><th>Pin</th></tr>
        </thead>
        <tbody>
          {#each visible as entry (entry.name)}
            <tr>
              <td class="dependency">
                <span class="dot {entry.status}" aria-label={entry.status}></span>
                {#if isLink(entry.url)}
                  <a href={entry.url} target="_blank" rel="noreferrer">{entry.name} <Icon name="external" size={11} /></a>
                {:else}
                  <span>{entry.name}</span>
                {/if}
                {#if entry.notes.length}
                  <small>{entry.notes.join(" · ")}</small>
                {/if}
              </td>
              <td><span class="verdict">{entry.verdict || "—"}</span></td>
              <td class="pin">{entry.pin || "—"}</td>
            </tr>
          {/each}
        </tbody>
      </table>
    </div>

    {#if remaining > 0}
      <button class="more" onclick={() => (limit += PAGE_SIZE)}>Show {Math.min(PAGE_SIZE, remaining)} more <span>({remaining} remaining)</span></button>
    {/if}
  {/if}
{/if}

<style>
  .overview { display: flex; align-items: center; justify-content: space-between; gap: 1rem; padding: 0.8rem 0; border-top: 1px solid var(--card-border); border-bottom: 1px solid var(--card-border); }
  .overview div { display: flex; align-items: baseline; flex-wrap: wrap; gap: 0.55rem; }
  .overview strong { font-size: 1rem; }
  .overview span, .overview p { color: var(--text-secondary); font-size: 0.8rem; }
  .overview p, .offline, .err, .loading, .empty { display: flex; align-items: center; gap: 0.4rem; margin: 0; font-size: 0.8rem; }
  .overview p { color: var(--success); white-space: nowrap; }
  .overview p.attention, .err { color: var(--warning); }
  .offline { margin: 0.65rem 0 1rem; color: var(--text-secondary); }
  .toolbar { display: flex; align-items: center; justify-content: space-between; gap: 0.75rem; margin-bottom: 0.65rem; }
  .filters { display: flex; gap: 0.3rem; flex-wrap: wrap; }
  .filters button { display: inline-flex; align-items: center; gap: 0.3rem; padding: 0.28rem 0.6rem; background: transparent; border: 1px solid var(--card-border); border-radius: 999px; color: var(--text-secondary); font: inherit; font-size: 0.78rem; cursor: pointer; }
  .filters button:hover { color: var(--text-primary); border-color: var(--text-secondary); }
  .filters button.active { color: var(--primary); border-color: var(--primary); background: color-mix(in srgb, var(--primary) 8%, transparent); font-weight: 650; }
  .filters button span { color: inherit; font-variant-numeric: tabular-nums; opacity: 0.75; }
  .search { display: inline-flex; align-items: center; gap: 0.35rem; width: min(15rem, 100%); padding: 0.35rem 0.55rem; border: 1px solid var(--card-border); border-radius: 7px; color: var(--text-secondary); }
  .search input { width: 100%; min-width: 0; padding: 0; border: 0; outline: 0; background: transparent; color: var(--text-primary); font: inherit; font-size: 0.8rem; }
  .table-wrap { overflow-x: auto; border-top: 1px solid var(--card-border); }
  table { width: 100%; border-collapse: collapse; text-align: left; }
  th { padding: 0.45rem 0.25rem; color: var(--text-secondary); font-size: 0.68rem; font-weight: 650; letter-spacing: 0.05em; text-transform: uppercase; }
  th:first-child, td:first-child { padding-left: 0.2rem; }
  td { padding: 0.52rem 0.25rem; border-top: 1px solid var(--card-border); font-size: 0.82rem; vertical-align: middle; }
  tbody tr:first-child td { border-top: 0; }
  .dependency { min-width: 16rem; }
  .dependency > :not(small) { display: inline-flex; align-items: center; gap: 0.25rem; }
  .dependency a { color: var(--text-primary); font-weight: 600; text-decoration: none; }
  .dependency a:hover { color: var(--primary); text-decoration: underline; }
  .dependency small { display: block; max-width: 44rem; margin: 0.2rem 0 0 1rem; color: var(--text-secondary); font-size: 0.72rem; overflow-wrap: anywhere; }
  .dot { display: inline-block; width: 8px; height: 8px; margin-right: 0.45rem; border-radius: 50%; background: var(--success); }
  .dot.warn { background: var(--warning); }
  .dot.fail { background: var(--danger); }
  /* Hollow, not green: nothing checked this entry, which is not the same as it passing. */
  .dot.na { background: transparent; box-shadow: inset 0 0 0 1.5px var(--text-secondary); }
  .verdict { display: inline-block; padding: 0.1rem 0.38rem; border: 1px solid var(--card-border); border-radius: 4px; color: var(--text-secondary); font-size: 0.68rem; letter-spacing: 0.04em; text-transform: uppercase; }
  .pin { color: var(--text-secondary); font-family: var(--font-mono); font-size: 0.76rem; overflow-wrap: anywhere; }
  .more { display: block; width: 100%; margin-top: 0.75rem; padding: 0.55rem; border: 1px solid var(--card-border); border-radius: 7px; background: transparent; color: var(--primary); font: inherit; font-size: 0.8rem; cursor: pointer; }
  .more:hover { background: var(--nav-hover); }
  .more span { color: var(--text-secondary); }
  .err, .loading, .empty { margin-bottom: 1rem; }
  .sr-only { position: absolute; width: 1px; height: 1px; padding: 0; margin: -1px; overflow: hidden; clip: rect(0, 0, 0, 0); white-space: nowrap; border: 0; }
  @media (max-width: 42rem) { .overview, .toolbar { align-items: flex-start; flex-direction: column; } .search { width: 100%; } th:nth-child(2), td:nth-child(2) { display: none; } .pin { max-width: 11rem; } }
</style>
