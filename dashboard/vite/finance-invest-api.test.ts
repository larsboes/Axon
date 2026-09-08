import { afterEach, describe, expect, test } from 'bun:test';
import { ApiError, describeFailure } from '../src/lib/api';
import { decisions, recordVerdict } from '../src/lib/finance/invest-api';

// The case this exists for: the investment client has its OWN `request`, because
// api.ts does not export one. Two request helpers is two failure vocabularies
// unless the copy goes through the same `describeFailure`, and a reader who sees
// "Request failed (500)" on one page and a named capability on another learns
// nothing from either.
//
// Nothing here reaches the network. `fetch` is replaced per test and restored.

const realFetch = globalThis.fetch;
afterEach(() => {
  globalThis.fetch = realFetch;
});

function answering(status: number, body: string): void {
  globalThis.fetch = (async () =>
    new Response(body, { status })) as unknown as typeof fetch;
}

describe('the investment client', () => {
  test('a bare 5xx names the capability and the fix, exactly as api.ts would', async () => {
    answering(500, '');
    const failure = await decisions('open').catch((cause) => cause);
    expect(failure).toBeInstanceOf(ApiError);
    expect(failure.status).toBe(500);
    expect(failure.message).toBe(describeFailure(500, '', '/finance/api/decisions?status=open'));
    expect(failure.message).toContain('finance is not running');
  });

  test("the capability's own error message wins over the generic one", async () => {
    answering(409, JSON.stringify({ error: 'no reviewed holdings snapshot is configured' }));
    const failure = await decisions('open').catch((cause) => cause);
    expect(failure.message).toContain('no reviewed holdings snapshot is configured');
    expect(failure.message).not.toContain('is not running');
  });

  test('a 409 from the verdict route surfaces the id the numbers moved to', async () => {
    // The whole point of the gate: the client must be able to show WHICH
    // proposal it should be answering, not just that something failed.
    answering(
      409,
      JSON.stringify({
        ok: false,
        error: 'the numbers moved since this was proposed',
        current_proposal_id: 'rebalance:asset_class:equity:0011',
      }),
    );
    const failure = await recordVerdict('stale', {
      expected_proposal_id: 'stale',
      verdict: 'accepted',
      note: '',
    }).catch((cause) => cause);
    expect(failure).toBeInstanceOf(ApiError);
    expect(failure.status).toBe(409);
    expect(failure.message).toContain('the numbers moved since this was proposed');
  });

  test('a 200 carrying an error field is still a failure', async () => {
    answering(200, JSON.stringify({ error: 'a note is required for a rejected proposal' }));
    const failure = await decisions('open').catch((cause) => cause);
    expect(failure).toBeInstanceOf(ApiError);
    expect(failure.message).toBe('a note is required for a rejected proposal');
  });

  test('an empty ledger is an empty array, not a placeholder row', async () => {
    answering(200, '[]');
    expect(await decisions('open')).toEqual([]);
  });
});
