import type {
  FinanceTransaction,
  TransactionCandidate,
  TripPlan,
} from "$lib/api";

export type ReviewQueueMode = "unreviewed" | "shared" | "all";

export type ExpenseReview = {
  candidate: TransactionCandidate;
  transaction: FinanceTransaction | null;
  totalCents: number;
  reviewed: boolean;
};

export type ReviewScope = {
  start: string;
  end: string;
  account: string;
  category: string;
};

export function reviewableExpenses(
  candidates: TransactionCandidate[],
  transactions: FinanceTransaction[],
  scope: ReviewScope,
): ExpenseReview[] {
  const transactionsBySource = new Map(
    transactions
      .filter((row) => row.source_id !== null)
      .map((row) => [row.source_id as string, row]),
  );
  return candidates
    .filter((candidate) => candidate.state === "confirmed"
      && candidate.amount_cents < 0
      && candidate.booked_at >= scope.start
      && candidate.booked_at <= scope.end
      && (scope.account === "" || candidate.source_account === scope.account)
      && (scope.category === "" || candidate.proposed_account === scope.category)
      && candidate.proposed_account.startsWith("expenses:")
      && !candidate.proposed_account.split(":").includes("uncategorized"))
    .map((candidate) => {
      const transaction = transactionsBySource.get(candidate.fingerprint) ?? null;
      return {
        candidate,
        transaction,
        totalCents: Math.abs(candidate.amount_cents),
        reviewed: transaction?.purpose !== null && transaction?.purpose !== undefined,
      };
    });
}

export function prioritizedExpenseQueue(
  entries: ExpenseReview[],
  mode: ReviewQueueMode,
  plan: Pick<TripPlan, "date_start" | "date_end"> | null,
): ExpenseReview[] {
  return entries
    .filter((entry) => plan === null
      || (entry.candidate.booked_at >= plan.date_start && entry.candidate.booked_at <= plan.date_end))
    .filter((entry) => mode === "all"
      || (mode === "unreviewed" && !entry.reviewed)
      || (mode === "shared" && (entry.transaction?.shared_cents ?? 0) > 0))
    .sort((left, right) => Number(left.reviewed) - Number(right.reviewed)
      || right.totalCents - left.totalCents
      || right.candidate.booked_at.localeCompare(left.candidate.booked_at));
}

export function parsePersonalCents(value: string): number | null {
  const normalized = value.trim().replace(",", ".");
  if (!/^\d+(?:\.\d{1,2})?$/.test(normalized)) return null;
  const cents = Math.round(Number(normalized) * 100);
  return Number.isSafeInteger(cents) ? cents : null;
}

export function presetShareCents(totalCents: number, percent: number): number {
  return Math.round(totalCents * percent / 100);
}
