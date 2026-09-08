// The Home ladder's finance kind: an investment proposal awaiting a call.
//
// WRITTEN AHEAD OF ITS REGISTRY, and the one thing to fix on merge is at the top
// of this file.
//
// Its row component ships beside it at `../rows/FinanceRow.svelte`, which is the
// name `view` below resolves to. That is not optional: the registry throws on a
// view name it cannot resolve and Home calls it inside a snippet with no
// boundary, so a kind without its row takes the whole page down rather than its
// own row.
//
// The Home registry's own two modules -- `decisions.ts` and `registry.ts`, in the
// directory above this one -- belong to the dashboard-refresh stream. They did not
// exist when this file was written; they do now, and the import below is the contract.
//
// Imports are RELATIVE, never `$lib`: the alias lives only in the generated
// tsconfig and the bun test job runs with no `svelte-kit sync`.
import { decisions, type Decision } from "../../finance/invest-api";
import { link } from "../../nav";

import type { DataClass, DecisionKind, LoadContext, ScoreContext } from "../decisions";

/// PRD §8.1's "Purchase decision" row. NOT a band this kind owns: the PRD gives
/// 640 to a renewal dated soon and to a budget overrun as well, so a band holds
/// several kinds. Its own stated justification is why an unsettled proposal
/// outranks a task — "Above tasks, because inaction is not free. A task left
/// undone stays undone. A subscription left undecided charges me."
export const BAND = 640;

/// Days-open, five points a day, capped at 50.
///
/// Halved and capped from the obvious `min(90, days * 10)` for an arithmetic
/// reason rather than a stylistic one: at ten points a day the row reaches 730 in
/// nine days and crosses the calendar band's 700 base, so an old proposal would
/// outrank a dated commitment. Capped here the row stays inside 640..690.
const URGENCY_PER_DAY = 5;
const URGENCY_CAP = 50;

const daysOpen = (row: Decision, ctx: ScoreContext): number => {
  const proposed = Date.parse(row.proposed_at);
  if (Number.isNaN(proposed)) return 0;
  return Math.max(0, Math.floor((ctx.nowMs - proposed) / 86_400_000));
};

export const rawUrgency = (row: Decision, ctx: ScoreContext): number =>
  Math.min(URGENCY_CAP, daysOpen(row, ctx) * URGENCY_PER_DAY);

const kind: DecisionKind<Decision[], Decision> = {
  key: "finance",
  band: BAND,
  label: "Finance",
  capability: "finance",
  lane: "commitment",

  // Only OPEN proposals reach Home. An accepted, rejected or superseded proposal
  // is a decision already made, and the ladder carries decisions that are owed.
  load: async (ctx) => decisions("open", ctx.signal),
  rows: (source) => source,
  id: (row) => row.id,

  // The contract's urgency scale is 0..999. The band table above is the ladder's
  // own 640..690 range, rescaled here so the shape is preserved exactly and only
  // the range changes.
  urgency: (row, ctx) =>
    Math.round((999 * rawUrgency(row, ctx)) / URGENCY_CAP),

  // The decision owed, and no valuation, no balance and no portfolio total. The
  // portfolio lives on /finance, for when the question is asked deliberately.
  title: (row) => row.proposal.title,
  view: "FinanceRow",
  href: () => link("/finance?view=investments"),

  whyHere: (row, ctx) => {
    const days = daysOpen(row, ctx);
    if (days === 0) return "Proposed today and not yet decided.";
    return `Open for ${days} day${days === 1 ? "" : "s"} without a decision.`;
  },
  startOrDueAt: (row) => row.proposed_at,
  candidateStatus: () => "proposed",
  // CONTRACT: the capability publishes this, so it is read and never computed
  // here. Every row in finance_decisions carries a validated class.
  dataClass: (row) =>
    (["c0", "c1", "c2", "c3"] as const).find((value) => value === row.data_class) ?? null,
  // CONTRACT: finance runs no model call on this path. The rule engine is local
  // arithmetic and the risk model is local linear algebra, so neither "local" nor
  // "cloud" describes a routing decision that was made — null is the honest
  // answer, matching every other kind today.
  processingRoute: () => null,
  scoreFactors: (row, ctx) => [
    { label: "Days open", score: rawUrgency(row, ctx) / URGENCY_CAP },
  ],
};

export default kind;
