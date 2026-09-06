/** The mail model rung's client surface.
 *
 *  A stream-owned module rather than three more functions in `$lib/api.ts`:
 *  that file is 3,500 lines and four streams edit it on the same night. The
 *  TYPES stay there, because `TriageItem` is the reader contract for
 *  `GET /triage` and a type-only import cycle to move one field would cost more
 *  than it saves.
 */
import { jsonInit, request, type MailCategory, type TriageModelVerdict } from '../api';

export type { TriageModelVerdict };

/** What the model rung did on one pass. Counts only: no subject, no snippet,
 *  no rationale ever reaches this shape, because the store query behind it
 *  cannot select them. */
export interface MailClassifyPass {
  mode: string;
  /** How many threads this pass was allowed to act on: the request's own limit,
   *  else the overlay's `mail_model.limit`, else 200. */
  limit: number;
  reviewed: number;
  eligible: number;
  /** Stored disagreements with no category write yet. What a shadow pass leaves
   *  for the operator to decide on, and what an apply pass then acts on. */
  awaiting_apply: number;
  prompted: number;
  refused_c3: number;
  over_window: number;
  unparseable: number;
  invalid_stream: number;
  errors: number;
  agreed_no_write: number;
  disagreed: number;
  applied: number;
  held_class_escalation: number;
  below_confidence: number;
  redactions: number;
  model: { producer: string; prompt_revision: string; loopback: boolean };
  classifier_version: string;
  rules_version: string;
  cloud_calls: number;
}

export interface MailStreamAgreement {
  rule_stream: string;
  n: number;
  agree: number;
  agree_percent: number | null;
  model_streams: { stream: string; n: number }[];
}

export interface TriageClassifyReport {
  verdicts: number;
  /** Threads the rung may look at at all. Not the same number as a pass's
   *  `eligible`, which is how many were due on that run. */
  candidates: number;
  by_rule_stream: MailStreamAgreement[];
  by_state: { state: string; n: number }[];
  /** What actually classified the open mailbox, counted by the capability. */
  by_classification_method: { method: string; n: number }[];
  by_data_class: { data_class: string; n: number; prompted: number }[];
  held: { reason: string; n: number }[];
  confidence_bp: { min: number | null; median: number | null; max: number | null };
  urgency_bp: {
    scored: number;
    min: number | null;
    median: number | null;
    max: number | null;
    /** False until the frozen corpus carries a measured band error. */
    validated: boolean;
  };
  apply_enabled: boolean;
  producer: string;
  prompt_revision: string;
  cloud_calls: number;
}

export const refreshMailClassification = (
  mode: 'shadow' | 'apply' = 'shadow',
  limit = 200
): Promise<MailClassifyPass> =>
  request('/comms/triage/classify/refresh', jsonInit('POST', { mode, limit }));

export const mailClassificationReport = (
  mode?: 'shadow' | 'applied' | 'held'
): Promise<TriageClassifyReport> =>
  request(`/comms/triage/classify/report${mode ? `?mode=${mode}` : ''}`);

export const revertMailClassification = (
  ids: string[] | 'all'
): Promise<{ reverted: number; class_unchanged: boolean; note: string }> =>
  request(
    '/comms/triage/classify/revert',
    jsonInit('POST', ids === 'all' ? { all: true } : { ids })
  );

/** The agreement table as a sorted table model, so the page renders and does not
 *  compute. Frontend renders, backend computes: the percentages arrive from the
 *  capability and this only orders them. */
export function agreementRows(report: TriageClassifyReport | null): MailStreamAgreement[] {
  if (!report) return [];
  return [...report.by_rule_stream].sort(
    (a, b) => b.n - a.n || a.rule_stream.localeCompare(b.rule_stream)
  );
}

/** Whether this verdict is worth showing an Accept button for.
 *
 *  Only a real disagreement, and it is measured against the category the row
 *  HOLDS — not against `rule_stream`, which is the deterministic verdict frozen
 *  at prompt time and is never rewritten. Comparing with the frozen one kept the
 *  card offering "Accept Advertising" after the accept had already happened
 *  (review, 2026-09-05). */
export function proposesAChange(
  verdict: TriageModelVerdict | null | undefined,
  current: MailCategory
): boolean {
  return Boolean(
    verdict &&
      verdict.state === 'generated' &&
      verdict.model_stream &&
      verdict.model_stream !== current
  );
}
