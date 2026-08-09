<script lang="ts">
  import Icon from "$lib/Icon.svelte";
  import { finance, type FinanceDashboard, type TransactionCandidate } from "$lib/api";

  let { data, start, end, account, onchanged }: {
    data: FinanceDashboard;
    start: string;
    end: string;
    account: string;
    onchanged?: () => void | Promise<void>;
  } = $props();
  let candidates = $state<TransactionCandidate[]>([]);
  let selected = $state<Record<string, boolean>>({});
  let targets = $state<Record<string, string>>({});
  let expanded = $state<Record<string, boolean>>({});
  let showAll = $state(false);
  let busyCluster = $state("");
  let error = $state<string | null>(null);
  let notice = $state<string | null>(null);

  const categoryOptions = $derived(data.categories.filter((account) =>
    account.startsWith("expenses:") && !account.split(":").includes("uncategorized"),
  ));
  const clusters = $derived.by(() => {
    const grouped = new Map<string, TransactionCandidate[]>();
    for (const candidate of candidates) {
      if (candidate.state !== "confirmed"
        || candidate.amount_cents >= 0
        || candidate.booked_at < start
        || candidate.booked_at > end
        || (account !== "" && candidate.source_account !== account)
        || !candidate.proposed_account.split(":").includes("uncategorized")) continue;
      const key = candidate.description.trim().replace(/\s+/g, " ").toLocaleLowerCase();
      grouped.set(key, [...(grouped.get(key) ?? []), candidate]);
    }
    return [...grouped.values()]
      .map((entries) => ({
        id: entries[0].id,
        label: entries[0].description,
        entries: entries.sort((left, right) => right.booked_at.localeCompare(left.booked_at)),
        totalCents: entries.reduce((sum, candidate) => sum + Math.abs(candidate.amount_cents), 0),
      }))
      .sort((left, right) => right.totalCents - left.totalCents || right.entries.length - left.entries.length);
  });
  const visibleClusters = $derived(showAll ? clusters : clusters.slice(0, 12));
  const uncategorizedTotal = $derived(clusters.reduce((sum, cluster) => sum + cluster.totalCents, 0));

  function money(cents: number, currency = data.summary.currency) {
    return new Intl.NumberFormat("de-DE", { style: "currency", currency }).format(cents / 100);
  }

  function selectedCount(cluster: (typeof clusters)[number]) {
    return cluster.entries.filter((candidate) => selected[candidate.id]).length;
  }

  function selectCluster(cluster: (typeof clusters)[number], checked: boolean) {
    selected = {
      ...selected,
      ...Object.fromEntries(cluster.entries.map((candidate) => [candidate.id, checked])),
    };
  }

  async function classify(cluster: (typeof clusters)[number]) {
    const account = targets[cluster.id]?.trim();
    const entries = cluster.entries.filter((candidate) => selected[candidate.id]);
    if (!account || entries.length === 0) return;
    busyCluster = cluster.id;
    error = null;
    notice = null;
    try {
      const result = await finance.reclassifyCandidates(
        entries.map((candidate) => ({ id: candidate.id, account })),
      );
      notice = `${result.reclassified} reviewed expense${result.reclassified === 1 ? "" : "s"} reclassified; the journal and analytics projection were rebuilt.`;
      selected = Object.fromEntries(Object.entries(selected).filter(([id]) =>
        !entries.some((candidate) => candidate.id === id),
      ));
      await load();
      await onchanged?.();
    } catch (cause) {
      error = cause instanceof Error ? cause.message : String(cause);
    } finally {
      busyCluster = "";
    }
  }

  async function load() {
    try {
      candidates = await finance.candidates();
      error = null;
    } catch (cause) {
      error = cause instanceof Error ? cause.message : String(cause);
    }
  }

  $effect(() => { void load(); });
</script>

<section class="categorization-review">
  <div class="heading">
    <div>
      <h2>Categorization review</h2>
      <p>Largest confirmed uncategorized merchant clusters first. Select explicit rows, choose one reviewed account, then update the canonical journal.</p>
    </div>
    <div class="remaining"><strong>{money(uncategorizedTotal)}</strong><span>gross candidate value · {clusters.reduce((sum, cluster) => sum + cluster.entries.length, 0)} rows</span></div>
  </div>

  <datalist id="finance-category-options">
    {#each categoryOptions as account (account)}<option value={account}></option>{/each}
  </datalist>
  {#if error}<p class="error"><Icon name="alert" size={14} /> {error}</p>{/if}
  {#if notice}<p class="notice">{notice}</p>{/if}

  {#if clusters.length}
    <div class="cluster-list">
      {#each visibleClusters as cluster (cluster.id)}
        <article>
          <div class="cluster-summary">
            <button class="disclosure" type="button" aria-expanded={expanded[cluster.id] ?? false} onclick={() => expanded = { ...expanded, [cluster.id]: !expanded[cluster.id] }}>
              <span>{expanded[cluster.id] ? "−" : "+"}</span>
              <strong>{cluster.label}</strong>
            </button>
            <span>{cluster.entries.length} transaction{cluster.entries.length === 1 ? "" : "s"}</span>
            <strong>{money(cluster.totalCents, cluster.entries[0].currency)}</strong>
          </div>
          {#if expanded[cluster.id]}
            <div class="cluster-review">
              <div class="selection-tools">
                <button type="button" onclick={() => selectCluster(cluster, true)}>Select all</button>
                <button type="button" onclick={() => selectCluster(cluster, false)}>Clear</button>
                <span>{selectedCount(cluster)} selected</span>
              </div>
              <div class="rows">
                {#each cluster.entries as candidate (candidate.id)}
                  <label>
                    <input type="checkbox" checked={selected[candidate.id] ?? false} onchange={(event) => selected = { ...selected, [candidate.id]: (event.currentTarget as HTMLInputElement).checked }} />
                    <time>{candidate.booked_at}</time>
                    <span>{candidate.description}</span>
                    <strong>{money(Math.abs(candidate.amount_cents), candidate.currency)}</strong>
                  </label>
                {/each}
              </div>
              <form onsubmit={(event) => { event.preventDefault(); void classify(cluster); }}>
                <label>Reviewed expense account
                  <input list="finance-category-options" bind:value={targets[cluster.id]} placeholder="expenses:food:groceries" />
                </label>
                <button disabled={busyCluster !== "" || selectedCount(cluster) === 0 || !targets[cluster.id]?.trim()}>
                  Apply to {selectedCount(cluster)} selected
                </button>
              </form>
            </div>
          {/if}
        </article>
      {/each}
    </div>
    {#if clusters.length > 12}
      <button class="show-all" type="button" onclick={() => showAll = !showAll}>{showAll ? "Show largest 12" : `Show all ${clusters.length} clusters`}</button>
    {/if}
  {:else}
    <p class="empty">Every confirmed expense candidate has a reviewed category.</p>
  {/if}
</section>

<style>
  .categorization-review { border: 1px solid var(--border, #333); border-radius: 8px; padding: .9rem; margin-bottom: .75rem; }
  .heading { display: flex; align-items: start; justify-content: space-between; gap: 1rem; }
  h2 { margin: 0; font-size: .95rem; }
  p { margin: .2rem 0 0; color: var(--muted, #888); font-size: .7rem; }
  .remaining { display: grid; justify-items: end; gap: .1rem; white-space: nowrap; }
  .remaining strong { font-size: 1rem; font-variant-numeric: tabular-nums; }
  .remaining span, .cluster-summary > span, .selection-tools span { color: var(--muted, #888); font-size: .65rem; }
  .cluster-list { margin-top: .75rem; border-top: 1px solid var(--border, #333); }
  article { border-bottom: 1px solid var(--border, #333); }
  .cluster-summary { display: grid; grid-template-columns: minmax(12rem, 1fr) 7rem 7rem; align-items: center; gap: .7rem; min-height: 2.8rem; font-size: .7rem; }
  .cluster-summary > strong { text-align: right; font-variant-numeric: tabular-nums; }
  button, input { min-width: 0; border: 1px solid var(--border, #333); border-radius: 6px; padding: .38rem .55rem; font: inherit; font-size: .7rem; background: transparent; color: inherit; }
  button { cursor: pointer; }
  button:disabled { opacity: .45; cursor: default; }
  .disclosure { display: flex; align-items: center; gap: .5rem; padding-inline: 0; border: 0; text-align: left; }
  .disclosure > span { width: .8rem; color: var(--muted, #888); font-size: .9rem; }
  .cluster-review { padding: .1rem 0 .75rem 1.3rem; }
  .selection-tools { display: flex; align-items: center; gap: .4rem; margin-bottom: .45rem; }
  .selection-tools button { padding: .25rem .4rem; border-color: transparent; }
  .rows { max-height: 12rem; overflow: auto; }
  .rows label { display: grid; grid-template-columns: 1.2rem 5.5rem minmax(12rem, 1fr) 7rem; align-items: center; gap: .5rem; padding: .35rem 0; font-size: .68rem; }
  .rows input { width: .85rem; height: .85rem; margin: 0; padding: 0; }
  .rows time { color: var(--muted, #888); }
  .rows strong { text-align: right; font-variant-numeric: tabular-nums; }
  form { display: grid; grid-template-columns: minmax(14rem, 1fr) auto; align-items: end; gap: .55rem; margin-top: .55rem; }
  form label { display: grid; gap: .2rem; color: var(--muted, #888); font-size: .65rem; }
  .show-all { margin-top: .65rem; }
  .error { color: var(--danger, #b44); }
  .notice { color: var(--primary); }
  .empty { margin-top: .8rem; }
  @media (max-width: 700px) { .heading { flex-direction: column; } .remaining { justify-items: start; } .cluster-summary { grid-template-columns: 1fr auto; } .cluster-summary > span { grid-column: 1; } .rows label { grid-template-columns: 1.2rem 5rem 1fr; } .rows strong { grid-column: 3; text-align: left; } form { grid-template-columns: 1fr; } }
</style>
