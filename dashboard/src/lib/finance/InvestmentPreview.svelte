<script lang="ts">
  import { onMount } from "svelte";
  import Icon from "$lib/Icon.svelte";
  import {
    finance,
    type HoldingsCoverage,
    type InvestmentCsvMapping,
    type InvestmentCsvMappingProfile,
    type InvestmentPreview,
  } from "$lib/api";

  let { onchanged = () => {} }: { onchanged?: () => void } = $props();

  let profiles = $state<InvestmentCsvMappingProfile[]>([]);
  let selectedProfile = $state("");
  let sourceKey = $state("");
  let coverage = $state<HoldingsCoverage>("complete");
  let content = $state("");
  let filename = $state("");
  let busy = $state(false);
  let error = $state<string | null>(null);
  let result = $state<InvestmentPreview | null>(null);
  let confirmed = $state(false);
  let positionActivityValues = $state("");
  let nonPositionActivityValues = $state("");
  let mapping = $state<InvestmentCsvMapping>({
    delimiter: ";",
    decimal_separator: ",",
    date_column: "Date",
    instrument_column: "Instrument",
    quantity_column: "Quantity",
    activity_type_column: null,
    position_activity_values: [],
    non_position_activity_values: [],
    reference_column: "Reference",
    price_column: "Price",
    currency_column: "Currency",
    default_currency: "EUR",
    instrument_aliases: {},
  });

  onMount(async () => {
    try {
      profiles = await finance.investmentMappings();
    } catch (cause) {
      error = cause instanceof Error ? cause.message : String(cause);
    }
  });

  async function choose(event: Event) {
    const input = event.currentTarget as HTMLInputElement;
    const file = input.files?.[0];
    if (!file) return;
    filename = file.name;
    content = await file.text();
    result = null;
    confirmed = false;
  }

  function selectMapping(event: Event) {
    selectedProfile = (event.currentTarget as HTMLSelectElement).value;
    if (!selectedProfile) {
      sourceKey = "";
      coverage = "complete";
      return;
    }
    const profile = profiles[Number(selectedProfile)];
    if (profile) {
      sourceKey = profile.source_key;
      coverage = profile.coverage;
      mapping = structuredClone(profile.mapping);
      positionActivityValues = mapping.position_activity_values.join(", ");
      nonPositionActivityValues = mapping.non_position_activity_values.join(", ");
      result = null;
      confirmed = false;
    }
  }

  function normalizedMapping(): InvestmentCsvMapping {
    return {
      ...mapping,
      activity_type_column: mapping.activity_type_column?.trim() || null,
      position_activity_values: positionActivityValues.split(",").map((value) => value.trim()).filter(Boolean),
      non_position_activity_values: nonPositionActivityValues.split(",").map((value) => value.trim()).filter(Boolean),
      reference_column: mapping.reference_column?.trim() || null,
      price_column: mapping.price_column?.trim() || null,
      currency_column: mapping.currency_column?.trim() || null,
    };
  }

  async function preview() {
    if (!content) return;
    busy = true;
    error = null;
    try {
      result = await finance.previewInvestments(content, normalizedMapping());
      confirmed = false;
    } catch (cause) {
      error = cause instanceof Error ? cause.message : String(cause);
    } finally {
      busy = false;
    }
  }

  async function confirm() {
    if (!content || !result) return;
    busy = true;
    error = null;
    try {
      const response = await finance.confirmInvestments(
        content,
        normalizedMapping(),
        sourceKey.trim(),
        result.snapshot_id,
        coverage,
      );
      confirmed = true;
      result = { ...result, holdings: response.snapshot.holdings };
      onchanged();
    } catch (cause) {
      error = cause instanceof Error ? cause.message : String(cause);
    } finally {
      busy = false;
    }
  }

  function quantity(mantissa: string, scale: number): string {
    const sign = mantissa.startsWith("-") ? "-" : "";
    const digits = (sign ? mantissa.slice(1) : mantissa).padStart(scale + 1, "0");
    if (scale === 0) return `${sign}${digits}`;
    return `${sign}${digits.slice(0, -scale)}.${digits.slice(-scale)}`;
  }

</script>

<section class="preview">
  <div class="heading">
    <div>
      <h2>Investment activity preview</h2>
      <p>Reconstruct holdings from signed quantities. Previewing stores nothing and never writes the journal.</p>
    </div>
    <label class="file">
      <Icon name="plus" size={14} /> {filename || "Choose activity CSV"}
      <input type="file" accept=".csv,text/csv" onchange={choose} />
    </label>
  </div>

  {#if content}
    <div class="mapping">
      {#if profiles.length > 0}
        <label class="profile">Mapping profile
          <select value={selectedProfile} onchange={selectMapping}>
            <option value="">Manual entry</option>
            {#each profiles as profile, index}
              <option value={index}>{profile.label}</option>
            {/each}
          </select>
        </label>
      {/if}
      <label>Source key<input bind:value={sourceKey} placeholder="required" /></label>
      <label>Coverage<select bind:value={coverage}><option value="complete">Complete source</option><option value="partial">Partial source</option></select></label>
      <label>Delimiter<input maxlength="1" bind:value={mapping.delimiter} /></label>
      <label>Decimal separator<input maxlength="1" bind:value={mapping.decimal_separator} /></label>
      <label>Date column<input bind:value={mapping.date_column} /></label>
      <label>Instrument column<input bind:value={mapping.instrument_column} /></label>
      <label>Quantity column<input bind:value={mapping.quantity_column} /></label>
      <label>Activity type column<input bind:value={mapping.activity_type_column} placeholder="optional" /></label>
      <label>Position activity values<input bind:value={positionActivityValues} placeholder="BUY, SELL" /></label>
      <label>Non-position activity values<input bind:value={nonPositionActivityValues} placeholder="DIVIDEND" /></label>
      <label>Reference column<input bind:value={mapping.reference_column} placeholder="optional" /></label>
      <label>Price column<input bind:value={mapping.price_column} placeholder="optional" /></label>
      <label>Currency column<input bind:value={mapping.currency_column} placeholder="optional" /></label>
      <label>Default currency<input maxlength="3" bind:value={mapping.default_currency} /></label>
      <button disabled={busy} onclick={preview}>Preview holdings</button>
    </div>
  {/if}

  {#if error}<p class="error">{error}</p>{/if}
  {#if result}
    <p class="summary">
      {result.activity_count} position-changing rows · {result.holdings.length} open positions ·
      {result.closed_positions} closed · {result.ignored_non_position_rows} non-position rows ignored ·
      {result.duplicate_rows} duplicates
    </p>
    <div class="review">
      <button class:partial={coverage === "partial"} disabled={busy || confirmed || !sourceKey.trim()} onclick={confirm}>
        {confirmed ? "Reviewed snapshot confirmed" : coverage === "partial" ? "Confirm partial snapshot" : "Confirm reviewed snapshot"}
      </button>
      <span>{coverage === "partial" ? "Partial means this source is known to omit positions. " : ""}Confirmation requires an explicit private alias for every instrument. Only aggregate aliases, quantities and prices are retained.</span>
    </div>
    {#if result.holdings.length > 0}
      <table>
        <thead><tr><th>Instrument</th><th class="num">Quantity</th><th class="num">Latest activity price</th></tr></thead>
        <tbody>
          {#each result.holdings as holding (holding.instrument)}
            <tr>
              <td>{holding.instrument}</td>
              <td class="num">{quantity(holding.quantity.mantissa, holding.quantity.scale)}</td>
              <td class="num">{holding.latest_unit_price === null ? "—" : `${quantity(holding.latest_unit_price.mantissa, holding.latest_unit_price.scale)} ${holding.currency}`}</td>
            </tr>
          {/each}
        </tbody>
      </table>
    {:else}
      <p class="empty">No open positions reconstructed.</p>
    {/if}
  {/if}
</section>

<style>
  .preview { margin-top: 2rem; border-top: 1px solid var(--border, #333); padding-top: 1.25rem; }
  .heading { display: flex; align-items: center; justify-content: space-between; gap: .75rem; flex-wrap: wrap; }
  h2 { margin: 0; font-size: 1rem; }
  p { margin: .25rem 0 0; color: var(--muted, #888); font-size: .8rem; }
  .file, button { display: inline-flex; align-items: center; gap: .35rem; border: 1px solid var(--border, #333); border-radius: 6px; padding: .4rem .65rem; font: inherit; font-size: .78rem; background: transparent; color: inherit; cursor: pointer; }
  .file input { display: none; }
  .mapping { display: grid; grid-template-columns: repeat(auto-fit, minmax(9rem, 1fr)); gap: .65rem; margin: 1rem 0; padding: .8rem; background: var(--card-bg, rgba(127,127,127,.05)); border-radius: 8px; }
  .mapping label { display: flex; flex-direction: column; gap: .2rem; color: var(--muted, #888); font-size: .68rem; }
  .mapping .profile { grid-column: span 2; }
  .mapping button { align-self: end; justify-content: center; }
  input, select { min-width: 0; border: 1px solid var(--border, #333); border-radius: 5px; padding: .35rem .45rem; font: inherit; font-size: .78rem; background: transparent; color: inherit; }
  button:disabled { opacity: .45; cursor: default; }
  button.partial { border-color: var(--warning, #a76b2c); }
  .error { color: var(--danger, #b44); }
  .summary { margin: 1rem 0 .65rem; }
  .review { display: flex; align-items: center; gap: .65rem; margin-bottom: .65rem; }
  .review span { color: var(--muted, #888); font-size: .7rem; }
  table { width: 100%; border-collapse: collapse; font-size: .78rem; }
  th, td { padding: .55rem .4rem; border-bottom: 1px solid var(--border, #333); text-align: left; }
  .num { text-align: right; font-variant-numeric: tabular-nums; }
  .empty { margin-top: 1rem; }
</style>
