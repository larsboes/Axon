<script lang="ts">
  import Icon from "$lib/Icon.svelte";
  import { finance, type CsvMapping, type TransactionCandidate } from "$lib/api";

  let { onchanged }: { onchanged?: () => void } = $props();
  let candidates = $state<TransactionCandidate[]>([]);
  let content = $state("");
  let filename = $state("");
  let busy = $state(false);
  let error = $state<string | null>(null);
  let notice = $state<string | null>(null);
  let accounts = $state<Record<string, string>>({});
  let mapping = $state<CsvMapping>({
    delimiter: ";",
    decimal_separator: ",",
    date_column: "Buchung",
    amount_column: "Betrag",
    description_column: "Verwendungszweck",
    reference_column: null,
    currency_column: "Währung",
    default_currency: "EUR",
    source_account: "assets:bank:checking",
  });

  async function load() {
    try {
      candidates = await finance.candidates();
      accounts = Object.fromEntries(candidates.map((candidate) => [
        candidate.id,
        accounts[candidate.id] ?? candidate.proposed_account,
      ]));
      error = null;
    } catch (cause) {
      error = cause instanceof Error ? cause.message : String(cause);
    }
  }

  async function choose(event: Event) {
    const input = event.currentTarget as HTMLInputElement;
    const file = input.files?.[0];
    if (!file) return;
    filename = file.name;
    content = await file.text();
  }

  async function stage() {
    if (!content) return;
    busy = true;
    notice = null;
    try {
      const result = await finance.importCsv(content, {
        ...mapping,
        reference_column: mapping.reference_column?.trim() || null,
        currency_column: mapping.currency_column?.trim() || null,
      });
      notice = `${result.created} staged, ${result.already_present} already known. Raw CSV rows were discarded.`;
      content = "";
      filename = "";
      await load();
    } catch (cause) {
      error = cause instanceof Error ? cause.message : String(cause);
    } finally {
      busy = false;
    }
  }

  async function review(candidate: TransactionCandidate, decision: "confirm" | "reject") {
    busy = true;
    notice = null;
    try {
      const result = await finance.reviewCandidate(
        candidate.id,
        decision,
        decision === "confirm" ? accounts[candidate.id] : undefined,
      );
      notice = result.state === "confirmed"
        ? result.journal_written
          ? "Confirmed, written to the journal, and projection rebuilt."
          : "Already present in the journal; review state repaired and projection rebuilt."
        : "Rejected. The journal was not changed.";
      await load();
      onchanged?.();
    } catch (cause) {
      error = cause instanceof Error ? cause.message : String(cause);
    } finally {
      busy = false;
    }
  }

  $effect(() => { void load(); });
</script>

<section class="review">
  <div class="heading">
    <div>
      <h2>Import and review</h2>
      <p>Choose an export locally, map its columns, then review every candidate. Nothing confirms itself.</p>
    </div>
    <label class="file">
      <Icon name="plus" size={14} /> {filename || "Choose CSV"}
      <input type="file" accept=".csv,text/csv" onchange={choose} />
    </label>
  </div>

  {#if content}
    <div class="mapping">
      <label>Delimiter<input maxlength="1" bind:value={mapping.delimiter} /></label>
      <label>Decimal separator<input maxlength="1" bind:value={mapping.decimal_separator} /></label>
      <label>Date column<input bind:value={mapping.date_column} /></label>
      <label>Amount column<input bind:value={mapping.amount_column} /></label>
      <label>Description column<input bind:value={mapping.description_column} /></label>
      <label>Reference column<input bind:value={mapping.reference_column} placeholder="optional" /></label>
      <label>Currency column<input bind:value={mapping.currency_column} placeholder="optional" /></label>
      <label>Default currency<input maxlength="3" bind:value={mapping.default_currency} /></label>
      <label class="account">Source account<input bind:value={mapping.source_account} /></label>
      <button disabled={busy} onclick={stage}>Stage candidates</button>
    </div>
  {/if}

  {#if error}<p class="error">{error}</p>{/if}
  {#if notice}<p class="notice">{notice}</p>{/if}

  {#if candidates.length > 0}
    <div class="candidate-list">
      {#each candidates as candidate (candidate.id)}
        <article class:resolved={candidate.state !== "pending"}>
          <div class="candidate-main">
            <time>{candidate.booked_at}</time>
            <strong>{candidate.description}</strong>
            <span class:outflow={candidate.amount_cents < 0} class="amount">
              {new Intl.NumberFormat("de-DE", { style: "currency", currency: candidate.currency }).format(candidate.amount_cents / 100)}
            </span>
          </div>
          {#if candidate.state === "pending"}
            <div class="decision">
              <input aria-label="Reviewed account" bind:value={accounts[candidate.id]} />
              <button disabled={busy || !accounts[candidate.id]?.trim()} onclick={() => review(candidate, "confirm")}>
                <Icon name="check" size={14} /> Confirm
              </button>
              <button class="reject" disabled={busy} onclick={() => review(candidate, "reject")}>Reject</button>
            </div>
          {:else}
            <span class="state">{candidate.state}</span>
          {/if}
        </article>
      {/each}
    </div>
  {:else}
    <p class="empty">No candidates staged.</p>
  {/if}
</section>

<style>
  .review { margin-top: 2rem; border-top: 1px solid var(--border, #333); padding-top: 1.25rem; }
  .heading, .candidate-main, .decision { display: flex; align-items: center; gap: .75rem; }
  .heading { justify-content: space-between; flex-wrap: wrap; }
  h2 { margin: 0; font-size: 1rem; }
  p { margin: .25rem 0 0; color: var(--muted, #888); font-size: .8rem; }
  .file, button { display: inline-flex; align-items: center; gap: .35rem; border: 1px solid var(--border, #333); border-radius: 6px; padding: .4rem .65rem; font: inherit; font-size: .78rem; background: transparent; color: inherit; cursor: pointer; }
  .file input { display: none; }
  .mapping { display: grid; grid-template-columns: repeat(auto-fit, minmax(9rem, 1fr)); gap: .65rem; margin: 1rem 0; padding: .8rem; background: var(--card-bg, rgba(127,127,127,.05)); border-radius: 8px; }
  .mapping label { display: flex; flex-direction: column; gap: .2rem; color: var(--muted, #888); font-size: .68rem; }
  input { min-width: 0; border: 1px solid var(--border, #333); border-radius: 5px; padding: .35rem .45rem; font: inherit; font-size: .78rem; background: transparent; color: inherit; }
  .mapping .account { grid-column: span 2; }
  .mapping button { align-self: end; justify-content: center; }
  .candidate-list { display: grid; gap: .4rem; margin-top: 1rem; }
  article { display: flex; align-items: center; justify-content: space-between; gap: 1rem; padding: .65rem .75rem; border: 1px solid var(--border, #333); border-radius: 7px; }
  article.resolved { opacity: .55; }
  .candidate-main { min-width: 0; flex: 1; }
  time, .state { color: var(--muted, #888); font-size: .72rem; }
  strong { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-size: .82rem; }
  .amount { margin-left: auto; font-variant-numeric: tabular-nums; font-size: .82rem; color: var(--primary); }
  .amount.outflow { color: inherit; }
  .decision { flex-wrap: wrap; }
  .decision input { width: 14rem; }
  button.reject { border-color: transparent; color: var(--muted, #888); }
  button:disabled { opacity: .45; cursor: default; }
  .error { color: var(--danger, #b44); }
  .notice { color: var(--primary); }
  .empty { margin-top: 1rem; }
  @media (max-width: 760px) { article, .candidate-main { align-items: flex-start; flex-direction: column; } .amount { margin-left: 0; } .decision, .decision input { width: 100%; } }
</style>
