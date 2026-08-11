<script lang="ts">
  import type { FinanceDashboard } from "$lib/api";

  type ReviewTarget = "overview" | "transactions" | "subscriptions";

  interface ReviewItem {
    id: string;
    label: string;
    detail: string;
    ready: boolean;
    target: ReviewTarget | null;
  }

  let {
    data,
    onnavigate,
  }: {
    data: FinanceDashboard;
    onnavigate: (target: ReviewTarget) => void;
  } = $props();

  const report = $derived(data.planning);
  const current = $derived(report.forecasts[0] ?? null);
  const future = $derived(report.forecasts.slice(1));
  const liquidity = $derived(report.liquidity);
  const decision = $derived(report.card_decision);
  const maxBehavior = $derived(Math.max(1, ...report.baseline.behavior.map((item) => item.monthly_cents)));

  function money(cents: number, currency = report.currency) {
    return new Intl.NumberFormat("de-DE", { style: "currency", currency }).format(cents / 100);
  }

  function percent(value: number | null, digits = 1) {
    return value === null ? "—" : `${value.toFixed(digits)}%`;
  }

  function pointsPerUnit(value: number) {
    return (value / 1000).toLocaleString("de-DE", { maximumFractionDigits: 3 });
  }

  function centsPerPoint(value: number) {
    return (value / 1000).toLocaleString("de-DE", { maximumFractionDigits: 3 });
  }

  function navigate(target: ReviewTarget | null) {
    if (target !== null) onnavigate(target);
  }

  const readiness = $derived.by<ReviewItem[]>(() => {
    const sourceIssues = data.source_freshness.filter(
      (source) => source.freshness !== "current" || source.coverage !== "complete",
    ).length;
    const classified = report.baseline.classified_value_percent ?? 0;
    const subscriptionsNeedReview =
      report.subscriptions.unknown_price_count + report.subscriptions.anomalies.length;

    return [
      {
        id: "sources",
        label: "Data coverage and freshness",
        detail: sourceIssues === 0
          ? "Every expected source is current and complete."
          : `${sourceIssues} source${sourceIssues === 1 ? "" : "s"} remain stale, missing, or partial.`,
        ready: data.source_freshness.length > 0 && sourceIssues === 0,
        target: "overview",
      },
      {
        id: "baseline",
        label: "Monthly baseline",
        detail: report.baseline.months.length === 0
          ? "No complete month is available yet."
          : `${percent(classified)} of baseline spending has a behavior rule.`,
        ready: report.baseline.months.length > 0 && classified >= 80,
        target: "transactions",
      },
      {
        id: "scenario",
        label: "Dated scenario",
        detail: future.length
          ? `${future.length} future change point${future.length === 1 ? "" : "s"} modelled.`
          : "No future comparison date is configured.",
        ready: future.length > 0,
        target: null,
      },
      {
        id: "subscriptions",
        label: "Subscription portfolio",
        detail: subscriptionsNeedReview === 0
          ? "Prices, states, and value signals have no open flags."
          : `${subscriptionsNeedReview} price, state, or value flag${subscriptionsNeedReview === 1 ? "" : "s"} need review.`,
        ready: subscriptionsNeedReview === 0,
        target: "subscriptions",
      },
      {
        id: "liquidity",
        label: "Cash and investments",
        detail: liquidity?.complete
          ? "Runway and allocation use complete reviewed snapshots."
          : "Net worth and allocation remain a lower bound.",
        ready: liquidity?.complete === true,
        target: "overview",
      },
      {
        id: "cards-loyalty",
        label: "Cards and loyalty",
        detail: !decision
          ? "No private card assumptions are configured."
          : decision.provisional
            ? "Usage or personal benefit values still need review."
            : report.loyalty.length === 0
              ? "Card inputs are reviewed; loyalty balances are still missing."
              : "Card usage, benefits, and loyalty assumptions are reviewed.",
        ready: decision !== null && !decision.provisional && report.loyalty.length > 0,
        target: null,
      },
    ];
  });
  const readyCount = $derived(readiness.filter((item) => item.ready).length);
</script>

<section class="intro">
  <div>
    <span>Planning baseline</span>
    <strong>{report.baseline.months.join(" · ") || "No complete month yet"}</strong>
  </div>
  <p>Median complete-month behavior, then dated commitments and reviewed subscription history. It is a forecast, not a ledger balance.</p>
</section>

<section class="readiness" aria-label="Finance decision readiness">
  <div class="readiness-heading">
    <div>
      <span>Decision readiness</span>
      <strong>{readyCount} of {readiness.length} areas ready</strong>
    </div>
    <p>The models remain usable when evidence is partial, but recommendations stay provisional until these checks close.</p>
  </div>
  <ol>
    {#each readiness as item (item.id)}
      <li class:ready={item.ready}>
        <span class="readiness-state">{item.ready ? "ready" : "review"}</span>
        <div><strong>{item.label}</strong><small>{item.detail}</small></div>
        {#if item.target}
          <button type="button" onclick={() => navigate(item.target)}>
            Open {item.target}
          </button>
        {:else if !item.ready}
          <span class="private-input">private inputs</span>
        {/if}
      </li>
    {/each}
  </ol>
</section>

<section class="metrics" aria-label="Monthly planning baseline">
  <article>
    <span>Typical income</span>
    <strong>{money(report.baseline.monthly_income_cents)}</strong>
    <small>Median across the selected complete months</small>
  </article>
  <article>
    <span>Typical personal spend</span>
    <strong>{money(report.baseline.monthly_spending_cents)}</strong>
    <small>{percent(report.baseline.classified_value_percent)} behavior-classified</small>
  </article>
  <article>
    <span>Typical result</span>
    <strong class:negative={report.baseline.monthly_result_cents < 0}>{money(report.baseline.monthly_result_cents)}</strong>
    <small>{percent(report.baseline.savings_rate_percent)} savings rate</small>
  </article>
  <article>
    <span>Current projection</span>
    <strong class:negative={(current?.projected_result_cents ?? 0) < 0}>{current ? money(current.projected_result_cents) : "—"}</strong>
    <small>{current ? `${money(current.projected_spending_cents)} planned spend` : "No projection"}</small>
  </article>
</section>

<section class="panel behavior">
  <div class="heading">
    <div><h2>What the baseline is made of</h2><p>Rules live in the private Finance config. Longest account-prefix match wins.</p></div>
  </div>
  <div class="behavior-rows">
    {#each report.baseline.behavior as item (item.behavior)}
      <div class="behavior-row">
        <span>{item.behavior}</span>
        <div><i style:width={`${item.monthly_cents / maxBehavior * 100}%`}></i></div>
        <strong>{money(item.monthly_cents)}</strong>
      </div>
    {/each}
  </div>
</section>

<section class="panel forecast">
  <div class="heading">
    <div><h2>Dated change model</h2><p>Historical base + active commitments + subscription burn + explicit private adjustments. Exceptional spending is excluded.</p></div>
    {#if future.length}<strong>{future.length} change point{future.length === 1 ? "" : "s"}</strong>{/if}
  </div>
  {#if report.forecasts.length}
    <div class="forecast-table" role="table" aria-label="Monthly forecasts">
      <div class="forecast-header" role="row">
        <span>Date</span><span>Historical base</span><span>Commitments</span><span>Subscriptions</span><span>Adjustments</span><span>Spend</span><span>Result</span>
      </div>
      {#each report.forecasts as point, index (point.as_of)}
        <div class:future={index > 0} class="forecast-row" role="row">
          <time datetime={point.as_of}>{point.as_of}</time>
          <span>{money(point.historical_base_cents)}</span>
          <span>{money(point.commitments_cents)}</span>
          <span>{money(point.subscriptions_cents)}</span>
          <span>{money(point.adjustments_cents)}</span>
          <strong>{money(point.projected_spending_cents)}</strong>
          <strong class:negative={point.projected_result_cents < 0}>{money(point.projected_result_cents)} <small>{percent(point.savings_rate_percent)}</small></strong>
        </div>
      {/each}
    </div>
  {:else}<p class="empty">No forecast can be calculated yet.</p>{/if}
</section>

<section class="split">
  <article class="panel liquidity">
    <div class="heading">
      <div><h2>Liquidity and allocation</h2><p>Cash runway uses projected monthly spend. Partial holdings remain a lower bound.</p></div>
      {#if report.liquidity}<span class:warning={!report.liquidity.complete}>{report.liquidity.complete ? "complete" : "partial"}</span>{/if}
    </div>
    {#if liquidity}
      <dl>
        <div><dt>Liquid assets</dt><dd>{money(liquidity.liquid_assets_cents, liquidity.currency)}</dd></div>
        <div><dt>Invested, reviewed</dt><dd>{liquidity.invested_cents === null ? "—" : `${liquidity.complete ? "" : "≥ "}${money(liquidity.invested_cents, liquidity.currency)}`}</dd></div>
        <div><dt>Tracked net worth</dt><dd>{liquidity.net_worth_cents === null ? "—" : `${liquidity.complete ? "" : "≥ "}${money(liquidity.net_worth_cents, liquidity.currency)}`}</dd></div>
        <div><dt>Cash share of tracked assets</dt><dd>{percent(liquidity.cash_share_percent)}</dd></div>
        <div><dt>Cash runway</dt><dd>{liquidity.runway_months === null ? "—" : `${liquidity.runway_months.toFixed(1)} months`}</dd></div>
        <div><dt>Target buffer</dt><dd>{money(liquidity.target_cash_cents, liquidity.currency)}</dd></div>
        <div><dt>Above / below target</dt><dd class:negative={liquidity.cash_buffer_cents < 0}>{liquidity.cash_buffer_cents >= 0 ? "+" : ""}{money(liquidity.cash_buffer_cents, liquidity.currency)}</dd></div>
        <div><dt>Largest priced holding</dt><dd>{percent(liquidity.largest_priced_holding_percent)}</dd></div>
        <div><dt>Liabilities</dt><dd>{money(liquidity.liabilities_cents, liquidity.currency)}</dd></div>
      </dl>
    {:else}<p class="empty">Add a reviewed manual balance snapshot to calculate runway.</p>{/if}
  </article>

  <article class="panel subscriptions">
    <div class="heading"><div><h2>Subscription portfolio</h2><p>Use the Subscriptions view for plan combinations and history.</p></div></div>
    <div class="subscription-total">
      <strong>{money(report.subscriptions.monthly_cents)}</strong>
      <span>per month · {money(report.subscriptions.annual_cents)} per year</span>
    </div>
    <p>{report.subscriptions.billing_count} personally billing · {report.subscriptions.covered_count} externally covered · {report.subscriptions.unknown_price_count} missing price</p>
    {#if report.subscriptions.anomalies.length}
      <ul class="anomalies">
        {#each report.subscriptions.anomalies as anomaly (`${anomaly.subscription_id}-${anomaly.kind}`)}
          <li><strong>{anomaly.subscription_name}</strong><span>{anomaly.detail}</span></li>
        {/each}
      </ul>
    {:else}<p class="empty">No price, state, or value anomalies detected.</p>{/if}
  </article>
</section>

<section class="panel cards">
  <div class="heading">
    <div><h2>Card and rewards decision</h2><p>Personal benefit value and eligible spend remain private. Public provider claims require dated source links.</p></div>
    {#if report.card_decision}<span class:warning={report.card_decision.provisional}>{report.card_decision.provisional ? "provisional" : "reviewed"}</span>{/if}
  </div>
  {#if decision}
    <p class="decision-input">Modelled on {money(decision.annual_eligible_spend_cents)} eligible spend ({decision.spend_source === "reviewed_transactions" ? `${decision.spend_period_start} to ${decision.spend_period_end}` : "manual annual estimate"}) and {money(decision.annual_fx_spend_cents)} FX spend.</p>
    <div class="card-grid">
      {#each decision.options as option (option.id)}
        <article>
          <div class="option-title"><strong>{option.label}</strong><strong class:negative={option.annual_net_value_cents < 0}>{option.annual_net_value_cents >= 0 ? "+" : ""}{money(option.annual_net_value_cents, option.currency)} / year</strong></div>
          <dl>
            <div><dt>Fee</dt><dd>−{money(option.annual_fee_cents, option.currency)}</dd></div>
            <div><dt>Personal benefit value</dt><dd>+{money(option.annual_benefit_value_cents, option.currency)}</dd></div>
            <div><dt>Unvalued face value</dt><dd>{money(option.annual_unvalued_face_value_cents, option.currency)}</dd></div>
            <div><dt>Rewards</dt><dd>+{money(option.annual_reward_value_cents, option.currency)}</dd></div>
            <div><dt>FX cost</dt><dd>{option.annual_fx_cost_cents ? `−${money(option.annual_fx_cost_cents, option.currency)}` : money(0, option.currency)}</dd></div>
            <div><dt>Break-even spend</dt><dd>{option.break_even_eligible_spend_cents === null ? "Not reachable from rewards" : money(option.break_even_eligible_spend_cents, option.currency)}</dd></div>
          </dl>
          {#if option.benefits.length}
            <ul class="benefits">
              {#each option.benefits as benefit (benefit.id)}
                <li><span>{benefit.label}</span><strong>{money(benefit.annual_personal_value_cents, option.currency)} <small>of {money(benefit.annual_face_value_cents, option.currency)}</small></strong></li>
              {/each}
            </ul>
          {/if}
          <p>{pointsPerUnit(option.points_per_currency_unit_milli)} point / currency unit · {centsPerPoint(option.point_value_milli_cents)} ct assumed per point{option.point_value_assumption ? ` · ${option.point_value_assumption}` : ""}</p>
          <footer>Terms checked {option.terms_checked_on}{#each option.source_urls as url, index (url)}<span> · </span><a href={url} target="_blank" rel="noreferrer">source {index + 1}</a>{/each}</footer>
        </article>
      {/each}
    </div>
  {:else}
    <p class="empty">No private card usage and valuation assumptions configured yet. The comparison stays blank instead of inventing a recommendation.</p>
  {/if}

  {#if report.loyalty.length}
    <div class="loyalty">
      <h3>Loyalty balances</h3>
      {#each report.loyalty as balance (balance.id)}
        <article>
          <strong>{balance.label}</strong>
          <span>{balance.points.toLocaleString("de-DE")} points</span>
          <span>≈ {money(balance.estimated_value_cents)}</span>
          <small>{balance.assumption || `${centsPerPoint(balance.point_value_milli_cents)} ct per point`}{balance.transfer_path ? ` · ${balance.transfer_path}` : balance.transferable ? " · transferable" : ""}{balance.as_of ? ` · as of ${balance.as_of}` : ""}{balance.expires_on ? ` · expires ${balance.expires_on}` : ""}{#each balance.source_urls as url, index (url)}<span> · </span><a href={url} target="_blank" rel="noreferrer">source {index + 1}</a>{/each}</small>
        </article>
      {/each}
    </div>
  {/if}
</section>

{#if report.caveats.length}
  <aside class="caveats" aria-label="Planning caveats">
    <strong>Before acting on this</strong>
    <ul>{#each report.caveats as caveat (caveat)}<li>{caveat}</li>{/each}</ul>
  </aside>
{/if}

<style>
  .intro { display: grid; grid-template-columns: minmax(14rem, .7fr) minmax(18rem, 1.3fr); gap: 1rem; align-items: end; border-block: 1px solid var(--border, #333); padding: .8rem 0; margin-bottom: .8rem; }
  .intro div { display: grid; gap: .15rem; }
  .intro span, p, small, dt, footer { color: var(--muted, #888); }
  .intro span { font-size: .64rem; text-transform: uppercase; letter-spacing: .05em; }
  .intro strong { font-size: .9rem; }
  .intro p { margin: 0; font-size: .7rem; line-height: 1.45; }
  .readiness { border-block: 1px solid var(--border, #333); margin-bottom: .8rem; }
  .readiness-heading { display: grid; grid-template-columns: minmax(12rem, .7fr) minmax(18rem, 1.3fr); gap: 1rem; align-items: end; padding: .65rem 0; }
  .readiness-heading > div { display: grid; gap: .12rem; }
  .readiness-heading span { color: var(--muted, #888); font-size: .61rem; text-transform: uppercase; letter-spacing: .05em; }
  .readiness-heading strong { font-size: .86rem; }
  .readiness-heading p { margin: 0; color: var(--muted, #888); font-size: .67rem; line-height: 1.4; }
  .readiness ol { margin: 0; padding: 0; list-style: none; display: grid; grid-template-columns: 1fr 1fr; }
  .readiness li { display: grid; grid-template-columns: 3.4rem minmax(0, 1fr) auto; gap: .65rem; align-items: center; padding: .55rem 0; border-top: 1px solid var(--border, #333); font-size: .68rem; }
  .readiness li:nth-child(odd) { padding-right: .75rem; }
  .readiness li:nth-child(even) { padding-left: .75rem; border-left: 1px solid var(--border, #333); }
  .readiness-state, .private-input { color: var(--danger, #b44); font-size: .59rem; text-transform: uppercase; letter-spacing: .05em; }
  .readiness li.ready .readiness-state { color: var(--primary); }
  .readiness li > div { display: grid; gap: .12rem; }
  .readiness li small { line-height: 1.35; }
  .readiness button { border: 1px solid var(--border, #333); border-radius: 5px; padding: .3rem .45rem; background: transparent; color: inherit; font: inherit; font-size: .62rem; cursor: pointer; white-space: nowrap; }
  .private-input { color: var(--muted, #888); white-space: nowrap; }
  .metrics { display: grid; grid-template-columns: repeat(4, minmax(0, 1fr)); gap: .65rem; margin-bottom: .75rem; }
  .metrics article, .panel { border: 1px solid var(--border, #333); border-radius: 8px; background: var(--card-bg, transparent); }
  .metrics article { display: grid; gap: .25rem; padding: .8rem; }
  .metrics span, .metrics small { color: var(--muted, #888); font-size: .63rem; }
  .metrics strong { font-size: 1.15rem; font-variant-numeric: tabular-nums; }
  .panel { padding: .9rem; min-width: 0; margin-bottom: .75rem; }
  .heading { display: flex; justify-content: space-between; gap: 1rem; align-items: start; margin-bottom: .8rem; }
  h2, h3 { margin: 0; font-size: .85rem; }
  .heading p, .panel > p { margin: .18rem 0 0; font-size: .68rem; }
  .heading > span { font-size: .66rem; text-transform: uppercase; letter-spacing: .05em; }
  .warning, .negative { color: var(--danger, #b44); }
  .behavior-rows { display: grid; gap: .5rem; }
  .behavior-row { display: grid; grid-template-columns: 6.5rem 1fr 7rem; gap: .65rem; align-items: center; font-size: .7rem; }
  .behavior-row > span { color: var(--muted, #888); text-transform: capitalize; }
  .behavior-row > div { height: 6px; background: color-mix(in srgb, currentColor 9%, transparent); overflow: hidden; }
  .behavior-row i { display: block; height: 100%; background: var(--primary); }
  .behavior-row strong { text-align: right; font-variant-numeric: tabular-nums; }
  .forecast-table { overflow-x: auto; }
  .forecast-header, .forecast-row { display: grid; grid-template-columns: 6.5rem repeat(5, minmax(6.5rem, 1fr)) minmax(8rem, 1.2fr); gap: .7rem; min-width: 58rem; align-items: baseline; }
  .forecast-header { color: var(--muted, #888); font-size: .61rem; text-transform: uppercase; letter-spacing: .04em; padding: 0 .35rem .4rem; }
  .forecast-row { border-top: 1px solid var(--border, #333); padding: .55rem .35rem; font-size: .69rem; }
  .forecast-row.future { background: color-mix(in srgb, var(--primary) 4%, transparent); }
  .forecast-row span, .forecast-row strong { text-align: right; font-variant-numeric: tabular-nums; }
  .forecast-row time { color: var(--muted, #888); }
  .forecast-row strong small { display: block; font-size: .58rem; font-weight: 400; }
  .split { display: grid; grid-template-columns: 1fr 1fr; gap: .75rem; }
  dl { margin: 0; }
  .liquidity dl, .cards article dl { display: grid; grid-template-columns: 1fr 1fr; gap: .45rem 1rem; }
  dl div { display: flex; justify-content: space-between; gap: .7rem; border-top: 1px solid var(--border, #333); padding-top: .4rem; font-size: .68rem; }
  dd { margin: 0; text-align: right; font-variant-numeric: tabular-nums; }
  .subscription-total { display: flex; align-items: baseline; gap: .45rem; margin: .25rem 0; }
  .subscription-total strong { font-size: 1.4rem; }
  .subscription-total span { color: var(--muted, #888); font-size: .65rem; }
  .anomalies { margin: .75rem 0 0; padding: 0; list-style: none; }
  .anomalies li { display: grid; gap: .12rem; border-top: 1px solid var(--border, #333); padding: .45rem 0; font-size: .68rem; }
  .anomalies span { color: var(--muted, #888); }
  .decision-input { margin-bottom: .7rem !important; }
  .card-grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(19rem, 1fr)); gap: .65rem; }
  .card-grid > article { border-top: 2px solid var(--primary); padding: .7rem; background: color-mix(in srgb, currentColor 2%, transparent); }
  .option-title { display: flex; justify-content: space-between; gap: 1rem; margin-bottom: .6rem; font-size: .76rem; }
  .card-grid p, .card-grid footer { font-size: .62rem; line-height: 1.45; }
  .benefits { margin: .65rem 0; padding: 0; list-style: none; border-top: 1px solid var(--border, #333); }
  .benefits li { display: flex; justify-content: space-between; gap: .7rem; padding: .35rem 0; border-bottom: 1px solid var(--border, #333); font-size: .64rem; }
  .benefits strong { text-align: right; font-variant-numeric: tabular-nums; }
  .benefits small { font-weight: 400; }
  .card-grid footer { border-top: 1px solid var(--border, #333); padding-top: .45rem; }
  a { color: var(--primary); }
  .loyalty { border-top: 1px solid var(--border, #333); margin-top: .85rem; padding-top: .75rem; }
  .loyalty h3 { margin-bottom: .5rem; }
  .loyalty > article { display: grid; grid-template-columns: 1fr auto auto minmax(10rem, 1fr); gap: 1rem; padding: .45rem 0; border-top: 1px solid var(--border, #333); font-size: .67rem; }
  .loyalty span { font-variant-numeric: tabular-nums; }
  .empty { color: var(--muted, #888); font-size: .68rem; }
  .caveats { border-left: 2px solid #a65f4c; padding: .2rem 0 .2rem .75rem; margin: .8rem 0; }
  .caveats strong { font-size: .72rem; }
  .caveats ul { margin: .35rem 0 0; padding-left: 1rem; color: var(--muted, #888); font-size: .66rem; }
  @media (max-width: 900px) { .metrics { grid-template-columns: repeat(2, 1fr); } .split { grid-template-columns: 1fr; } }
  @media (max-width: 700px) { .readiness ol { grid-template-columns: 1fr; } .readiness li:nth-child(odd), .readiness li:nth-child(even) { padding-inline: 0; border-left: 0; } }
  @media (max-width: 620px) { .intro, .readiness-heading, .metrics { grid-template-columns: 1fr; } .heading { flex-direction: column; } .readiness li { grid-template-columns: 3.4rem minmax(0, 1fr); } .readiness li button, .readiness li .private-input { grid-column: 2; justify-self: start; } .behavior-row { grid-template-columns: 5.5rem 1fr; } .behavior-row strong { grid-column: 2; text-align: left; } .liquidity dl, .cards article dl { grid-template-columns: 1fr; } .loyalty > article { grid-template-columns: 1fr 1fr; gap: .3rem 1rem; } }
</style>
