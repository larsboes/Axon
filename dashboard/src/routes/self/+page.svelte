<script lang="ts">
  import { onMount } from "svelte";
  import Icon from "$lib/Icon.svelte";
  import PageHeader from "$lib/PageHeader.svelte";
  import { axonStatus, type SelfModelResponse, type SelfUnit } from "$lib/api";
  import RepoStatusCard from "$lib/RepoStatusCard.svelte";

  // Deliberately no graph-rendering dependency. The interesting relation here is
  // 16 coupling pairs over ~31 units — small enough that a selected unit plus its
  // in/out edges reads better than a force layout, and it costs no bundle. A real
  // node graph is worth adding the day file-level drill-down is wanted, not before.

  let data = $state<SelfModelResponse | null>(null);
  let error = $state<string | null>(null);
  let selected = $state<string | null>(null);

  onMount(() => {
    axonStatus
      .self()
      .then((d) => (data = d))
      .catch((e) => (error = e instanceof Error ? e.message : String(e)));
  });

  const model = $derived(data?.model ?? null);
  const live = $derived(data?.live ?? {});

  /** Outgoing = what this unit compiles in. Incoming = who compiles it in. */
  const outgoing = $derived(
    selected && model ? model.coupling.filter((c) => c.from === selected) : [],
  );
  const incoming = $derived(
    selected && model ? model.coupling.filter((c) => c.to === selected) : [],
  );

  const related = (name: string): boolean =>
    !selected ||
    selected === name ||
    outgoing.some((c) => c.to === name) ||
    incoming.some((c) => c.from === name);

  const detail = $derived(
    selected && model ? (model.units.find((u) => u.name === selected) ?? null) : null,
  );

  function upClass(name: string): string {
    const up = live[name];
    if (up === true) return "up";
    if (up === false) return "down";
    return "unknown";
  }

  function upLabel(name: string): string {
    const up = live[name];
    if (up === true) return "running";
    if (up === false) return "off";
    return "no health check";
  }

  function toggle(u: SelfUnit) {
    selected = selected === u.name ? null : u.name;
  }
</script>

<PageHeader
  badge="Self-model"
  title="Axon about Axon"
  desc="What is here, how the units connect, and how large each one is. Generated from tracked files and supplemented with live state."
/>

<!--
  Version identity belongs on the self-model page for the same reason the rest of it
  does: it is a fact about this repo, not about this machine. The home page carries the
  same card in its compact form, so the link is one glance away without this page.
-->
<RepoStatusCard detailed />

{#if error}
  <p class="err">
    <Icon name="alert" size={14} /> axon-status unavailable — {error}
  </p>
{:else if !model}
  <p class="loading"><Icon name="loader" size={14} /> loading…</p>
{:else}
  <ul class="stats">
    <li><b>{model.units.length}</b><span>Units</span></li>
    <li><b>{model.coupling.length}</b><span>Couplings</span></li>
    <li><b>{model.graph.present ? model.graph.nodes : "—"}</b><span>Graph nodes</span></li>
    <li><b>{model.upstreams.length}</b><span>Upstreams</span></li>
  </ul>

  {#if model.graph.stale.length}
    <p class="err">
      <Icon name="alert" size={14} />
      {model.graph.stale.length} graph paths no longer exist — <span class="mono"
        >tools/graphify.sh</span
      >
    </p>
  {/if}

  {#if selected && detail}
    <div class="card detail">
      <header>
        <strong>{detail.name}</strong>
        <span class="kind">{detail.kind}</span>
        <button onclick={() => (selected = null)} aria-label="Clear selection">
          <Icon name="close" size={13} />
        </button>
      </header>
      <dl>
        {#if detail.code}
          <div><dt>Code</dt><dd>{detail.code.files} files · {detail.code.nodes} nodes</dd></div>
        {/if}
        {#if detail.service}
          <div>
            <dt>Service</dt>
            <dd>
              {detail.service.kind}{detail.service.port ? ` · Port ${detail.service.port}` : ""}
            </dd>
          </div>
          <div>
            <dt>runtime dependencies</dt>
            <dd>{detail.service.requires.length ? detail.service.requires.join(", ") : "—"}</dd>
          </div>
        {/if}
        <div>
          <dt>compiles in</dt>
          <dd>{outgoing.length ? outgoing.map((c) => c.to).join(", ") : "—"}</dd>
        </div>
        <div>
          <dt>compiled into</dt>
          <dd>{incoming.length ? incoming.map((c) => c.from).join(", ") : "—"}</dd>
        </div>
      </dl>
      {#if outgoing.length}
        <ul class="evidence">
          {#each outgoing as c (c.to)}
            <li>
              <span class="mono">{c.to}</span>
              {#each c.kinds as k (k)}<span class="tag">{k}</span>{/each}
              <span class="mono files">{c.evidence.join(" · ")}</span>
            </li>
          {/each}
        </ul>
      {/if}
    </div>
  {/if}

  <p class="hint">
    Select a row to see its couplings. <b>runtime dependencies</b> come from
    <span class="mono">requires=</span>; <b>compiles in</b> comes from
    <span class="mono">#[path]</span> or a Bazel label. They are different relationships.
  </p>

  <div class="wrap">
    <table>
      <thead>
        <tr>
          <th></th>
          <th>Unit</th>
          <th>Kind</th>
          <th class="num">Files</th>
          <th class="num">Port</th>
          <th>Runtime dependencies</th>
          <th class="num">Couplings</th>
        </tr>
      </thead>
      <tbody>
        {#each model.units as u (u.name)}
          {@const deg = model.coupling.filter((c) => c.from === u.name || c.to === u.name).length}
          <tr
            class:dim={!related(u.name)}
            class:sel={selected === u.name}
            onclick={() => toggle(u)}
          >
            <td>
              <span class="dot {upClass(u.name)}" title={upLabel(u.name)}></span>
            </td>
            <td class="mono name">{u.name}</td>
            <td><span class="kind">{u.kind}</span></td>
            <td class="num">{u.code?.files ?? "—"}</td>
            <td class="num mono">{u.service?.port ?? "—"}</td>
            <td class="req">{u.service?.requires.length ? u.service.requires.join(", ") : "—"}</td>
            <td class="num">{deg || "—"}</td>
          </tr>
        {/each}
      </tbody>
    </table>
  </div>
{/if}

<style>
  .stats {
    display: flex;
    flex-wrap: wrap;
    gap: 0.5rem;
    list-style: none;
    padding: 0;
    margin: 0 0 1rem;
  }
  .stats li {
    display: flex;
    flex-direction: column;
    gap: 0.1rem;
    padding: 0.55rem 0.9rem;
    border: 1px solid var(--line, #2a2a3e);
    border-radius: 8px;
    min-width: 6.5rem;
  }
  .stats b {
    font-size: 1.25rem;
    font-variant-numeric: tabular-nums;
  }
  .stats span {
    font-size: 0.72rem;
    opacity: 0.6;
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }

  .hint {
    font-size: 0.8rem;
    opacity: 0.65;
    margin: 0 0 0.6rem;
    line-height: 1.5;
  }

  .err,
  .loading {
    display: flex;
    align-items: center;
    gap: 0.45rem;
    font-size: 0.85rem;
    margin: 0 0 0.9rem;
  }
  .err {
    color: var(--warn, #e0a458);
  }

  /*
   * min-width: 0 guards the flex-item trap: `main` is a flex item whose default
   * min-width: auto refuses to shrink below its content, which would let a table of
   * nowrap columns widen the page instead of scrolling inside its own box. Kept as a
   * guard, not a fix for an observed bug — measured scrollWidth equals clientWidth at
   * this viewport, so the table already fits. It earns its place at narrower widths.
   */
  .wrap {
    overflow-x: auto;
    min-width: 0;
    max-width: 100%;
  }
  table {
    width: 100%;
    border-collapse: collapse;
    font-size: 0.84rem;
  }
  th {
    text-align: left;
    font-weight: 500;
    font-size: 0.7rem;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    opacity: 0.55;
    padding: 0 0.55rem 0.4rem;
    white-space: nowrap;
  }
  td {
    padding: 0.42rem 0.55rem;
    border-top: 1px solid var(--line, #2a2a3e);
    white-space: nowrap;
  }
  tbody tr {
    cursor: pointer;
  }
  tbody tr:hover td {
    background: var(--hover, rgba(255, 255, 255, 0.04));
  }
  tr.dim {
    opacity: 0.28;
  }
  tr.sel td {
    background: var(--hover, rgba(255, 255, 255, 0.07));
  }
  .num {
    text-align: right;
    font-variant-numeric: tabular-nums;
  }
  .name {
    font-weight: 500;
  }
  .req {
    opacity: 0.75;
  }

  .kind {
    font-size: 0.68rem;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    opacity: 0.6;
  }

  .dot {
    display: inline-block;
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--muted, #555);
  }
  .dot.up {
    background: var(--ok, #4ba36a);
  }
  .dot.down {
    background: var(--warn, #e0a458);
  }

  .detail {
    padding: 0.9rem 1rem;
    margin: 0 0 0.9rem;
    min-width: 0;
    max-width: 100%;
  }
  .detail header {
    display: flex;
    align-items: center;
    gap: 0.55rem;
    margin-bottom: 0.6rem;
  }
  .detail header button {
    margin-left: auto;
    background: none;
    border: none;
    color: inherit;
    cursor: pointer;
    opacity: 0.6;
    padding: 0.2rem;
    display: flex;
  }
  .detail header button:hover {
    opacity: 1;
  }
  .detail dl {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(11rem, 1fr));
    gap: 0.5rem 1rem;
    margin: 0;
  }
  .detail dt {
    font-size: 0.68rem;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    opacity: 0.55;
  }
  .detail dd {
    margin: 0.1rem 0 0;
    font-size: 0.84rem;
  }

  .evidence {
    list-style: none;
    padding: 0.7rem 0 0;
    margin: 0.7rem 0 0;
    border-top: 1px solid var(--line, #2a2a3e);
    display: flex;
    flex-direction: column;
    gap: 0.3rem;
  }
  .evidence li {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    flex-wrap: wrap;
    font-size: 0.78rem;
  }
  .evidence .files {
    opacity: 0.5;
    font-size: 0.72rem;
    /* Repo-relative paths are long and unbreakable; let them break rather than widen
       the page. Same reason as .wrap's min-width. */
    overflow-wrap: anywhere;
  }
  .tag {
    font-size: 0.65rem;
    padding: 0.05rem 0.35rem;
    border: 1px solid var(--line, #2a2a3e);
    border-radius: 4px;
    opacity: 0.75;
  }
</style>
