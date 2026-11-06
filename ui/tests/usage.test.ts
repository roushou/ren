import test from 'node:test';
import assert from 'node:assert/strict';
import {
  axisTicks,
  cacheShare,
  measureValue,
  parseRange,
  rangeStart,
  usagePoints,
} from '../src/lib/usage.ts';
import type { UsageBucket } from '../src/lib/api.ts';

const HOUR = 3_600_000;
const DAY = 24 * HOUR;
const utcDay = (ms: number) => Math.floor(ms / DAY) * DAY;
const now = Date.UTC(2026, 10, 6, 16, 30);
const hour = (start: number, calls: number, input: number | null = null): UsageBucket =>
  ({
    start,
    calls,
    errors: calls > 2 ? 1 : 0,
    tokens: {
      input,
      output: input === null ? null : 1,
      cache_read: input === null ? null : input / 2,
    },
  }) as UsageBucket;

test('ranges parse with a week as the default', () => {
  assert.deepEqual(['24h', '7d', 'all', null, 'year'].map(parseRange), [
    '24h',
    '7d',
    'all',
    '7d',
    '7d',
  ]);
  assert.equal(rangeStart('24h', now, utcDay), Date.UTC(2026, 10, 5, 17));
  assert.equal(rangeStart('7d', now, utcDay), Date.UTC(2026, 9, 31));
  assert.equal(rangeStart('all', now, utcDay), undefined);
});

test('a day by the hour has 24 bars, empty hours included', () => {
  const points = usagePoints([hour(Date.UTC(2026, 10, 6, 14), 3, 100)], '24h', now, utcDay);
  assert.equal(points.length, 24);
  assert.equal(points.at(-1)?.start, Date.UTC(2026, 10, 6, 16));
  const busy = points.find((p) => p.calls);
  assert.equal(busy?.start, Date.UTC(2026, 10, 6, 14));
  assert.equal(points.filter((p) => p.calls === 0).length, 23);
  // An empty bar reports no tokens, not zero tokens.
  assert.equal(points[0].tokens.input, null);
});

test('a week folds hours into seven days', () => {
  const points = usagePoints(
    [
      hour(Date.UTC(2026, 10, 6, 1), 1, 10),
      hour(Date.UTC(2026, 10, 6, 15), 3, 30),
      hour(Date.UTC(2026, 10, 2, 9), 2),
      // Before the range: not drawn.
      hour(Date.UTC(2026, 9, 20, 9), 9, 900),
    ],
    '7d',
    now,
    utcDay,
  );
  assert.equal(points.length, 7);
  const today = points.at(-1)!;
  assert.deepEqual(
    [today.calls, today.errors, today.tokens.input, today.tokens.cache_read],
    [4, 1, 40, 20],
  );
  const nov2 = points.find((p) => p.start === Date.UTC(2026, 10, 2))!;
  assert.deepEqual([nov2.calls, nov2.tokens.input], [2, null]);
  assert.equal(
    points.reduce((sum, p) => sum + p.calls, 0),
    6,
  );
});

test('all retained starts at the first day with calls', () => {
  const points = usagePoints([hour(Date.UTC(2026, 10, 4, 9), 1)], 'all', now, utcDay);
  assert.deepEqual(
    points.map((p) => p.start),
    [Date.UTC(2026, 10, 4), Date.UTC(2026, 10, 5), Date.UTC(2026, 10, 6)],
  );
  assert.equal(usagePoints([], 'all', now, utcDay).length, 1);
});

test('axis ticks are round and reach the maximum', () => {
  assert.deepEqual(axisTicks(0), [0, 1]);
  assert.deepEqual(axisTicks(7), [0, 2, 4, 6, 8]);
  assert.deepEqual(axisTicks(3_188_443), [0, 1_000_000, 2_000_000, 3_000_000, 4_000_000]);
  assert.deepEqual(axisTicks(90), [0, 25, 50, 75, 100]);
});

test('measures and cache share read the right numbers', () => {
  const counts = { calls: 3, errors: 1, tokens: { input: 200, output: 7, cache_read: 150 } };
  assert.deepEqual(
    (['input', 'output', 'calls', 'errors'] as const).map((m) => measureValue(counts, m)),
    [200, 7, 3, 1],
  );
  assert.equal(cacheShare(counts), 0.75);
  assert.equal(
    cacheShare({ ...counts, tokens: { input: null, output: null, cache_read: null } }),
    null,
  );
});
