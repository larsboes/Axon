<script lang="ts">
  /**
   * Rooms, and what stands in them.
   *
   * **This page renders and does not compute.** Every verdict, every corridor width and the
   * plan drawing itself arrive finished from `/interior/api/…`. A clearance rule reimplemented
   * here would be a second answer to a question that already has one, and drift between the
   * two is the failure the capability exists to prevent (PRD B27).
   *
   * The one arithmetic below is cents to euros for display.
   */
  import { onMount } from "svelte";
  import Icon from "$lib/Icon.svelte";
  import PageHeader from "$lib/PageHeader.svelte";
  import {
    axonStatus,
    interior,
    type InteriorItem,
    type InteriorLayoutDetail,
    type InteriorLayoutSummary,
    type InteriorState,
    type InteriorWishlist,
  } from "$lib/api";
  import { capabilities } from "$lib/capabilities.svelte";

  type View = "plans" | "inventory";

  let view = $state<View>("plans");
  let loading = $state(true);
  let error = $state<string | null>(null);
  let starting = $state(false);

  let layouts = $state<InteriorLayoutSummary[]>([]);
  let selected = $state<string | null>(null);
  let detail = $state<InteriorLayoutDetail | null>(null);
  let detailError = $state<string | null>(null);

  let inventory = $state<{ item: InteriorItem; state: InteriorState | null }[]>([]);
  let wishlist = $state<InteriorWishlist | null>(null);

  const euro = (cents: number): string =>
    (cents / 100).toLocaleString("de-DE", { style: "currency", currency: "EUR" });

  const size = (i: InteriorItem): string =>
    i.b !== null && i.t !== null ? `${i.b} × ${i.t}${i.h !== null ? ` × ${i.h}` : ""} cm` : "—";

  /** The lower edge: a product's own price, otherwise the bottom of its estimate. */
  const floorPrice = (i: InteriorItem): number | null => i.preis_cent ?? i.kosten_min_cent;

  const owned = $derived(inventory.filter((r) => r.state === "owned"));
  const gone = $derived(inventory.filter((r) => r.state === "gone"));

  async function load(): Promise<void> {
    loading = true;
    error = null;
    try {
      const [l, inv, wish] = await Promise.all([
        interior.layouts(),
        interior.inventory(),
        interior.wishlist(),
      ]);
      layouts = l;
      inventory = inv;
      wishlist = wish;
      if (selected === null && l.length > 0) await select(l[0].id);
    } catch (caught) {
      error = caught instanceof Error ? caught.message : String(caught);
    } finally {
      loading = false;
    }
  }

  async function select(id: string): Promise<void> {
    selected = id;
    detail = null;
    detailError = null;
    try {
      detail = await interior.layout(id);
    } catch (caught) {
      detailError = caught instanceof Error ? caught.message : String(caught);
    }
  }

  async function start(): Promise<void> {
    starting = true;
    error = null;
    try {
      await axonStatus.start("interior");
      await capabilities.refresh();
      await load();
    } catch (caught) {
      error = caught instanceof Error ? caught.message : String(caught);
    } finally {
      starting = false;
    }
  }

  onMount(() => {
    capabilities.subscribe();
    void load();
  });
</script>

<PageHeader
  badge="Interior"
  title="Rooms and what stands in them"
  desc="Layouts judged against the flat's own clearance rules, and an inventory that outlives the flat."
/>

{#if error}
  <div class="card offer">
    <p class="lead"><Icon name="alert" size={15} /> {error}</p>
    <!-- On-demand is the design: nothing but the shell runs until you open something. -->
    <button onclick={start} disabled={starting}>
      {starting ? "Starting…" : "Start interior"}
    </button>
  </div>
{:else if loading}
  <p class="empty">Reading the model…</p>
{:else}
  <nav class="views" aria-label="Interior views">
    <button class:active={view === "plans"} onclick={() => (view = "plans")}>
      Plans <span class="count">{layouts.length}</span>
    </button>
    <button class:active={view === "inventory"} onclick={() => (view = "inventory")}>
      Inventory <span class="count">{inventory.length}</span>
    </button>
  </nav>
{/if}

{#if !loading && !error && view === "plans"}
  {#if layouts.length === 0}
    <p class="empty">No layout on disk for this flat.</p>
  {:else}
    <div class="plans">
      <ul class="variants">
        {#each layouts as l (l.id)}
          <li>
            <button class:active={selected === l.id} onclick={() => select(l.id)}>
              <span class="verdict" class:pass={l.pass}>{l.pass ? "passes" : "fails"}</span>
              <span class="name">{l.name}</span>
              <span class="tally mono">
                {l.hard} hard · {l.soft} soft
              </span>
            </button>
          </li>
        {/each}
      </ul>

      <section class="plan">
        {#if detailError}
          <p class="error"><Icon name="alert" size={15} /> {detailError}</p>
        {:else if detail === null}
          <p class="empty">Drawing…</p>
        {:else}
          <!-- The capability draws it. eslint-disable-next-line svelte/no-at-html-tags -->
          <div class="svg">{@html detail.svg}</div>

          <dl class="metrics mono">
            <div><dt>room</dt><dd>{detail.check.metrics.room_area_m2.toFixed(2)} m²</dd></div>
            <div><dt>occupied</dt><dd>{detail.check.metrics.occupied_area_m2.toFixed(2)} m²</dd></div>
            <div><dt>free</dt><dd>{detail.check.metrics.free_area_m2.toFixed(2)} m²</dd></div>
          </dl>

          <table class="corridors">
            <caption>Walks, at their narrowest point</caption>
            <tbody>
              {#each detail.check.metrics.corridors as c (c.from + c.to)}
                <tr>
                  <th scope="row">{c.from} → {c.to}</th>
                  <td class="mono">{c.width_cm === null ? "no way through" : `${c.width_cm} cm`}</td>
                </tr>
              {/each}
            </tbody>
          </table>

          {#if detail.check.hard.length > 0}
            <ul class="violations hard">
              {#each detail.check.hard as v, i (v.rule + i)}
                <li>
                  <span class="rule mono">{v.rule}</span>
                  {v.message}
                  {#if v.text}<span class="rule-text">{v.text}</span>{/if}
                </li>
              {/each}
            </ul>
          {/if}
          {#if detail.check.soft.length > 0}
            <ul class="violations soft">
              {#each detail.check.soft as v, i (v.rule + i)}
                <li>
                  <span class="rule mono">{v.rule}</span>
                  {v.message}
                  {#if v.text}<span class="rule-text">{v.text}</span>{/if}
                </li>
              {/each}
            </ul>
          {/if}
          <!--
            Declared and not measured. Shown on a passing layout too, and that is the point:
            "passes" means "passes the rules that were checked", and this is the difference
            between the two. It was invisible until 2026-08-31.
          -->
          {#if detail.check.nicht_geprueft.length > 0}
            <p class="unchecked">
              <Icon name="alert" size={14} />
              Declared by this flat, not measured here:
              {detail.check.nicht_geprueft.map((r) => `${r.rule} — ${r.text}`).join(" · ")}
            </p>
          {/if}
          {#if detail.check.uncertainties.length > 0}
            <p class="guessed">
              <Icon name="alert" size={14} />
              Every number above inherits a guess:
              {detail.check.uncertainties.map((u) => `${u.label} (${u.fields.join(", ")})`).join("; ")}
            </p>
          {/if}
        {/if}
      </section>
    </div>
  {/if}
{/if}

{#if !loading && !error && view === "inventory"}
  {#if wishlist}
    <section class="card budget">
      <h2>What is still missing</h2>
      <p class="sum">
        <strong>{euro(wishlist.summe_untere_kante_cent)}</strong>
        {#if wishlist.summe_obere_kante_cent > wishlist.summe_untere_kante_cent}
          <span class="upper">to {euro(wishlist.summe_obere_kante_cent)}</span>
        {/if}
        <span class="over">across {wishlist.items.length} open items</span>
      </p>
      {#if wishlist.posten_ohne_preis > 0}
        <p class="caveat">
          <Icon name="alert" size={14} />
          {wishlist.posten_ohne_preis} of them carry no price at all and count as zero — the
          total is a floor, not an estimate.
        </p>
      {/if}
      {#if wishlist.monatssaldo}
        <p class="finance">
          Finance measures a median monthly balance of
          <strong class:negative={wishlist.monatssaldo.median_cent < 0}>
            {euro(wishlist.monatssaldo.median_cent)}
          </strong>
          over {wishlist.monatssaldo.monate} complete months
          ({wishlist.monatssaldo.von} – {wishlist.monatssaldo.bis}).
          {#if wishlist.monate_bis_bezahlt !== null}
            That is <strong>{wishlist.monate_bis_bezahlt.toFixed(1)} months</strong> of surplus.
          {:else}
            Nothing accrues out of a balance that is not positive, so there is no number of
            months to give.
          {/if}
        </p>
      {:else}
        <p class="caveat">Finance has written nothing on this machine, so there is no balance to measure against.</p>
      {/if}
    </section>

    <h3 class="group">Wanted <span class="count">{wishlist.items.length}</span></h3>
    <ul class="items">
      {#each wishlist.items as i (i.id)}
        <li class="card item">
          <div class="head">
            <span class="label">{i.label}</span>
            {#if i.prioritaet}<span class="tag">{i.prioritaet}</span>{/if}
            {#if i.kind === "slot"}<span class="tag slot">slot</span>{/if}
          </div>
          <span class="dims mono">{size(i)}</span>
          <span class="price mono">
            {#if floorPrice(i) === null}
              <span class="unpriced">no price</span>
            {:else if i.preis_cent !== null}
              {euro(i.preis_cent)}
            {:else}
              {euro(i.kosten_min_cent ?? 0)} – {euro(i.kosten_max_cent ?? 0)}
            {/if}
          </span>
          {#if i.unsicher.length > 0}
            <span class="guess" title="measured? no.">~ {i.unsicher.join(", ")}</span>
          {/if}
          {#if i.ziel}<p class="target">{i.ziel}</p>{/if}
        </li>
      {/each}
    </ul>
  {/if}

  <h3 class="group">Owned <span class="count">{owned.length}</span></h3>
  <ul class="items">
    {#each owned as { item: i } (i.id)}
      <li class="card item">
        <div class="head">
          <span class="label">{i.label}</span>
          {#if i.mitnahme}<span class="tag">{i.mitnahme}</span>{/if}
        </div>
        <span class="dims mono">{size(i)}</span>
        {#if i.unsicher.length > 0}
          <span class="guess" title="measured? no.">~ {i.unsicher.join(", ")}</span>
        {:else if i.gemessen_am}
          <span class="measured mono">measured {i.gemessen_am.slice(0, 10)}</span>
        {/if}
      </li>
    {/each}
  </ul>

  {#if gone.length > 0}
    <h3 class="group">Gone <span class="count">{gone.length}</span></h3>
    <ul class="items">
      {#each gone as { item: i } (i.id)}
        <li class="card item muted">
          <div class="head"><span class="label">{i.label}</span></div>
          <span class="dims mono">{size(i)}</span>
        </li>
      {/each}
    </ul>
  {/if}
{/if}

<style>
  .views {
    display: flex;
    gap: 0.5rem;
    margin: 1rem 0 1.25rem;
  }
  .views button {
    background: var(--card-bg);
    border: 1px solid var(--card-border);
    border-radius: var(--radius-sm);
    color: var(--text-secondary);
    cursor: pointer;
    font-size: 0.8125rem;
    padding: 0.4rem 0.85rem;
  }
  .views button.active {
    border-color: var(--primary);
    color: var(--text-primary);
  }
  .count {
    color: var(--text-tertiary);
    margin-left: 0.35rem;
  }

  .plans {
    display: grid;
    gap: 1.25rem;
    grid-template-columns: minmax(14rem, 20rem) 1fr;
  }
  @media (max-width: 60rem) {
    .plans {
      grid-template-columns: 1fr;
    }
  }

  .variants {
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .variants button {
    background: var(--card-bg);
    border: 1px solid var(--card-border);
    border-radius: var(--radius-sm);
    cursor: pointer;
    display: grid;
    gap: 0.15rem;
    padding: 0.6rem 0.75rem;
    text-align: left;
    width: 100%;
  }
  .variants button.active {
    border-color: var(--primary);
  }
  .variants .name {
    color: var(--text-primary);
    font-size: 0.875rem;
  }
  .variants .tally,
  .measured {
    color: var(--text-tertiary);
    font-size: 0.75rem;
  }
  .verdict {
    color: var(--danger);
    font-size: 0.6875rem;
    letter-spacing: 0.04em;
    text-transform: uppercase;
  }
  .verdict.pass {
    color: var(--success);
  }

  .plan .svg :global(svg) {
    background: var(--surface);
    border: 1px solid var(--card-border);
    border-radius: var(--radius-md);
    height: auto;
    max-width: 100%;
  }

  .metrics {
    display: flex;
    flex-wrap: wrap;
    gap: 1.25rem;
    margin: 0.85rem 0 0;
  }
  .metrics div {
    display: flex;
    gap: 0.4rem;
  }
  .metrics dt {
    color: var(--text-tertiary);
  }
  .metrics dd {
    color: var(--text-primary);
    margin: 0;
  }

  .corridors {
    border-collapse: collapse;
    margin-top: 0.85rem;
    width: 100%;
  }
  .corridors caption {
    color: var(--text-tertiary);
    font-size: 0.75rem;
    padding-bottom: 0.35rem;
    text-align: left;
  }
  .corridors th {
    color: var(--text-secondary);
    font-weight: 400;
    text-align: left;
  }
  .corridors th,
  .corridors td {
    border-top: 1px solid var(--card-border);
    font-size: 0.8125rem;
    padding: 0.3rem 0;
  }
  .corridors td {
    text-align: right;
  }

  .violations {
    display: grid;
    gap: 0.3rem;
    list-style: none;
    margin: 0.85rem 0 0;
    padding: 0;
  }
  .violations li {
    border-left: 2px solid var(--danger);
    color: var(--text-secondary);
    font-size: 0.8125rem;
    padding-left: 0.6rem;
  }
  .violations.soft li {
    border-left-color: var(--warning);
  }
  .rule {
    color: var(--text-tertiary);
    margin-right: 0.4rem;
  }

  /* The rule's own wording, under the violation it explains. Quiet: the reader came for what
     is wrong, and stays for why it counts as wrong. */
  .rule-text {
    color: var(--text-tertiary);
    display: block;
    font-size: 0.75rem;
    margin-top: 0.15rem;
  }

  .guessed,
  .caveat,
  .unchecked {
    align-items: center;
    color: var(--warning);
    display: flex;
    font-size: 0.75rem;
    gap: 0.35rem;
    margin: 0.85rem 0 0;
  }

  .budget h2 {
    font-size: 0.9375rem;
    margin: 0 0 0.5rem;
  }
  .budget .sum {
    align-items: baseline;
    display: flex;
    flex-wrap: wrap;
    gap: 0.5rem;
    margin: 0;
  }
  .budget .sum strong {
    color: var(--text-primary);
    font-size: 1.5rem;
  }
  .upper,
  .over {
    color: var(--text-tertiary);
    font-size: 0.8125rem;
  }
  .finance {
    color: var(--text-secondary);
    font-size: 0.8125rem;
    margin: 0.75rem 0 0;
  }
  .finance strong {
    color: var(--success);
  }
  .finance strong.negative {
    color: var(--danger);
  }

  .group {
    align-items: baseline;
    display: flex;
    font-size: 0.75rem;
    gap: 0.35rem;
    letter-spacing: 0.06em;
    margin: 1.5rem 0 0.6rem;
    text-transform: uppercase;
  }

  .items {
    display: grid;
    gap: 0.5rem;
    grid-template-columns: repeat(auto-fill, minmax(16rem, 1fr));
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .item {
    display: grid;
    gap: 0.25rem;
    padding: 0.7rem 0.85rem;
  }
  .item.muted {
    opacity: 0.55;
  }
  .item .head {
    align-items: baseline;
    display: flex;
    flex-wrap: wrap;
    gap: 0.4rem;
  }
  .item .label {
    color: var(--text-primary);
    font-size: 0.875rem;
  }
  .tag {
    background: var(--primary-soft);
    border-radius: var(--radius-sm);
    color: var(--text-secondary);
    font-size: 0.6875rem;
    padding: 0.05rem 0.35rem;
  }
  .tag.slot {
    background: var(--warning-soft);
  }
  .dims,
  .price {
    color: var(--text-secondary);
    font-size: 0.8125rem;
  }
  .unpriced {
    color: var(--warning);
  }
  .guess {
    color: var(--warning);
    font-size: 0.75rem;
  }
  .target {
    color: var(--text-tertiary);
    font-size: 0.75rem;
    margin: 0.25rem 0 0;
  }

  .card {
    background: var(--card-bg);
    border: 1px solid var(--card-border);
    border-radius: var(--radius-md);
  }
  .offer,
  .budget {
    padding: 1rem 1.15rem;
  }
  .offer .lead {
    align-items: center;
    color: var(--danger);
    display: flex;
    gap: 0.4rem;
    margin: 0 0 0.75rem;
  }
  .offer button {
    background: var(--primary);
    border: none;
    border-radius: var(--radius-sm);
    color: white;
    cursor: pointer;
    font-size: 0.8125rem;
    padding: 0.45rem 0.9rem;
  }
  .empty,
  .error {
    color: var(--text-tertiary);
    font-size: 0.8125rem;
  }
  .error {
    align-items: center;
    color: var(--danger);
    display: flex;
    gap: 0.4rem;
  }
  .mono {
    font-family: var(--font-mono);
  }
</style>
