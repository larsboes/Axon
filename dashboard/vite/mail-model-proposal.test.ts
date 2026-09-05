import { describe, expect, test } from 'bun:test';
import {
  agreementRows,
  mailHomePriority,
  proposesAChange,
  type TriageClassifyReport,
  type TriageModelVerdict,
} from '../src/lib/mail/api';
import type { TriageItem } from '../src/lib/api';

const report = (rows: TriageClassifyReport['by_rule_stream']): TriageClassifyReport => ({
  verdicts: rows.reduce((sum, row) => sum + row.n, 0),
  candidates: 0,
  by_rule_stream: rows,
  by_state: [],
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

const item = (over: Partial<TriageItem> = {}): TriageItem =>
  ({
    id: 'thread-1',
    from_addr: 'sender@example.com',
    subject: 'A subject',
    snippet: 'A preview.',
    stream: 'aktiv',
    rationale: 'No rule matched; kept active as the conservative default.',
    classification_method: 'deterministic',
    classification_version: 'mail-rules-v1',
    data_class: 'c1',
    data_class_rationale: 'Mail metadata is Mine by default.',
    data_classification_method: 'deterministic',
    data_classification_version: 'data-class-rules-v2',
    status: 'proposed',
    gmail_action: null,
    gmail_action_at: null,
    purge_after: null,
    gmail_location: 'inbox',
    gmail_observed_at: null,
    gmail_sync_status: 'synced',
    gmail_sync_action: null,
    gmail_sync_error: null,
    waiting: false,
    waiting_since: null,
    internal_date: null,
    relevance: [],
    model: null,
    ...over,
  }) as TriageItem;

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

describe('mailHomePriority', () => {
  // The gate this whole field is behind. While the corpus carries no measured
  // urgency band error, Home's numbers have to be byte for byte what they are
  // today, whatever the model reported.
  test('reproduces today’s 550 and 580 exactly while urgency is unvalidated', () => {
    const old = item({ internal_date: '2020-01-01T00:00:00Z', model: verdict({ urgency_bp: 10_000 }) });
    const recent = item({
      internal_date: new Date(Date.now() - 3_600_000).toISOString(),
      model: verdict({ urgency_bp: 10_000 }),
    });
    expect(mailHomePriority(old, false)).toBe(550);
    expect(mailHomePriority(recent, false)).toBe(580);
    // No verdict at all is the same answer, not a crash.
    expect(mailHomePriority(item({ internal_date: null }), false)).toBe(550);
  });

  test('once validated, urgency ranks inside the band and never reaches the task band', () => {
    const recent = new Date(Date.now() - 3_600_000).toISOString();
    expect(
      mailHomePriority(item({ internal_date: recent, model: verdict({ urgency_bp: 10_000 }) }), true),
    ).toBe(619);
    expect(
      mailHomePriority(item({ internal_date: recent, model: verdict({ urgency_bp: 0 }) }), true),
    ).toBe(580);
    // 620 is the task band. Nothing in the mail band may reach it.
    for (const urgency_bp of [0, 1, 2_500, 5_000, 9_999, 10_000]) {
      const priority = mailHomePriority(
        item({ internal_date: recent, model: verdict({ urgency_bp }) }),
        true,
      );
      expect(priority).toBeLessThan(620);
      expect(priority).toBeGreaterThanOrEqual(580);
    }
  });
});

describe('proposesAChange', () => {
  test('a refusal offers nothing to accept', () => {
    expect(
      proposesAChange(verdict({ state: 'local_refused', model_stream: null, rationale: null })),
    ).toBe(false);
    expect(proposesAChange(verdict({ state: 'skipped_over_window', model_stream: null }))).toBe(
      false,
    );
    expect(proposesAChange(null)).toBe(false);
  });

  test('agreement offers nothing either — re-stamping it human would erase that a rule decided it', () => {
    expect(proposesAChange(verdict({ model_stream: 'aktiv' }))).toBe(false);
  });

  test('a real disagreement does, held or not', () => {
    expect(proposesAChange(verdict())).toBe(true);
    expect(
      proposesAChange(
        verdict({
          mode: 'held',
          model_stream: 'belege',
          held_reason: 'applying this would raise the mail from c1 to c2',
        }),
      ),
    ).toBe(true);
  });
});
