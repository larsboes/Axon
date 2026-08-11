import { describe, expect, test } from "bun:test";
import type { FinanceTransaction, TransactionCandidate } from "../src/lib/api";
import {
  parsePersonalCents,
  presetShareCents,
  prioritizedExpenseQueue,
  reviewableExpenses,
} from "../src/lib/finance/spending-review";

function candidate(
  id: string,
  amountCents: number,
  bookedAt: string,
  account = "expenses:travel:transport",
): TransactionCandidate {
  return {
    id,
    fingerprint: `source-${id}`,
    booked_at: bookedAt,
    description: `Synthetic expense ${id}`,
    amount_cents: amountCents,
    currency: "EUR",
    source_account: "assets:bank:checking",
    source_reference: null,
    proposed_account: account,
    confidence_basis_points: 10_000,
    state: "confirmed",
    transfer_match_ids: [],
  };
}

function transaction(id: string, purpose: FinanceTransaction["purpose"], sharedCents = 0): FinanceTransaction {
  return {
    id: `row-${id}`,
    date: "2025-12-05",
    description: `Synthetic expense ${id}`,
    kind: "expense",
    account: "assets:bank:checking",
    category: "expenses:travel:transport",
    amount_cents: 25_00,
    currency: "EUR",
    source_id: `source-${id}`,
    purpose,
    trip_id: purpose === "trip" ? "trip:synthetic" : null,
    cash_amount_cents: 100_00,
    shared_cents: sharedCents,
    reimbursement_for: null,
  };
}

describe("Finance spending review queue", () => {
  test("only exposes categorized confirmed expenses in the active scope", () => {
    const entries = reviewableExpenses([
      candidate("included", -100_00, "2025-12-05"),
      candidate("uncategorized", -200_00, "2025-12-05", "expenses:uncategorized"),
      candidate("outside", -300_00, "2025-10-01"),
      candidate("inflow", 400_00, "2025-12-05", "income:salary"),
    ], [], {
      start: "2025-12-01",
      end: "2025-12-31",
      account: "assets:bank:checking",
      category: "",
    });

    expect(entries.map((entry) => entry.candidate.id)).toEqual(["included"]);
  });

  test("puts the largest unresolved expense first and respects a trip date window", () => {
    const entries = reviewableExpenses([
      candidate("small", -20_00, "2025-12-03"),
      candidate("large", -120_00, "2025-12-04"),
      candidate("reviewed", -300_00, "2025-12-05"),
      candidate("outside-trip", -500_00, "2025-12-20"),
    ], [transaction("reviewed", "trip")], {
      start: "2025-12-01",
      end: "2025-12-31",
      account: "",
      category: "",
    });

    expect(prioritizedExpenseQueue(entries, "all", {
      date_start: "2025-12-02",
      date_end: "2025-12-10",
    }).map((entry) => entry.candidate.id)).toEqual(["large", "small", "reviewed"]);
  });

  test("shared mode contains only reviewed allocations with a shared receivable", () => {
    const entries = reviewableExpenses([
      candidate("shared", -100_00, "2025-12-05"),
      candidate("personal", -80_00, "2025-12-05"),
    ], [
      transaction("shared", "trip", 75_00),
      transaction("personal", "day_to_day"),
    ], {
      start: "2025-12-01",
      end: "2025-12-31",
      account: "",
      category: "",
    });

    expect(prioritizedExpenseQueue(entries, "shared", null)
      .map((entry) => entry.candidate.id)).toEqual(["shared"]);
  });

  test("personal-share helpers preserve cents and reject ambiguous input", () => {
    expect(presetShareCents(99_99, 25)).toBe(25_00);
    expect(parsePersonalCents("25,00")).toBe(25_00);
    expect(parsePersonalCents("25.001")).toBeNull();
    expect(parsePersonalCents("-1,00")).toBeNull();
  });
});
