import test from 'node:test';
import assert from 'node:assert/strict';
import { ago, DayChange, Failures, Providers, type FailureInput } from '../src/lib/overview.ts';
import type { ModelUsage, UsageView } from '../src/lib/api.ts';
import { callId, finishedFixture } from './fixtures.ts';

const HOUR = 3_600_000;
const NOW = 100 * HOUR;
const at = (hoursAgo: number) => (NOW - hoursAgo * HOUR) as never;

test('relative times', () => {
  const now = 10_000_000;
  assert.deepEqual(
    [now, now - 30_000, now - 12 * 60_000, now - 3 * 3_600_000].map((t) => ago(t, now)),
    ['just now', '30 s ago', '12 min ago', '3 h ago'],
  );
});

const failures = (overrides: Partial<FailureInput>): FailureInput => ({
  now: NOW,
  failed: [],
  stopped: [],
  bookkeeping: [],
  ...overrides,
});

test('failures fold by provider and ending: failed work, then cut-off work, then bookkeeping', () => {
  const probe = (n: number, hoursAgo: number) =>
    finishedFixture(callId(10 + n), { status: 429, started_at: at(hoursAgo) });
  const groups = Failures.of(
    failures({
      bookkeeping: [probe(1, 1), probe(2, 3), probe(3, 9)],
      stopped: [finishedFixture(callId(20), { terminal: 'interrupted', started_at: at(1) })],
      failed: [
        finishedFixture(callId(30), { status: 500, started_at: at(2), model: 'm-a' }),
        finishedFixture(callId(31), { status: 500, started_at: at(5), model: 'm-b' }),
      ],
    }),
  );
  assert.deepEqual(
    groups.map((group) => [group.severity, group.name, group.count, group.models]),
    [
      ['error', 'Anthropic 500', '2', 'm-a · m-b'],
      ['warning', 'Anthropic interrupted', '1', ''],
      ['neutral', 'Anthropic 429', '3', 'bookkeeping'],
    ],
  );
  const probes = groups[2];
  // Repeats fold into one line, with when each happened along the day.
  assert.deepEqual(
    probes?.marks.map((mark) => Math.round(mark * 24)),
    [23, 21, 15],
  );
  assert.equal(probes?.at, at(1));
  assert.equal(
    probes?.href,
    '/monitor/traffic?status=errors&kind=other&provider=anthropic&since=24h',
  );
});

test('a full listing says there may be more', () => {
  assert.equal(Failures.count(Array.from({ length: 100 })), '100+');
  assert.equal(Failures.count([1, 2]), '2');
});

const model = (provider: string, name: string, calls: number, errors: number): ModelUsage => ({
  provider: provider as never,
  model: name,
  calls,
  errors,
  tokens: { input: null, output: null, cache_read: null },
});

test('providers count calls and failures, and stand by the last hour', () => {
  const failing = Array.from({ length: 12 }, (_, n) =>
    finishedFixture(callId(40 + n), { provider: 'codex', status: 500, started_at: at(0.5) }),
  );
  const earlier = finishedFixture(callId(60), {
    provider: 'anthropic',
    status: 429,
    started_at: at(5.5),
  });
  const recent = finishedFixture(callId(61), { provider: 'anthropic', started_at: at(0.1) });
  const rows = Providers.rows({
    now: NOW,
    models: [
      model('anthropic', 'small', 100, 1),
      model('anthropic', 'large', 300, 0),
      model('codex', 'gpt', 50, 12),
    ],
    failed: [...failing, earlier],
    recent: [recent],
  });
  assert.deepEqual(
    rows.map((row) => [row.name, row.state, row.calls, row.failed, row.models]),
    [
      ['Codex', 'failing', 50, 12, ['gpt']],
      ['Anthropic', 'healthy', 400, 1, ['large', 'small']],
    ],
  );
  const [codex, anthropic] = rows;
  assert.equal(codex?.hours[23], 12);
  assert.equal(anthropic?.hours[18], 1);
  assert.deepEqual(anthropic?.last, { ending: '429', at: at(5.5) });
  // The newest finished call is the one whose headers say what is left.
  assert.equal(anthropic?.newest, callId(61));
  assert.equal(codex?.newest, failing[0]?.id);
  assert.equal(anthropic?.rate, 1 / 400);
  assert.equal(Providers.state([0, 0, 1]), 'failures');
});

const usage = (calls: number, errors: number, input: number, cached: number): UsageView => ({
  totals: { calls, errors, tokens: { input, output: 10, cache_read: cached } },
  hours: [],
  models: [],
  users: [],
});

test('the day against the day before', () => {
  const rows = DayChange.rows(
    usage(1180, 37, 2_000_000, 1_900_000),
    usage(1000, 7, 1_000_000, 960_000),
  );
  assert.deepEqual(
    rows.map((row) => [row.label, row.value, row.delta, row.worse]),
    [
      ['Calls', '1,180', '+18%', false],
      ['Failure rate', '3.1%', '+2.4 pt', true],
      ['Tokens in', '2.0M', '+100%', false],
      ['Read from cache', '95%', '−1 pt', true],
    ],
  );
  assert.deepEqual(
    DayChange.rows(usage(5, 0, 0, 0), null).map((row) => row.delta),
    [null, null, null, null],
  );
});
