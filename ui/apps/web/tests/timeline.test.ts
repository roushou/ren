import test from 'node:test';
import assert from 'node:assert/strict';
import { CallTimeline } from '../src/lib/timeline.ts';
import type { UnixMillis } from '../src/lib/api.ts';
import { callFixture, callId, finishedFixture } from './fixtures.ts';

const ms = (value: number) => value as UnixMillis;

test('an empty page draws no lanes and counts nothing', () => {
  const timeline = CallTimeline.of([], 5_000);
  assert.deepEqual(timeline.lanes, []);
  assert.equal(timeline.figures.calls, 0);
  assert.equal(timeline.figures.p50, null);
});

test('calls lie on their recording’s lane, placed from start to end across the window', () => {
  const calls = [
    finishedFixture(callId(1), {
      recording: 'r-a' as never,
      started_at: ms(0),
      ended_at: ms(10_000),
    }),
    finishedFixture(callId(2), {
      recording: 'r-a' as never,
      started_at: ms(50_000),
      ended_at: ms(60_000),
    }),
    finishedFixture(callId(3), {
      recording: 'r-b' as never,
      started_at: ms(20_000),
      ended_at: ms(30_000),
      status: 529,
    }),
    // Still open: it runs to now, and its lane is live.
    callFixture(callId(4), { recording: 'r-b' as never, started_at: ms(90_000) }),
  ];
  const timeline = CallTimeline.of(calls, 100_000);
  assert.equal(timeline.from, 0);
  assert.equal(timeline.to, 100_000);
  assert.deepEqual(
    timeline.lanes.map((lane) => [
      lane.recording,
      lane.live,
      lane.bars.map((bar) => [bar.left, bar.width, bar.mark]),
    ]),
    // As busy as each other: the newer recording first.
    [
      [
        'r-b',
        true,
        [
          [20, 10, 'failed'],
          [90, 10, 'responding'],
        ],
      ],
      [
        'r-a',
        false,
        [
          [0, 10, 'passed'],
          [50, 10, 'passed'],
        ],
      ],
    ],
  );
  assert.deepEqual(timeline.figures, {
    calls: 4,
    perMinute: 2.4,
    failed: 1,
    p50: 10_000,
    p95: 10_000,
    open: 1,
    probes: 0,
  });
});

test('past six recordings the quietest are counted, not drawn; a short call still shows', () => {
  const calls = Array.from({ length: 8 }, (_, n) =>
    finishedFixture(callId(n + 1), {
      recording: `r-${n}` as never,
      started_at: ms(n * 1_000),
      ended_at: ms(n * 1_000 + 1),
    }),
  );
  const timeline = CallTimeline.of(calls, 1_000_000);
  assert.equal(timeline.lanes.length, 6);
  assert.equal(timeline.folded, 2);
  assert.equal(timeline.lanes.at(-1)?.bars[0]?.width, 0.4);
});

test('a call reads as a mark: a quota probe as information, a refusal as a warning', () => {
  assert.equal(
    CallTimeline.mark(finishedFixture(callId(1), { purpose: 'quota_probe' })).mark,
    'info',
  );
  assert.equal(
    CallTimeline.mark(finishedFixture(callId(1), { rejection: 'paused' })).mark,
    'warning',
  );
  assert.equal(CallTimeline.mark(callFixture(callId(1))).mark, 'responding');
  assert.equal(
    CallTimeline.mark(finishedFixture(callId(1), { status: 500 })).label,
    'Failed, HTTP 500',
  );
});

test('a call’s context counts the cache as the provider’s accounting says', () => {
  const usage = (accounting: string) => ({
    accounting,
    input_tokens: 1_000,
    output_tokens: 10,
    cache_read_tokens: 3_000,
    cache_creation_tokens: 1_000,
    reasoning_tokens: null,
  });
  assert.deepEqual(
    CallTimeline.context(finishedFixture(callId(1), { usage: usage('excludes_cache') as never })),
    { tokens: 5_000, cached: 0.6 },
  );
  assert.deepEqual(
    CallTimeline.context(finishedFixture(callId(1), { usage: usage('includes_cache') as never })),
    { tokens: 1_000, cached: 1 },
  );
  assert.equal(CallTimeline.context(finishedFixture(callId(1))), null);
});

test('a call’s prompt splits into cache read, cache written and new input', () => {
  const usage = (accounting: string) =>
    ({
      accounting,
      input_tokens: 5_000,
      output_tokens: 10,
      cache_read_tokens: 3_000,
      cache_creation_tokens: 1_000,
      reasoning_tokens: null,
    }) as never;
  assert.deepEqual(
    CallTimeline.tokens(finishedFixture(callId(1), { usage: usage('includes_cache') })),
    {
      context: 5_000,
      read: 3_000,
      written: 1_000,
      fresh: 1_000,
      output: 10,
    },
  );
  assert.equal(
    CallTimeline.tokens(finishedFixture(callId(1), { usage: usage('excludes_cache') }))?.context,
    9_000,
  );
});

test('attempts lie across their call, a failed try before the one that streamed', () => {
  const call = finishedFixture(callId(1), { started_at: ms(0), ended_at: ms(10_000) });
  const attempt = (ordinal: number, from: number, to: number, status: number, terminal: string) =>
    ({
      id: `a${ordinal}`,
      ordinal,
      started_at: ms(from),
      ended_at: ms(to),
      upstream: 'anthropic',
      status,
      terminal,
    }) as never;
  const bars = CallTimeline.attempts(
    call,
    [attempt(1, 0, 1_000, 529, 'body_complete'), attempt(2, 3_000, 10_000, 200, 'body_complete')],
    20_000,
  );
  assert.deepEqual(
    bars.map((bar) => [bar.left, bar.width, bar.mark, bar.label]),
    [
      [0, 10, 'failed', 'HTTP 529 · Stream complete'],
      [30, 70, 'passed', 'HTTP 200 · Stream complete'],
    ],
  );
});
