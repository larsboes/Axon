<script lang="ts">
  import { onMount } from "svelte";
  import Icon from "$lib/Icon.svelte";
  import ImportReview from "$lib/finance/ImportReview.svelte";
  import InvestmentPreview from "$lib/finance/InvestmentPreview.svelte";
  import TransactionTable from "$lib/finance/TransactionTable.svelte";
  import { finance, type FinanceDashboard } from "$lib/api";

  type Mode = "overview" | "budget" | "transactions";
  let { mode }: { mode: Mode } = $props();
  const today = new Date().toISOString().slice(0, 10);
  let start = $state(`${today.slice(0, 4)}-01-01`);
  let end = $state(today);
  let account = $state("");
  let category = $state("");
  let data = $state<FinanceDashboard | null>(null);
  let error = $state<string | null>(null);
  let busy = $state(false);
  let activeLink = $state<number | null>(null);

  async function load() {
    busy = true;
    try {
      data = await finance.dashboard({ start, end, account, category, currency: "EUR" });
      error = null;
    } catch (cause) {
      error = cause instanceof Error ? cause.message : String(cause);
    } finally {
      busy = false;
    }
  }

  async function rebuild() {
    busy = true;
    try {
      await finance.rebuildLedger();
      await load();
    } catch (cause) {
      error = cause instanceof Error ? cause.message : String(cause);
      busy = false;
    }
  }

  function selectLink(index: number) {
    if (!data) return;
    const link = data.sankey[index];
    account = link.account;
    category = link.category;
    void load();
  }

  function clearLinkFilter() {
    account = "";
    category = "";
    void load();
  }

  function money(cents: number, currency = "EUR") {
    return new Intl.NumberFormat("de-DE", { style: "currency", currency }).format(cents / 100);
  }

  function short(account: string) {
    return account.split(":").slice(1).join(" · ") || account;
  }

  const trendMax = $derived(Math.max(1, ...(data?.trend.flatMap((point) => [point.income_cents, point.expense_cents]) ?? [1])));
  const incomeNodes = $derived([...new Set(data?.sankey.filter((link) => link.source.startsWith("income:")).map((link) => link.source) ?? [])]);
  const accountNodes = $derived([...new Set(data?.sankey.map((link) => link.account) ?? [])]);
  const expenseNodes = $derived([...new Set(data?.sankey.filter((link) => link.target.startsWith("expenses:")).map((link) => link.target) ?? [])]);
  const linkMax = $derived(Math.max(1, ...(data?.sankey.map((link) => link.amount_cents) ?? [1])));
  const nodeY = (nodes: string[], name: string) => 35 + (Math.max(0, nodes.indexOf(name)) + 0.5) * (290 / Math.max(1, nodes.length));
  const linkPath = (source: string, target: string) => {
    const income = source.startsWith("income:");
    const x1 = income ? 150 : 500;
    const x2 = income ? 500 : 850;
    const y1 = income ? nodeY(incomeNodes, source) : nodeY(accountNodes, source);
    const y2 = income ? nodeY(accountNodes, target) : nodeY(expenseNodes, target);
    return `M ${x1} ${y1} C ${(x1 + x2) / 2} ${y1}, ${(x1 + x2) / 2} ${y2}, ${x2} ${y2}`;
  };

  onMount(() => void load());
</script>

<form class="filters" onsubmit={(event) => { event.preventDefault(); void load(); }}>
  <label>From<input type="date" bind:value={start} /></label>
  <label>To<input type="date" bind:value={end} /></label>
  <label>Account<select bind:value={account}>
    <option value="">All accounts</option>
    {#each data?.accounts ?? [] as value (value)}<option value={value}>{short(value)}</option>{/each}
  </select></label>
  <button type="submit" disabled={busy}>Apply</button>
  {#if category}<button type="button" class="quiet" onclick={clearLinkFilter}>Clear Sankey filter</button>{/if}
  <button type="button" class="rebuild" disabled={busy} onclick={rebuild}>
    <Icon name="refresh" size={13} /> Rebuild journal index
  </button>
</form>

{#if error}
  <p class="error"><Icon name="alert" size={14} /> {error}</p>
{:else if !data}
  <p class="muted">Loading finance projection…</p>
{:else if mode === "overview"}
  <section class="cards" aria-label="Finance summary">
    <article><span>Income</span><strong>{money(data.summary.income_cents)}</strong></article>
    <article><span>Spending</span><strong>{money(data.summary.expense_cents)}</strong></article>
    <article><span>Net cash flow</span><strong class:negative={data.summary.net_cash_flow_cents < 0}>{money(data.summary.net_cash_flow_cents)}</strong></article>
    <article><span>Savings rate</span><strong>{data.summary.savings_rate_percent === null ? "—" : `${data.summary.savings_rate_percent.toFixed(1)}%`}</strong></article>
    <article><span>Budget variance</span><strong class:negative={data.summary.budget_variance_cents < 0}>{money(data.summary.budget_variance_cents)}</strong></article>
  </section>

  <div class="overview-grid">
    <section class="panel trend">
      <h2>Cash flow by month</h2>
      {#if data.trend.length}
        <div class="trend-rows">
          {#each data.trend as point (point.month)}
            <div class="trend-row">
              <time>{point.month}</time>
              <div class="bars">
                <span class="income" style:width={`${Math.max(1, point.income_cents / trendMax * 100)}%`} title={`Income ${money(point.income_cents)}`}></span>
                <span class="expense" style:width={`${Math.max(1, point.expense_cents / trendMax * 100)}%`} title={`Spending ${money(point.expense_cents)}`}></span>
              </div>
              <strong class:negative={point.net_cash_flow_cents < 0}>{money(point.net_cash_flow_cents)}</strong>
            </div>
          {/each}
        </div>
        <p class="legend"><i class="income"></i> income <i class="expense"></i> spending; value at right is net cash flow.</p>
      {:else}<p class="muted">No transactions in this period.</p>{/if}
    </section>

    <section class="panel flow">
      <div class="panel-heading">
        <div><h2>Money flow</h2><p>Income source → account → expense category</p></div>
        {#if activeLink !== null}<strong>{money(data.sankey[activeLink].amount_cents)}</strong>{/if}
      </div>
      {#if data.sankey.length}
        <svg viewBox="0 0 1000 360" role="img" aria-label="Interactive money-flow Sankey">
          {#each data.sankey as link, index (`${link.source}-${link.target}`)}
            <a href={`#flow-${index}`} onclick={(event) => { event.preventDefault(); selectLink(index); }} onmouseenter={() => activeLink = index} onmouseleave={() => activeLink = null}>
              <path d={linkPath(link.source, link.target)} stroke-width={Math.max(3, link.amount_cents / linkMax * 25)} class:active={activeLink === index}>
                <title>{short(link.source)} to {short(link.target)}: {money(link.amount_cents)}</title>
              </path>
            </a>
          {/each}
          {#each incomeNodes as node (node)}<text x="140" y={nodeY(incomeNodes, node)} text-anchor="end">{short(node)}</text>{/each}
          {#each accountNodes as node (node)}<text x="500" y={nodeY(accountNodes, node)} text-anchor="middle">{short(node)}</text>{/each}
          {#each expenseNodes as node (node)}<text x="860" y={nodeY(expenseNodes, node)}>{short(node)}</text>{/each}
        </svg>
        <p class="hint">Hover for exact values. Select a flow to filter the transaction table.</p>
      {:else}<p class="muted">No income or expense flow in this period.</p>{/if}
    </section>
  </div>

  <section class="panel recent">
    <h2>Recent transactions</h2>
    <TransactionTable rows={data.transactions.slice(0, 8)} />
  </section>
{:else if mode === "budget"}
  <section class="panel budget">
    <div class="panel-heading"><div><h2>Budget against actual</h2><p>Variance is budget minus spending for the selected period.</p></div><strong>{money(data.summary.budget_variance_cents)}</strong></div>
    {#if data.budgets.length}
      {#each data.budgets as row (row.account)}
        <article>
          <div><strong>{short(row.account)}</strong><span>{money(row.actual_cents)} of {money(row.budget_cents)}</span></div>
          <div class="track"><span class:over={row.actual_cents > row.budget_cents} style:width={`${Math.min(100, row.budget_cents > 0 ? row.actual_cents / row.budget_cents * 100 : 0)}%`}></span></div>
          <strong class:negative={row.variance_cents < 0}>{money(row.variance_cents)}</strong>
        </article>
      {/each}
    {:else}<p class="muted">No budget targets configured in the private Finance config.</p>{/if}
  </section>
{:else}
  <section class="panel transactions">
    <div class="panel-heading"><div><h2>Transactions</h2><p>{data.transactions.length} normalized journal rows. Transfers are excluded from totals by default.</p></div></div>
    <TransactionTable rows={data.transactions} />
  </section>
  <ImportReview onchanged={load} />
  <InvestmentPreview />
{/if}

<style>
  .filters { display: flex; flex-wrap: wrap; align-items: end; gap: .65rem; margin-bottom: 1.2rem; }
  label { display: flex; flex-direction: column; gap: .2rem; font-size: .68rem; color: var(--muted, #888); }
  input, select, button { border: 1px solid var(--border, #333); border-radius: 6px; padding: .38rem .55rem; font: inherit; font-size: .78rem; background: transparent; color: inherit; }
  button { cursor: pointer; }
  button:disabled { opacity: .45; }
  button.rebuild { display: inline-flex; align-items: center; gap: .3rem; margin-left: auto; }
  button.quiet { border-color: transparent; color: var(--muted, #888); }
  .cards { display: grid; grid-template-columns: repeat(auto-fit, minmax(9.5rem, 1fr)); gap: .65rem; margin-bottom: .75rem; }
  .cards article, .panel { border: 1px solid var(--border, #333); border-radius: 8px; background: var(--card-bg, transparent); }
  .cards article { display: flex; flex-direction: column; gap: .3rem; padding: .8rem; }
  .cards span, .panel p, .hint, .legend { color: var(--muted, #888); font-size: .7rem; }
  .cards strong { font-size: 1.25rem; font-variant-numeric: tabular-nums; }
  .negative { color: var(--danger, #b44); }
  .overview-grid { display: grid; grid-template-columns: minmax(18rem, .8fr) minmax(24rem, 1.2fr); gap: .75rem; }
  .panel { padding: .9rem; min-width: 0; }
  .panel h2 { margin: 0; font-size: .85rem; }
  .panel p { margin: .2rem 0 0; }
  .panel-heading { display: flex; align-items: start; justify-content: space-between; gap: 1rem; margin-bottom: .75rem; }
  .trend-rows { display: grid; gap: .55rem; margin-top: 1rem; }
  .trend-row { display: grid; grid-template-columns: 4.5rem 1fr 6.5rem; align-items: center; gap: .6rem; font-size: .7rem; }
  .trend-row time { color: var(--muted, #888); }
  .trend-row > strong { text-align: right; font-variant-numeric: tabular-nums; }
  .bars { display: grid; gap: 3px; }
  .bars span { display: block; height: 5px; border-radius: 3px; }
  .income { background: var(--primary); }
  .expense { background: #9b7653; }
  .legend { display: flex; align-items: center; gap: .35rem; }
  .legend i { width: .55rem; height: .35rem; display: inline-block; }
  svg { width: 100%; min-height: 17rem; overflow: visible; }
  svg path { fill: none; stroke: var(--primary); opacity: .28; transition: opacity .12s; cursor: pointer; }
  svg path:hover, svg path.active { opacity: .85; }
  svg text { fill: currentColor; font-size: 14px; dominant-baseline: middle; pointer-events: none; }
  .hint { text-align: center; }
  .recent { margin-top: .75rem; }
  .budget article { display: grid; grid-template-columns: minmax(12rem, 1fr) minmax(12rem, 2fr) 7rem; gap: 1rem; align-items: center; padding: .75rem 0; border-top: 1px solid var(--border, #333); font-size: .78rem; }
  .budget article > div:first-child { display: flex; justify-content: space-between; gap: 1rem; }
  .budget article span { color: var(--muted, #888); }
  .budget article > strong { text-align: right; }
  .track { height: 7px; border-radius: 4px; background: color-mix(in srgb, currentColor 10%, transparent); overflow: hidden; }
  .track span { display: block; height: 100%; background: var(--primary); }
  .track span.over { background: var(--danger, #b44); }
  .error { display: flex; align-items: center; gap: .35rem; }
  .muted { color: var(--muted, #888); }
  @media (max-width: 850px) { .overview-grid { grid-template-columns: 1fr; } button.rebuild { margin-left: 0; } .budget article { grid-template-columns: 1fr; gap: .4rem; } .budget article > strong { text-align: left; } }
</style>
