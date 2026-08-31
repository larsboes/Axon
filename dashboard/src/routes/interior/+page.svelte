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
    type InteriorImpact,
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

  // ─── Editing ───────────────────────────────────────────────────────────────
  //
  // Every write goes through the capability. The page holds no rule and computes no verdict:
  // even the impact preview below is the capability re-checking every layout, not the browser
  // guessing. A second implementation of a clearance rule is the drift this whole thing exists
  // to prevent (PRD B27).

  let editing = $state<string | null>(null);
  /** Form values are strings while they are being typed; `patchFrom` converts on the way out. */
  type Draft = {
    label: string;
    b: string | number;
    t: string | number;
    h: string | number;
    preis_cent: string | number;
    prioritaet: string;
    hinweis: string;
    begruendung: string;
    opens: string;
    open_clear: string | number;
    wall_ok: string;
    access_sides: string | number;
    access_clear: string | number;
    raumtrenner: boolean;
  };
  let draft = $state<Draft>({
    label: "", b: "", t: "", h: "", preis_cent: "", prioritaet: "", hinweis: "",
    begruendung: "", opens: "", open_clear: "", wall_ok: "", access_sides: "",
    access_clear: "", raumtrenner: false,
  });
  let impact = $state<InteriorImpact | null>(null);
  let impactBusy = $state(false);
  let saving = $state(false);
  let saveError = $state<string | null>(null);
  let history = $state<{ state: InteriorState; since: string; note: string | null }[]>([]);
  let creating = $state(false);
  let draftNew = $state({ id: "", label: "", kind: "piece" as "piece" | "slot", state: "wanted" as InteriorState, b: "", t: "", h: "", preis: "" });

  const byId = $derived(new Map(inventory.map((r) => [r.item.id, r])));

  /** Which Q61 fields a piece fills in, if any. Empty means the name heuristic still judges it. */
  const declares = (i: InteriorItem): string =>
    [
      i.opens !== null && i.opens !== undefined ? "opens" : null,
      i.open_clear !== null && i.open_clear !== undefined ? "open_clear" : null,
      i.expands_to !== null && i.expands_to !== undefined ? "expands" : null,
      i.access_sides !== null && i.access_sides !== undefined ? "access" : null,
      i.raumtrenner ? "raumtrenner" : null,
    ]
      .filter(Boolean)
      .join(" · ");

  /** cm and € come out of inputs as strings; empty means "clear it", not "zero". */
  const num = (v: unknown): number | null => {
    const t = String(v ?? "").trim();
    if (t === "") return null;
    const n = Number(t);
    return Number.isFinite(n) ? n : null;
  };

  async function openEditor(id: string): Promise<void> {
    const row = byId.get(id);
    if (!row) return;
    editing = id;
    saveError = null;
    impact = null;
    const i = row.item;
    draft = {
      label: i.label,
      b: i.b ?? "",
      t: i.t ?? "",
      h: i.h ?? "",
      preis_cent: i.preis_cent ?? "",
      prioritaet: i.prioritaet ?? "",
      hinweis: i.hinweis ?? "",
      begruendung: i.begruendung ?? "",
      opens: i.opens ?? "",
      open_clear: i.open_clear ?? "",
      wall_ok: i.wall_ok === null ? "" : String(i.wall_ok),
      access_sides: i.access_sides ?? "",
      access_clear: i.access_clear ?? "",
      raumtrenner: i.raumtrenner ?? false,
    };
    history = [];
    try {
      history = await interior.stateHistory(id);
    } catch {
      // A missing history is not a reason to refuse the edit; the panel says so itself.
    }
  }

  /** Only the fields that actually differ. Sending the unchanged ones would work, but a diff
      that names exactly what moved is what makes the impact preview readable. */
  function patchFrom(id: string): Record<string, unknown> {
    const i = byId.get(id)?.item;
    if (!i) return {};
    const out: Record<string, unknown> = {};
    const put = (k: string, v: unknown, was: unknown) => {
      if (v !== was) out[k] = v;
    };
    put("label", String(draft.label ?? "").trim(), i.label);
    put("b", num(draft.b), i.b);
    put("t", num(draft.t), i.t);
    put("h", num(draft.h), i.h);
    put("preis_cent", num(draft.preis_cent), i.preis_cent);
    put("prioritaet", String(draft.prioritaet ?? "").trim() || null, i.prioritaet);
    put("hinweis", String(draft.hinweis ?? "").trim() || null, i.hinweis);
    put("begruendung", String(draft.begruendung ?? "").trim() || null, i.begruendung);
    put("opens", String(draft.opens ?? "") || null, i.opens);
    put("open_clear", num(draft.open_clear), i.open_clear);
    put("wall_ok", draft.wall_ok === "" ? null : draft.wall_ok === "true", i.wall_ok);
    put("access_sides", num(draft.access_sides), i.access_sides);
    put("access_clear", num(draft.access_clear), i.access_clear);
    put("raumtrenner", draft.raumtrenner ? true : null, i.raumtrenner);
    return out;
  }

  async function preview(): Promise<void> {
    if (editing === null) return;
    const patch = patchFrom(editing);
    if (Object.keys(patch).length === 0) {
      impact = null;
      return;
    }
    impactBusy = true;
    saveError = null;
    try {
      impact = await interior.impact(editing, patch);
    } catch (caught) {
      saveError = caught instanceof Error ? caught.message : String(caught);
    } finally {
      impactBusy = false;
    }
  }

  async function save(): Promise<void> {
    if (editing === null) return;
    const patch = patchFrom(editing);
    if (Object.keys(patch).length === 0) {
      editing = null;
      return;
    }
    saving = true;
    saveError = null;
    try {
      await interior.patchItem(editing, patch);
      editing = null;
      impact = null;
      await load();
    } catch (caught) {
      saveError = caught instanceof Error ? caught.message : String(caught);
    } finally {
      saving = false;
    }
  }

  async function move(id: string, to: InteriorState): Promise<void> {
    saving = true;
    saveError = null;
    try {
      await interior.setState(id, to);
      if (editing === id) history = await interior.stateHistory(id);
      await load();
    } catch (caught) {
      saveError = caught instanceof Error ? caught.message : String(caught);
    } finally {
      saving = false;
    }
  }

  async function create(): Promise<void> {
    saving = true;
    saveError = null;
    try {
      await interior.createItem({
        id: draftNew.id.trim(),
        kind: draftNew.kind,
        label: draftNew.label.trim(),
        state: draftNew.state,
        b: num(draftNew.b),
        t: num(draftNew.t),
        h: num(draftNew.h),
        preis_cent: num(draftNew.preis),
      });
      creating = false;
      draftNew = { id: "", label: "", kind: "piece", state: "wanted", b: "", t: "", h: "", preis: "" };
      await load();
    } catch (caught) {
      saveError = caught instanceof Error ? caught.message : String(caught);
    } finally {
      saving = false;
    }
  }

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
          <div class="actions">
            <button class="ghost" onclick={() => openEditor(i.id)}>Edit</button>
            <button class="ghost" disabled={saving} onclick={() => move(i.id, "owned")}>
              Mark bought
            </button>
          </div>
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
        {#if declares(i)}
          <span class="declares mono" title="This piece states what it needs, so the name heuristic does not run for it (PRD Q61).">
            declares {declares(i)}
          </span>
        {/if}
        <div class="actions">
          <button class="ghost" onclick={() => openEditor(i.id)}>Edit</button>
          <button class="ghost" disabled={saving} onclick={() => move(i.id, "gone")}>
            Given away
          </button>
        </div>
      </li>
    {/each}
  </ul>

  {#if editing !== null}
    {@const row = byId.get(editing)}
    <div class="card editor">
      <h4>{row?.item.label ?? editing} <span class="mono id">{editing}</span></h4>

      {#if saveError}<p class="err">{saveError}</p>{/if}

      <div class="grid">
        <label>label <input bind:value={draft.label} /></label>
        <label>width cm <input bind:value={draft.b} inputmode="numeric" /></label>
        <label>depth cm <input bind:value={draft.t} inputmode="numeric" /></label>
        <label>height cm <input bind:value={draft.h} inputmode="numeric" /></label>
        <label>price ct <input bind:value={draft.preis_cent} inputmode="numeric" /></label>
        <label>priority <input bind:value={draft.prioritaet} /></label>
      </div>

      <label class="wide">note <textarea rows="2" bind:value={draft.hinweis}></textarea></label>
      <label class="wide">reasoning <textarea rows="2" bind:value={draft.begruendung}></textarea></label>

      <h5>What this piece needs</h5>
      <p class="note">
        Fill any of these and the name heuristic stops judging this piece — it is checked against
        exactly what it states and nothing else (PRD Q61). Leave them empty and it is still
        measured by its name.
      </p>
      <div class="grid">
        <label>
          opens
          <select bind:value={draft.opens}>
            <option value="">— not stated —</option>
            <option value="nord">nord</option>
            <option value="sued">sued</option>
            <option value="ost">ost</option>
            <option value="west">west</option>
          </select>
        </label>
        <label>open_clear cm <input bind:value={draft.open_clear} inputmode="numeric" /></label>
        <label>
          wall_ok
          <select bind:value={draft.wall_ok}>
            <option value="">— not stated —</option>
            <option value="true">true — may sit against a wall</option>
            <option value="false">false — that side is useless there</option>
          </select>
        </label>
        <label>access_sides <input bind:value={draft.access_sides} inputmode="numeric" /></label>
        <label>access_clear cm <input bind:value={draft.access_clear} inputmode="numeric" /></label>
        <label class="check">
          <input type="checkbox" bind:checked={draft.raumtrenner} />
          raumtrenner — meant to stand free
        </label>
      </div>

      <div class="actions">
        <button class="ghost" disabled={impactBusy} onclick={preview}>
          {impactBusy ? "Checking…" : "Check impact"}
        </button>
        <button disabled={saving} onclick={save}>{saving ? "Saving…" : "Save"}</button>
        <button class="ghost" onclick={() => { editing = null; impact = null; }}>Close</button>
      </div>

      <!--
        The reason this is a button and not a surprise. Declaring which side the wardrobe opens
        costs 2 or 4 layouts depending on the direction, and nothing said so until it was worked
        out by hand. The capability re-checks every layout here; the page only prints the answer.
      -->
      {#if impact}
        <div class="impact" class:worse={impact.bestanden_nachher < impact.bestanden_vorher}>
          <strong>
            {impact.bestanden_vorher} → {impact.bestanden_nachher} of {impact.layouts} layouts pass
          </strong>
          {#if impact.geaendert.length === 0}
            <p>No verdict moves. Safe to save.</p>
          {:else}
            <ul>
              {#each impact.geaendert as g (g.layout)}
                <li>
                  <span class="mono">{g.layout}</span>
                  {g.vorher.pass ? "passes" : "fails"} → {g.nachher.pass ? "passes" : "fails"}
                  {#if g.nachher.hard.length > 0}
                    <span class="mono rules">{g.nachher.hard.join(", ")}</span>
                  {/if}
                </li>
              {/each}
            </ul>
          {/if}
        </div>
      {/if}

      <h5>State history</h5>
      {#if history.length === 0}
        <p class="note">No history recorded.</p>
      {:else}
        <ul class="history">
          {#each history as h, n (h.since + n)}
            <li>
              <span class="mono st">{h.state}</span>
              <span class="mono when">{h.since.slice(0, 10)}</span>
              {#if h.note}<span class="why">{h.note}</span>{/if}
            </li>
          {/each}
        </ul>
        <p class="note">
          Appended, never overwritten — a wish that gets bought is a second row, and that span is
          what the wishlist joins to money with.
        </p>
      {/if}
    </div>
  {/if}

  <div class="addrow">
    <button class="ghost" onclick={() => (creating = !creating)}>
      {creating ? "Cancel" : "+ New entry"}
    </button>
  </div>

  {#if creating}
    <form class="card editor" onsubmit={(e) => { e.preventDefault(); create(); }}>
      <h4>New entry</h4>
      <div class="grid">
        <label>id <input bind:value={draftNew.id} placeholder="stehlampe" required /></label>
        <label>label <input bind:value={draftNew.label} placeholder="IKEA …" required /></label>
        <label>
          kind
          <select bind:value={draftNew.kind}>
            <option value="piece">piece — a thing, or a specific product</option>
            <option value="slot">slot — a need with target sizes, no product yet</option>
          </select>
        </label>
        <label>
          state
          <select bind:value={draftNew.state}>
            <option value="wanted">wanted</option>
            <option value="owned">owned</option>
          </select>
        </label>
        <label>width cm <input bind:value={draftNew.b} inputmode="numeric" /></label>
        <label>depth cm <input bind:value={draftNew.t} inputmode="numeric" /></label>
        <label>height cm <input bind:value={draftNew.h} inputmode="numeric" /></label>
        <label>price ct <input bind:value={draftNew.preis} inputmode="numeric" /></label>
      </div>
      <p class="note">
        State is required, not a default — an entry with no state joins to nothing and would
        never appear in any list.
      </p>
      <div class="actions">
        <button type="submit" disabled={saving || !draftNew.id.trim() || !draftNew.label.trim()}>
          {saving ? "Creating…" : "Create"}
        </button>
      </div>
    </form>
  {/if}

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

  /* ─── Editing ─────────────────────────────────────────────────────────── */

  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: 0.4rem;
    margin-top: 0.6rem;
  }

  .actions button {
    background: var(--primary);
    border: 1px solid transparent;
    border-radius: 6px;
    color: var(--bg);
    cursor: pointer;
    font: inherit;
    font-size: 0.75rem;
    padding: 0.3rem 0.7rem;
  }

  .actions button.ghost {
    background: transparent;
    border-color: var(--border);
    color: var(--text-secondary);
  }

  .actions button:disabled {
    cursor: not-allowed;
    opacity: 0.5;
  }

  .addrow {
    margin: 1rem 0;
  }

  .addrow button {
    background: transparent;
    border: 1px dashed var(--border);
    border-radius: 6px;
    color: var(--text-secondary);
    cursor: pointer;
    font: inherit;
    font-size: 0.8rem;
    padding: 0.45rem 0.9rem;
  }

  .editor {
    margin: 1rem 0 1.5rem;
    padding: 1rem;
  }

  .editor h4 {
    align-items: baseline;
    display: flex;
    gap: 0.5rem;
    margin: 0 0 0.75rem;
  }

  .editor h5 {
    margin: 1.1rem 0 0.3rem;
  }

  .editor .id {
    color: var(--text-tertiary);
    font-size: 0.75rem;
    font-weight: 400;
  }

  .editor .grid {
    display: grid;
    gap: 0.6rem;
    grid-template-columns: repeat(auto-fit, minmax(11rem, 1fr));
  }

  .editor label {
    color: var(--text-tertiary);
    display: flex;
    flex-direction: column;
    font-size: 0.7rem;
    gap: 0.2rem;
    text-transform: uppercase;
  }

  .editor label.wide {
    margin-top: 0.6rem;
  }

  .editor label.check {
    align-items: center;
    flex-direction: row;
    text-transform: none;
  }

  .editor input,
  .editor select,
  .editor textarea {
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: 5px;
    color: var(--text);
    font: inherit;
    font-size: 0.8rem;
    padding: 0.35rem 0.45rem;
    text-transform: none;
    width: 100%;
  }

  .editor .note {
    color: var(--text-tertiary);
    font-size: 0.72rem;
    margin: 0.35rem 0 0;
  }

  .editor .err {
    color: var(--danger, #f87171);
    font-size: 0.78rem;
    margin: 0 0 0.6rem;
  }

  /* The arithmetic that used to have to be done by hand, one direction at a time. */
  .impact {
    background: var(--bg);
    border: 1px solid var(--border);
    border-left: 3px solid var(--primary);
    border-radius: 6px;
    font-size: 0.78rem;
    margin-top: 0.85rem;
    padding: 0.6rem 0.75rem;
  }

  .impact.worse {
    border-left-color: var(--warning);
  }

  .impact ul,
  .history {
    list-style: none;
    margin: 0.4rem 0 0;
    padding: 0;
  }

  .impact li,
  .history li {
    display: flex;
    flex-wrap: wrap;
    gap: 0.45rem;
    padding: 0.15rem 0;
  }

  .impact .rules {
    color: var(--warning);
  }

  .history {
    font-size: 0.78rem;
  }

  .history .st {
    color: var(--primary);
    min-width: 4.5rem;
  }

  .history .when {
    color: var(--text-tertiary);
  }

  .history .why {
    color: var(--text-secondary);
  }

  .declares {
    color: var(--primary);
    font-size: 0.7rem;
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
