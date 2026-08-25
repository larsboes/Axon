<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import Icon from "$lib/Icon.svelte";
  import PageHeader from "$lib/PageHeader.svelte";
  import { macmon, type MacmonSample } from "$lib/api";

  let macmonState = $state<"checking" | "up" | "down">("checking");
  let sample = $state<MacmonSample | null>(null);
  let error = $state<string | null>(null);
  let pollTimer: ReturnType<typeof setInterval> | undefined;
  let topProcs = $state<Array<{ pid: number; rss_mb: number; name: string }>>([]);
  let procErr = $state(false);

  /** °C → CSS class name */
  function tempClass(celsius: number): string {
    if (celsius >= 80) return "hot";
    if (celsius >= 60) return "warm";
    return "cool";
  }

  /** Bytes → human-readable */
  function bytes(gb: number): string {
    return (gb / 1024 / 1024 / 1024).toFixed(1) + " GB";
  }

  /** Fraction 0-1 → % */
  function pct(v: number): string {
    return (v * 100).toFixed(1) + "%";
  }

  /** Power in watts */
  function watts(v: number): string {
    return v.toFixed(2) + " W";
  }

  /** Sum of top processes' RSS for the "bekannt" total. */
  const topRssTotal = $derived(topProcs.reduce((s, p) => s + p.rss_mb, 0));

  onMount(() => {
    // Fetch top memory consumers once; cold data is fine on this page since macmon
    // gives the live totals and these names change slowly.
    fetch("/api/top-processes")
      .then((r) => { if (r.ok) return r.json(); throw new Error(); })
      .then((d) => { topProcs = d; procErr = false; })
      .catch(() => { procErr = true; });

    const poll = () => {
      macmon
        .json()
        .then((d) => {
          sample = d;
          macmonState = "up";
          error = null;
        })
        .catch((e) => {
          macmonState = "down";
          error = e instanceof Error ? e.message : String(e);
        });
    };

    poll();
    pollTimer = setInterval(poll, 3_000);
  });

  onDestroy(() => {
    if (pollTimer) clearInterval(pollTimer);
  });
</script>

<PageHeader
  badge="Local machine"
  title="Systems"
  desc="What this computer is doing now: temperature, power, and utilisation."
/>

<!-- ─── macmon dashboard ──────────────────────────────────────────────────── -->

{#if macmonState === "checking"}
  <p class="loading"><Icon name="loader" size={14} /> collecting data…</p>
{:else if macmonState === "down"}
  <div class="card err-card">
    <p class="err">
      <Icon name="alert" size={14} />
      macmon unavailable
    </p>
    <p class="err-hint">
      Start <code class="mono">macmon serve --port 9911</code> to show live metrics on
      the Systems page.
    </p>
    {#if error}
      <p class="err-detail mono">{error}</p>
    {/if}
  </div>
{:else if sample}
  <div class="grid">
    <!-- Temperatur -->
    <div class="card metric">
      <span class="metric-head">
        <Icon name="thermometer" size={14} />
        Temperature
      </span>
      <div class="metric-body">
        <div class="temp-row">
          <span class="temp-value {tempClass(sample.temp.cpu_temp_avg)}">
            {sample.temp.cpu_temp_avg.toFixed(0)}°
          </span>
          <span class="temp-label">CPU</span>
        </div>
        <div class="temp-row">
          <span class="temp-value {tempClass(sample.temp.gpu_temp_avg)}">
            {sample.temp.gpu_temp_avg.toFixed(0)}°
          </span>
          <span class="temp-label">GPU</span>
        </div>
      </div>
    </div>

    <!-- Leistungsaufnahme -->
    <div class="card metric">
      <span class="metric-head">
        <Icon name="activity" size={14} />
        Power
      </span>
      <div class="metric-body watts">
        <div class="watts-total">{watts(sample.all_power)}</div>
        <div class="watts-breakdown">
          <span title="CPU">CPU {watts(sample.cpu_power)}</span>
          <span title="GPU">GPU {watts(sample.gpu_power)}</span>
          <span title="RAM">RAM {watts(sample.ram_power)}</span>
          <span title="System (remainder)">Sys {watts(sample.sys_power)}</span>
        </div>
      </div>
    </div>

    <!-- CPU Auslastung -->
    <div class="card metric">
      <span class="metric-head">
        <Icon name="cpu" size={14} />
        CPU
      </span>
      <div class="metric-body">
        <div class="usage-row">
          <span class="usage-pct">{pct(sample.cpu_usage_pct)}</span>
          <span class="usage-label">total</span>
        </div>
        <div class="bar-track">
          <div class="bar-fill" style="width: {sample.cpu_usage_pct * 100}%"></div>
        </div>
        <div class="core-row">
          <span>P-Cores <b>{sample.pcpu_usage[0]} MHz</b> {pct(sample.pcpu_usage[1])}</span>
          <span>E-Cores <b>{sample.ecpu_usage[0]} MHz</b> {pct(sample.ecpu_usage[1])}</span>
        </div>
      </div>
    </div>

    <!-- Memory, with hover detail for swap consumers. -->
    <div class="card metric mem-card">
      <span class="metric-head">
        <Icon name="database" size={14} />
        Memory
        <span class="mem-pressure" title="Swap use as an indicator of memory pressure">
          {#if sample.memory.swap_usage > sample.memory.swap_total * 0.5}
            <Icon name="alert" size={11} />
          {/if}
        </span>
      </span>
      <div class="metric-body">
        <div class="mem-row">
          <span>RAM</span>
          <span class="mono">{bytes(sample.memory.ram_usage)} / {bytes(sample.memory.ram_total)}</span>
        </div>
        <div class="bar-track">
          <div class="bar-fill mem" style="width: {sample.memory.ram_usage / sample.memory.ram_total * 100}%"></div>
        </div>
        <div class="mem-row swap">
          <span>Swap</span>
          <span class="mono">{bytes(sample.memory.swap_usage)} / {bytes(sample.memory.swap_total)}</span>
        </div>
        <div class="bar-track">
          <div class="bar-fill swap" style="width: {sample.memory.swap_usage / sample.memory.swap_total * 100}%"></div>
        </div>
      </div>

      <!-- Hover popover: what is consuming memory. -->
      <div class="mem-hover">
        <div class="mem-hover-head">
          <strong>Largest RAM users</strong>
          <span class="mono">{topRssTotal} MB across the largest processes</span>
        </div>
        {#if procErr}
          <p class="mem-hint">Process list unavailable</p>
        {:else if topProcs.length === 0}
          <p class="mem-hint">loading…</p>
        {:else}
          <ol class="proc-list">
            {#each topProcs.slice(0, 8) as p (p.pid)}
              <li>
                <span class="proc-name">{p.name}</span>
                <span class="proc-bar-wrap">
                  <span class="proc-bar" style="width:{Math.min(100, p.rss_mb / 8)}%"></span>
                </span>
                <span class="proc-rss mono">{p.rss_mb} MB</span>
              </li>
            {/each}
          </ol>
          {#if topProcs.length > 8}
            <p class="mem-hint">+ {topProcs.length - 8} more processes above 100 MB</p>
          {/if}
          <p class="mem-hint warn">
            <Icon name="alert" size={10} />
            macOS has used {bytes(sample.memory.swap_usage)} of
            {bytes(sample.memory.swap_total)} swap. {topRssTotal >= 6000
              ? "Active processes no longer fit in RAM."
              : "The system is compressing and moving rarely used pages to swap."}
          </p>
        {/if}
      </div>
    </div>
  </div>

  <p class="ts mono">
    <Icon name="clock" size={12} />
    Last updated: {new Date(sample.timestamp).toLocaleTimeString("en-GB")}
    (every 3 s)
  </p>
{/if}

<style>
  /* ── Loading / error ────────────────────────────────────────── */
  .loading {
    display: flex;
    align-items: center;
    gap: 0.45rem;
    font-size: 0.85rem;
    margin: 0 0 0.9rem;
  }

  .err-card {
    padding: 0.85rem 1rem;
    margin: 0 0 1rem;
  }

  .err {
    display: flex;
    align-items: center;
    gap: 0.45rem;
    font-size: 0.85rem;
    margin: 0 0 0.3rem;
    color: var(--warning);
  }

  .err-hint {
    font-size: 0.75rem;
    color: var(--text-secondary);
    margin: 0 0 0.15rem;
  }

  .err-detail {
    font-size: 0.7rem;
    color: var(--text-tertiary);
    margin: 0;
  }

  /* ── Metric grid ────────────────────────────────────────────── */
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(17rem, 1fr));
    gap: 0.75rem;
    margin: 0 0 0.5rem;
  }

  .metric {
    padding: 0.85rem 1rem;
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }

  .metric-head {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    font-size: 0.6875rem;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-secondary);
  }

  .metric-body {
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
  }

  /* ── Temperature ────────────────────────────────────────────── */
  .temp-row {
    display: flex;
    align-items: baseline;
    gap: 0.5rem;
  }

  .temp-value {
    font-size: 1.5rem;
    font-weight: 700;
    font-variant-numeric: tabular-nums;
    line-height: 1;
  }

  .temp-value.cool {
    color: var(--success);
  }

  .temp-value.warm {
    color: var(--warning);
  }

  .temp-value.hot {
    color: var(--danger);
  }

  .temp-label {
    font-size: 0.75rem;
    color: var(--text-tertiary);
    text-transform: uppercase;
    font-weight: 500;
  }

  /* ── Power ──────────────────────────────────────────────────── */
  .watts-total {
    font-size: 1.5rem;
    font-weight: 700;
    font-variant-numeric: tabular-nums;
    line-height: 1;
  }

  .watts-breakdown {
    display: flex;
    flex-wrap: wrap;
    gap: 0.35rem 0.7rem;
    font-size: 0.72rem;
    color: var(--text-tertiary);
  }

  /* ── CPU / Usage bars ───────────────────────────────────────── */
  .usage-row {
    display: flex;
    align-items: baseline;
    gap: 0.5rem;
  }

  .usage-pct {
    font-size: 1.5rem;
    font-weight: 700;
    font-variant-numeric: tabular-nums;
    line-height: 1;
  }

  .usage-label {
    font-size: 0.75rem;
    color: var(--text-tertiary);
    text-transform: uppercase;
    font-weight: 500;
  }

  .core-row {
    display: flex;
    flex-wrap: wrap;
    gap: 0.3rem 0.9rem;
    font-size: 0.72rem;
    color: var(--text-secondary);
  }

  .core-row b {
    font-weight: 600;
    color: var(--text-primary);
  }

  /* ── Bar track ──────────────────────────────────────────────── */
  .bar-track {
    height: 0.35rem;
    border-radius: 999px;
    background-color: var(--surface);
    overflow: hidden;
  }

  .bar-fill {
    height: 100%;
    border-radius: 999px;
    background-color: var(--primary);
    transition: width 0.5s ease;
  }

  .bar-fill.mem {
    background-color: var(--primary);
  }

  .bar-fill.swap {
    background-color: var(--accent);
  }

  /* ── Memory ─────────────────────────────────────────────────── */
  .mem-card {
    position: relative;
  }

  .mem-pressure {
    margin-left: auto;
    display: flex;
    align-items: center;
    color: var(--warning);
  }

  .mem-row {
    display: flex;
    justify-content: space-between;
    font-size: 0.8125rem;
  }

  .mem-row.swap {
    font-size: 0.75rem;
    color: var(--text-tertiary);
    margin-top: 0.2rem;
  }

  /* ── Hover popover: who is eating memory ───────────────────── */
  .mem-hover {
    display: none;
    position: absolute;
    top: calc(100% + 0.5rem);
    left: 0;
    right: 0;
    z-index: 20;
    padding: 0.85rem 1rem;
    background: var(--card-bg);
    border: 1px solid var(--card-border);
    border-radius: var(--radius);
    box-shadow: var(--card-shadow-hover);
  }

  .mem-card:hover .mem-hover,
  .mem-card:focus-within .mem-hover {
    display: block;
  }

  /* Keep hover stable: don't disappear when cursor moves from card to popover */
  .mem-card:hover .mem-hover:hover {
    display: block;
  }

  .mem-hover-head {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
    gap: 0.5rem;
    font-size: 0.75rem;
    margin-bottom: 0.5rem;
    padding-bottom: 0.4rem;
    border-bottom: 1px solid var(--card-border);
  }

  .mem-hover-head span {
    font-size: 0.65rem;
    color: var(--text-tertiary);
  }

  .proc-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
  }

  .proc-list li {
    display: grid;
    grid-template-columns: minmax(5rem, 1fr) 1.5fr 4rem;
    align-items: center;
    gap: 0.4rem;
    font-size: 0.7rem;
  }

  .proc-name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-weight: 500;
  }

  .proc-bar-wrap {
    height: 0.35rem;
    border-radius: 999px;
    background: var(--surface);
    overflow: hidden;
  }

  .proc-bar {
    height: 100%;
    border-radius: 999px;
    background: var(--primary-soft);
  }

  .proc-rss {
    text-align: right;
    font-size: 0.65rem;
    color: var(--text-secondary);
    font-variant-numeric: tabular-nums;
  }

  .mem-hint {
    margin: 0.45rem 0 0;
    font-size: 0.65rem;
    color: var(--text-tertiary);
    line-height: 1.4;
  }

  .mem-hint.warn {
    display: flex;
    align-items: start;
    gap: 0.3rem;
    margin-top: 0.55rem;
    padding-top: 0.45rem;
    border-top: 1px solid var(--card-border);
    color: var(--text-secondary);
  }

  /* ── Timestamp ──────────────────────────────────────────────── */
  .ts {
    display: flex;
    align-items: center;
    gap: 0.35rem;
    font-size: 0.6875rem;
    color: var(--text-tertiary);
    margin: 0 0 1.5rem;
  }
</style>
