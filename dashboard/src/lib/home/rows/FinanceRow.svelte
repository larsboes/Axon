<script lang="ts">
  import type { Decision } from "../../finance/invest-api";
  import Icon from "../../Icon.svelte";
  import ListRow from "../../ListRow.svelte";
  import RowMeta from "../../RowMeta.svelte";
  import { metaParts } from "../format";
  import type { DecisionRowProps } from "../decisions";

  /**
   * The ladder row `kinds/finance.ts` names through `view: "FinanceRow"`.
   *
   * MERGE NOTE DISCHARGED, 2026-09-08: this row was written on a branch where
   * `ListRow`, `RowMeta` and `DecisionRowProps` did not exist, so it drew its own
   * markup and typed its props by hand. Both primitives are here now, and the note
   * asked for exactly this swap. The visible consequence is the one B50 was about:
   * the props declared `dataClass`, the row never destructured it, and the class
   * `finance_decisions` validates on every row reached the page and rendered nothing.
   * A chip that exists in a type and not in a pixel is not a published class.
   *
   * The row still carries the decision owed and no valuation, no balance and no
   * portfolio total (PRD:2233). Accept and Reject live on /finance, because Home
   * does not write.
   */
  let {
    row,
    id,
    current,
    tone,
    href,
    whyHere,
    dataClass,
    candidateStatus,
  }: DecisionRowProps<Decision, Decision[]> = $props();

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

  const kind = $derived(metaParts("Investment", row.proposal.kind, figure));
</script>

<ListRow {id} {current} {tone} {href}>
  {#snippet mark()}<Icon name="wallet" size={15} />{/snippet}

  <span class="row-kind">{#each kind as part}<span>{part}</span>{/each}</span>
  <a class="row-title" {href}>{row.proposal.title}</a>

  {#snippet meta()}
    <!-- `rung` rather than `model_revision`: the kind states that this path runs no
         model call, so naming a revision beside the class would credit a rank to
         something that never ran. The rung is which ladder produced the proposal. -->
    <RowMeta {whyHere} {dataClass} {candidateStatus} method={row.rung} />
  {/snippet}

  {#snippet actions()}
    <a class="btn btn-soft" {href}>Decide</a>
  {/snippet}
</ListRow>
