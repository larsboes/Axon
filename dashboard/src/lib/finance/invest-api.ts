// The investment surfaces' own HTTP client.
//
// `dashboard/src/lib/api.ts` is not edited. Its `request` helper is not
// exported, so this module has its own nine-line copy over the two things that
// ARE exported — `ApiError` and `describeFailure` — which is what keeps one
// failure message for the whole dashboard rather than two.
//
// The import is RELATIVE and must stay relative. `.svelte-kit/` is gitignored,
// the `$lib` alias lives only in the generated tsconfig, and the bun test job
// runs with no install and no `svelte-kit sync`. `ApiError` is a VALUE import,
// which bun would have to resolve — a `$lib` specifier would pass locally and
// fail CI.
import { ApiError, describeFailure } from "../api";

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const res = await fetch(path, init);
  if (!res.ok) {
    const body = await res.text().catch(() => "");
    throw new ApiError(res.status, describeFailure(res.status, body, path));
  }
  const text = await res.text();
  const parsed = text ? JSON.parse(text) : undefined;
  if (parsed && typeof parsed === "object" && "error" in parsed && parsed.error) {
    throw new ApiError(res.status, String(parsed.error));
  }
  return parsed as T;
}

const jsonInit = (method: string, body: unknown): RequestInit => ({
  method,
  headers: { "Content-Type": "application/json" },
  body: JSON.stringify(body),
});

/** An exact decimal. The mantissa is a STRING on the wire and stays one here:
 *  parsing it as a number is where an exact decimal stops being exact. */
export interface Decimal {
  mantissa: string;
  scale: number;
}

/** `fresh` and `stale` both mean a market price exists. `none` means the
 *  position is valued at its reviewed broker price and no quote was found. */
export type PriceFreshness = "fresh" | "stale" | "none";

export interface Position {
  instrument: string;
  label: string;
  asset_class: string;
  quantity: Decimal;
  currency: string;
  review_price: Decimal | null;
  market_price: Decimal | null;
  market_price_source: string | null;
  market_price_observed_on: string | null;
  price_age_days: number | null;
  price_freshness: PriceFreshness;
  value: Decimal;
  value_basis: "market" | "review" | "none";
  share_bp: number;
  target_bp: number | null;
  band_bp: number | null;
  drift_bp: number | null;
  outside_band: boolean;
  /** Against the reviewed broker price. NOT a return and NOT P&L: there is no
   *  lot and no cost basis anywhere in the finance capability. */
  change_since_review: Decimal | null;
  change_since_review_bp: number | null;
}

export interface AssetClassRow {
  asset_class: string;
  value: Decimal;
  share_bp: number;
  target_bp: number | null;
  band_bp: number | null;
  drift_bp: number | null;
  outside_band: boolean;
}

export interface Portfolio {
  as_of: string;
  currency: string;
  coverage: string;
  /** False renders as "no policy declared yet" — never as a table of zero
   *  drift, which reads as a portfolio on target. */
  targets_configured: boolean;
  total: Decimal;
  priced_positions: number;
  unpriced_positions: number;
  positions: Position[];
  asset_classes: AssetClassRow[];
  caveats: string[];
}

export interface FeedEvidenceItem {
  id: string;
  title: string;
  url: string;
  day: string;
}

export interface InstrumentRisk {
  instrument: string;
  observations: number;
  annualised_volatility_bp: number | null;
  annualised_mean_bp: number | null;
  status: string;
  detail: string | null;
}

export interface RiskEvidence {
  trading_days: number;
  min_observations: number;
  min_overlap: number;
  instruments: InstrumentRisk[];
  correlations: {
    left: string;
    right: string;
    overlap: number;
    correlation_bp: number | null;
    status: string;
  }[];
  portfolio_volatility_bp: number | null;
  minimum_variance_weights: { instrument: string; weight_bp: number; current_bp: number | null }[] | null;
  max_sharpe_weights: { instrument: string; weight_bp: number; current_bp: number | null }[] | null;
  caveats: string[];
}

export interface ProposalBody {
  kind: string;
  subject: string;
  title: string;
  summary: string;
  rung: string;
  instrument: string | null;
  asset_class: string | null;
  actual_bp: number | null;
  target_bp: number | null;
  band_bp: number | null;
  drift_bp: number | null;
  amount_cents: number | null;
  currency: string;
}

export interface EvidenceBody {
  numbers: Record<string, number>;
  feed_items: FeedEvidenceItem[];
  caveats: string[];
  risk?: RiskEvidence | null;
}

export interface Decision {
  id: string;
  kind: string;
  subject: string;
  rung: string;
  data_class: string;
  data_class_rationale: string;
  model_revision: string;
  proposed_at: string;
  status: "open" | "accepted" | "rejected" | "superseded";
  verdict: string | null;
  verdict_at: string | null;
  verdict_note: string | null;
  reviewed_at: string | null;
  proposal: ProposalBody;
  evidence: EvidenceBody;
}

export interface PriceStatus {
  as_of: string;
  freshness_days: number;
  instruments: {
    instrument: string;
    latest_observed_on: string;
    source: string;
    age_days: number | null;
    freshness: string;
    observations: number;
  }[];
  recent_fetches: {
    provider: string;
    target: string;
    status: string;
    detail: string;
    rows_written: number;
    fetched_at: string;
  }[];
  providers: {
    name: string;
    last_status: string | null;
    last_detail: string | null;
    last_fetched_at: string | null;
  }[];
}

/** The six fields `analytics::TripSpendingSummary` defines, at the top level,
 *  plus the currency they are denominated in. The travel surfaces read this
 *  shape rather than downloading the whole projection to pick one object. */
export interface TripSpending {
  trip_id: string;
  personal_spending_cents: number;
  gross_cash_outflow_cents: number;
  reimbursed_cents: number;
  outstanding_cents: number;
  expense_posting_count: number;
  currency: string;
}

export interface DecisionRunResult {
  ok: boolean;
  dry_run: boolean;
  proposed: number;
  unchanged: number;
  superseded: number;
  caveats: string[];
}

export interface VerdictResult {
  ok: boolean;
  id: string;
  status: string;
  recorded_at: string;
  exported_to: string | null;
}

export const portfolio = (currency = "EUR", signal?: AbortSignal) =>
  request<Portfolio>(
    `/finance/api/portfolio?currency=${encodeURIComponent(currency)}`,
    signal ? { signal } : undefined,
  );

export const decisions = (status = "open", signal?: AbortSignal) =>
  request<Decision[]>(
    `/finance/api/decisions?status=${encodeURIComponent(status)}`,
    signal ? { signal } : undefined,
  );

export const priceStatus = (signal?: AbortSignal) =>
  request<PriceStatus>("/finance/api/prices/status", signal ? { signal } : undefined);

export const runDecisions = (dryRun = false) =>
  request<DecisionRunResult>("/finance/api/decisions/run", jsonInit("POST", { dry_run: dryRun }));

/** `expectedProposalId` is re-derived server-side and compared. A mismatch is a
 *  409 carrying the current id, never a silently recorded verdict on numbers
 *  that have moved. */
export const recordVerdict = (
  id: string,
  body: { expected_proposal_id: string; verdict: "accepted" | "rejected"; note: string },
) => request<VerdictResult>(`/finance/api/decisions/${encodeURIComponent(id)}/verdict`, jsonInit("POST", body));

export const tripSpending = (planId: string, signal?: AbortSignal) =>
  request<TripSpending>(
    `/finance/api/trips/${encodeURIComponent(planId)}/spending`,
    signal ? { signal } : undefined,
  );
