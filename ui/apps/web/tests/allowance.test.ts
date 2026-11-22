import test from 'node:test';
import assert from 'node:assert/strict';
import { Allowances } from '../src/lib/allowance.ts';

const headers = (pairs: [string, string | null][]) =>
  pairs.map(([name, value]) => ({ name, value }));

test("a subscription's windows: the fuller one says it", () => {
  assert.deepEqual(
    Allowances.read(
      headers([
        ['anthropic-ratelimit-unified-status', 'allowed'],
        ['anthropic-ratelimit-unified-5h-utilization', '0.42'],
        ['anthropic-ratelimit-unified-7d-utilization', '0.18'],
      ]),
    ),
    { left: 0.5800000000000001, text: '42% used · 5 h' },
  );
});

test("Codex's used percent, with its window's length", () => {
  assert.deepEqual(
    Allowances.read(
      headers([
        ['x-codex-primary-used-percent', '7'],
        ['x-codex-primary-window-minutes', '300'],
        ['x-codex-secondary-used-percent', '18'],
        ['x-codex-secondary-window-minutes', '10080'],
      ]),
    )?.text,
    '18% used · 7 d',
  );
});

test("an API key's remaining requests and tokens: the scarcer one says it", () => {
  assert.deepEqual(
    Allowances.read(
      headers([
        ['x-ratelimit-limit-requests', '500'],
        ['x-ratelimit-remaining-requests', '38'],
        ['x-ratelimit-limit-tokens', '4000000'],
        ['x-ratelimit-remaining-tokens', '2100000'],
      ]),
    ),
    { left: 0.076, text: '38 / 500 requests' },
  );
  assert.equal(
    Allowances.read(
      headers([
        ['anthropic-ratelimit-input-tokens-limit', '400000'],
        ['anthropic-ratelimit-input-tokens-remaining', '100000'],
      ]),
    )?.text,
    '100k / 400k input tokens',
  );
});

test('withheld or missing values state nothing', () => {
  assert.equal(Allowances.read(headers([['x-ratelimit-remaining-requests', null]])), null);
  assert.equal(Allowances.read(headers([['retry-after', '30']])), null);
  assert.equal(Allowances.read([]), null);
});
