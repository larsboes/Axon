<script lang="ts">
  import Icon from "$lib/Icon.svelte";
  import {
    finance,
    type BalanceCoverage,
    type ManualBalance,
    type ManualBalanceSnapshot,
  } from "$lib/api";

  type DraftBalance = Omit<ManualBalance, "amount_cents"> & { amount: string };

  let { snapshot, onsaved }: {
    snapshot: ManualBalanceSnapshot | null;
    onsaved?: () => void;
  } = $props();
  let asOf = $state("");
  let currency = $state("EUR");
  let coverage = $state<BalanceCoverage>("partial");
  let balances = $state<DraftBalance[]>([]);
  let loadedVersion = $state("");
  let busy = $state(false);
  let error = $state<string | null>(null);
  let notice = $state<string | null>(null);

  function amountText(cents: number): string {
    return `${Math.trunc(cents / 100)},${String(cents % 100).padStart(2, "0")}`;
  }

  function parseAmount(value: string): number | null {
    const match = /^(\d+)(?:[,.](\d{1,2}))?$/.exec(value.trim());
    if (!match) return null;
    const cents = Number(match[1]) * 100 + Number((match[2] ?? "").padEnd(2, "0"));
    return Number.isSafeInteger(cents) ? cents : null;
  }

  function updatedText(value: string): string {
    const date = new Date(value);
    return Number.isNaN(date.getTime())
      ? value
      : new Intl.DateTimeFormat("de-DE", { dateStyle: "medium", timeStyle: "short" }).format(date);
  }

  function addBalance() {
    balances = [
      ...balances,
      {
        id: crypto.randomUUID(),
        label: "",
        kind: "asset",
        amount: "0,00",
      },
    ];
  }

  function removeBalance(id: string) {
    balances = balances.filter((balance) => balance.id !== id);
  }

  async function save() {
    const parsed = balances.map((balance) => ({ ...balance, amount_cents: parseAmount(balance.amount) }));
    if (parsed.length === 0 || parsed.some((balance) => balance.amount_cents === null)) {
      error = "Enter at least one non-negative balance with at most two decimal places.";
      return;
    }
    busy = true;
    error = null;
    notice = null;
    try {
      await finance.updateBalanceSnapshot({
        as_of: asOf,
        currency,
        coverage,
        balances: parsed.map(({ amount, amount_cents, ...balance }) => ({
          ...balance,
          amount_cents: amount_cents as number,
        })),
      });
      notice = "Balance snapshot updated.";
      loadedVersion = "";
      onsaved?.();
    } catch (cause) {
      error = cause instanceof Error ? cause.message : String(cause);
    } finally {
      busy = false;
    }
  }

  $effect(() => {
    const version = snapshot?.updated_at ?? "empty";
    if (version === loadedVersion) return;
    loadedVersion = version;
    asOf = snapshot?.as_of ?? new Date().toISOString().slice(0, 10);
    currency = snapshot?.currency ?? "EUR";
    coverage = snapshot?.coverage ?? "partial";
    balances = (snapshot?.balances ?? []).map((balance) => ({
      id: balance.id,
      label: balance.label,
      kind: balance.kind,
      amount: amountText(balance.amount_cents),
    }));
  });
</script>

<section class="balances">
  <div class="heading">
    <div>
      <h2>Manual balances</h2>
      <p>
        Point-in-time account values, separate from transaction-derived cash flow.
        {#if snapshot} Last updated {updatedText(snapshot.updated_at)}.{/if}
      </p>
    </div>
    <label>As of<input type="date" bind:value={asOf} /></label>
  </div>

  {#if balances.length === 0}
    <p class="empty">No manual balance snapshot yet.</p>
  {:else}
    <div class="rows">
      {#each balances as balance (balance.id)}
        <div class="row">
          <input class="label" aria-label="Balance label" bind:value={balance.label} />
          <select aria-label="Balance kind" bind:value={balance.kind}>
            <option value="asset">Asset</option>
            <option value="liability">Liability</option>
          </select>
          <div class="amount"><input aria-label={`${balance.label || "Balance"} amount`} inputmode="decimal" bind:value={balance.amount} /><span>{currency}</span></div>
          <button class="remove" type="button" onclick={() => removeBalance(balance.id)}>Remove</button>
        </div>
      {/each}
    </div>
  {/if}

  <div class="actions">
    <button type="button" onclick={addBalance}><Icon name="plus" size={13} /> Add balance</button>
    <label>Coverage<select bind:value={coverage}>
      <option value="partial">Incomplete</option>
      <option value="complete">Complete</option>
    </select></label>
    <button class="save" type="button" disabled={busy || balances.length === 0 || !asOf} onclick={save}>
      <Icon name="check" size={13} /> Save snapshot
    </button>
  </div>
  {#if coverage === "partial"}<p class="warning">Tracked net worth is incomplete until all material assets and liabilities are represented.</p>{/if}
  {#if error}<p class="error">{error}</p>{/if}
  {#if notice}<p class="notice">{notice}</p>{/if}
</section>

<style>
  .balances { margin-bottom: .75rem; padding: .9rem; border: 1px solid var(--border, #333); border-radius: 8px; background: var(--card-bg, transparent); }
  .heading, .actions, .row { display: flex; align-items: center; gap: .65rem; }
  .heading { align-items: start; justify-content: space-between; }
  h2 { margin: 0; font-size: .85rem; }
  p { margin: .2rem 0 0; color: var(--muted, #888); font-size: .7rem; }
  label { display: flex; flex-direction: column; gap: .2rem; color: var(--muted, #888); font-size: .68rem; }
  input, select, button { min-width: 0; border: 1px solid var(--border, #333); border-radius: 6px; padding: .38rem .55rem; font: inherit; font-size: .76rem; background: transparent; color: inherit; }
  button { display: inline-flex; align-items: center; justify-content: center; gap: .3rem; cursor: pointer; }
  button:disabled { opacity: .45; cursor: default; }
  .rows { display: grid; gap: .4rem; margin-top: .8rem; }
  .row { display: grid; grid-template-columns: minmax(11rem, 1fr) 7rem minmax(9rem, .5fr) auto; }
  .amount { display: flex; align-items: center; }
  .amount input { width: 100%; border-radius: 6px 0 0 6px; text-align: right; font-variant-numeric: tabular-nums; }
  .amount span { align-self: stretch; display: grid; place-items: center; padding: 0 .45rem; border: 1px solid var(--border, #333); border-left: 0; border-radius: 0 6px 6px 0; color: var(--muted, #888); font-size: .68rem; }
  .remove { border-color: transparent; color: var(--muted, #888); }
  .actions { margin-top: .7rem; flex-wrap: wrap; }
  .actions label { margin-left: auto; }
  .save { align-self: end; }
  .warning { color: var(--warning, #a76b2c); }
  .error { color: var(--danger, #b44); }
  .notice { color: var(--primary); }
  @media (max-width: 720px) { .heading { flex-direction: column; } .row { grid-template-columns: 1fr 7rem; } .amount { grid-column: 1 / -1; } .actions label { margin-left: 0; } }
</style>
