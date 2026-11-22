import test from 'node:test';
import assert from 'node:assert/strict';
import { SessionInspector } from '../src/lib/inspector.ts';
import type { RecordingReport, TurnMetrics } from '../src/lib/api.ts';
import type { Act, Step } from '../src/lib/steps.ts';

function turn(start: number, end: number, context: number, hit: number | null): TurnMetrics {
  return {
    turn: `turn-${start}` as TurnMetrics['turn'],
    started_at: start as TurnMetrics['started_at'],
    ended_at: end as TurnMetrics['started_at'],
    call_count: 1,
    idle_before_ms: null,
    context_start_tokens: null,
    context_end_tokens: context,
    cache_hit_percent: hit,
    cache_written_tokens: 0,
    output_tokens: 0,
  };
}

function report(turns: TurnMetrics[]): RecordingReport {
  return {
    recording: 'rec' as RecordingReport['recording'],
    totals: {
      calls: 57,
      calls_with_usage: 57,
      peak_context_tokens: 0,
      context_tokens: 1_840_000,
      cache_read_tokens: 0,
      cache_written_tokens: 0,
      output_tokens: 14_200,
      cache_hit_percent: 71,
    },
    turns,
    findings: [],
    omitted_calls: 0,
  };
}

function step(act: Act, took: number | null, ...targets: string[]): Step {
  return { act, took, targets } as unknown as Step;
}

test('context draws each turn against the tallest, split by what came from cache', () => {
  const view = SessionInspector.context(
    report([turn(0, 10, 40_000, 50), turn(20, 30, 80_000, 75), turn(40, 50, 60_000, null)]),
  );
  assert.deepEqual(
    view.bars.map((bar) => [bar.cached, bar.fresh]),
    [
      [0.25, 0.25],
      [0.75, 0.25],
      [0, 0.75],
    ],
  );
  assert.equal(view.last, 60_000);
  assert.equal(view.cacheHit, 71);
});

test('context keeps only the newest turns', () => {
  const many = Array.from({ length: 20 }, (_, i) => turn(i * 10, i * 10 + 5, 1000 * (i + 1), 0));
  const view = SessionInspector.context(report(many));
  assert.equal(view.bars.length, SessionInspector.TURNS);
  assert.equal(view.last, 20_000);
});

test('a recording with no turns has no last context', () => {
  assert.equal(SessionInspector.context(report([])).last, null);
});

test('totals read compactly', () => {
  assert.deepEqual(SessionInspector.totals(report([])), {
    input: '1.8M',
    output: '14k',
    calls: '57',
  });
});

test('time goes to each kind of step, and the rest of the turns to the model', () => {
  const steps = [
    step('run', 6000),
    step('read', 1000),
    step('search', 500),
    step('edit', 500),
    step('run', null),
  ];
  const view = SessionInspector.time(
    steps,
    report([turn(0, 10_000, 1, 0), turn(20_000, 22_000, 1, 0)]),
  );
  assert.equal(view.total, 12_000);
  assert.deepEqual(
    view.parts.map((part) => [part.kind, part.ms]),
    [
      ['run', 6000],
      ['model', 4000],
      ['read', 1500],
      ['edit', 500],
    ],
  );
  assert.equal(view.parts[0].share, 0.5);
});

test('without a report, time is the steps alone', () => {
  const view = SessionInspector.time([step('tool', 2000), step('delegate', 1000)], null);
  assert.deepEqual(
    view.parts.map((part) => [part.label, part.ms]),
    [['Tools', 3000]],
  );
});

test('files count reads and edits per path, the edited first', () => {
  const rows = SessionInspector.files([
    step('read', 1, 'src/a.rs', 'src/b.rs'),
    step('read', 1, 'src/a.rs'),
    step('edit', 1, 'src/b.rs'),
    step('run', 1, 'cargo test'),
    step('search', 1, 'sleep('),
  ]);
  assert.deepEqual(rows, [
    { path: 'src/b.rs', reads: 1, edits: 1 },
    { path: 'src/a.rs', reads: 2, edits: 0 },
  ]);
});
