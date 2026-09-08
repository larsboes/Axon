import { describe, expect, test } from 'bun:test';
import {
  agreementRows,
  proposesAChange,
  type TriageClassifyReport,
  type TriageModelVerdict,
} from '../src/lib/mail/api';

const report = (rows: TriageClassifyReport['by_rule_stream']): TriageClassifyReport => ({
  verdicts: rows.reduce((sum, row) => sum + row.n, 0),
  candidates: 0,
  by_rule_stream: rows,
  by_state: [],
  by_classification_method: [],
  by_data_class: [],
  held: [],
  confidence_bp: { min: null, median: null, max: null },
  urgency_bp: { scored: 0, min: null, median: null, max: null, validated: false },
  apply_enabled: false,
  producer: 'foundation-models:apple:mail-stream-v1-english',
  prompt_revision: 'mail-stream-v1-english',
  cloud_calls: 0,
});

const verdict = (over: Partial<TriageModelVerdict> = {}): TriageModelVerdict => ({
  mode: 'shadow',
  state: 'generated',
  rule_stream: 'aktiv',
  model_stream: 'feed',
  confidence_bp: 8_500,
  urgency_bp: 0,
  urgency_validated: false,
  rationale: 'It is a newsletter.',
  urgency_rationale: 'Nothing is asked.',
  data_class: 'c1',
  held_reason: null,
  classification_version: 'mail-model-v1',
  applied_at: null,
  ...over,
});

describe('agreementRows', () => {
  test('sorts by sample size and keeps the capability-computed percentages', () => {
    const rows = agreementRows(
      report([
        { rule_stream: 'feed', n: 4, agree: 4, agree_percent: 100, model_streams: [] },
        { rule_stream: 'aktiv', n: 102, agree: 37, agree_percent: 36.3, model_streams: [] },
      ]),
    );
    expect(rows.map((row) => row.rule_stream)).toEqual(['aktiv', 'feed']);
    // Frontend renders, backend computes: the percentage arrives from comms and
    // is not recalculated here.
    expect(rows[0].agree_percent).toBe(36.3);
  });

  test('a report that never loaded is an empty table, not a thrown page', () => {
    expect(agreementRows(null)).toEqual([]);
  });
});

describe('proposesAChange', () => {
  test('a refusal offers nothing to accept', () => {
    expect(
      proposesAChange(
        verdict({ state: 'local_refused', model_stream: null, rationale: null }),
        'aktiv',
      ),
    ).toBe(false);
    expect(
      proposesAChange(verdict({ state: 'skipped_over_window', model_stream: null }), 'aktiv'),
    ).toBe(false);
    expect(proposesAChange(null, 'aktiv')).toBe(false);
  });

  test('agreement offers nothing either — re-stamping it human would erase that a rule decided it', () => {
    expect(proposesAChange(verdict({ model_stream: 'aktiv' }), 'aktiv')).toBe(false);
  });

  test('a real disagreement does, held or not', () => {
    expect(proposesAChange(verdict(), 'aktiv')).toBe(true);
    expect(
      proposesAChange(
        verdict({
          mode: 'held',
          model_stream: 'belege',
          held_reason: 'applying this would raise the mail from c1 to c2',
        }),
        'aktiv',
      ),
    ).toBe(true);
  });

  // `rule_stream` is the deterministic verdict frozen at prompt time and is
  // never rewritten, so comparing with it kept the card offering an Accept
  // button for a change the human had already made.
  test('an accepted proposal stops proposing: the comparison is the row, not the frozen rule', () => {
    const accepted = verdict({ rule_stream: 'aktiv', model_stream: 'werbung' });
    expect(proposesAChange(accepted, 'aktiv')).toBe(true);
    expect(proposesAChange(accepted, 'werbung')).toBe(false);
    // Same for a row an apply pass moved.
    expect(proposesAChange(verdict({ mode: 'applied', model_stream: 'werbung' }), 'werbung')).toBe(
      false,
    );
  });
});
