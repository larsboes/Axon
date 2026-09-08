<script lang="ts">
  import { link } from "$lib/nav";
  import type { Decision } from "$lib/finance/invest-api";

  /**
   * The ladder row `kinds/finance.ts` names through `view: "FinanceRow"`.
   *
   * MERGE NOTE, 2026-09-06: this file lands in a directory the dashboard-refresh
   * stream owns and had not created when it was written. `registry.ts` there
   * resolves `view` through `import.meta.glob` over this directory and THROWS on
   * a name it cannot resolve, inside a snippet with no boundary — so a kind
   * without its row takes the whole Home page down, not just its own row.
   *
   * It is deliberately self-contained: it renders its own markup instead of that
   * stream's `ListRow`/`RowMeta` primitives, which do not exist on this branch
   * and could not be imported without failing `bun run check` here. Re-point it
   * at those primitives on merge, the way `TripRow.svelte` uses them; the props
   * below are `DecisionRowProps<Decision>` minus the fields this row does not
   * read, so the swap is an import and a type, not a rewrite.
   *
   * The row carries the decision owed and no valuation, no balance and no
   * portfolio total (PRD:2233). Accept and Reject live on /finance, because Home
   * does not write.
   */
  let {
    row,
    href = link("/finance?view=investments"),
    whyHere = "",
  }: {
    row: Decision;
    id?: string;
    current?: boolean;
    tone?: string;
    href?: string;
    whyHere?: string;
    dataClass?: string | null;
    processingRoute?: "local" | "cloud" | null;
    candidateStatus?: "proposed" | "accepted" | "open";
    busy?: boolean;
    act?: (run: () => Promise<void>, options?: { dismiss?: boolean }) => void;
  } = $props();

  // Formatting only. Every number here is computed by the capability: the drift
  // arrives in basis points and the amount in cents, and neither is derived,
  // rounded into a different unit or compared against anything on this side.
  const money = (cents: number, currency: string) =>
    new Intl.NumberFormat("de-DE", { style: "currency", currency }).format(cents / 100);
  const signedPercent = (bp: number) => `${bp > 0 ? "+" : ""}${(bp / 100).toFixed(2)}%`;

  const figure = $derived.by(() => {
    const proposal = row.proposal;
    if (proposal.drift_bp !== null) return `drift ${signedPercent(proposal.drift_bp)}`;
    if (proposal.amount_cents !== null) return money(proposal.amount_cents, proposal.currency);
    return null;
  });
</script>

<div class="row">
  <div>
    <a class="title" {href}>{row.proposal.title}</a>
    <p class="why">
      {row.proposal.kind}
      {#if figure}· {figure}{/if}
      {#if whyHere}· {whyHere}{/if}
    </p>
  </div>
  <a class="action" {href}>Decide</a>
</div>

<style>
  .row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.75rem;
    width: 100%;
  }

  .title {
    font-weight: 600;
    color: var(--text-primary);
    text-decoration: none;
  }

  .title:hover {
    text-decoration: underline;
  }

  .why {
    margin: 0.1rem 0 0;
    font-size: 0.75rem;
    color: var(--text-tertiary);
  }

  .action {
    flex: none;
    padding: 0.3rem 0.7rem;
    border: 1px solid var(--card-border);
    border-radius: var(--radius-sm);
    font-size: 0.8rem;
    color: var(--text-secondary);
    text-decoration: none;
  }
</style>
