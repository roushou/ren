import test from 'node:test';
import assert from 'node:assert/strict';
import { BottomEdge, type EdgeReading } from '../src/lib/edge.ts';

const well: EdgeReading = {
  failure: null,
  readAt: 1_000,
  recorder: null,
  dismissed: new Set(),
  readOnly: false,
};

test('while everything is well, the edge says nothing', () => {
  assert.deepEqual(BottomEdge.strips(well), []);
});

test('problems are said worst first: Ren, then the recorder, then read-only', () => {
  const strips = BottomEdge.strips({
    ...well,
    failure: 'unreachable',
    recorder: '12 calls not recorded',
    readOnly: true,
  });
  assert.deepEqual(
    strips.map((strip) => strip.kind),
    ['unreachable', 'recorder', 'readOnly'],
  );
  assert.deepEqual(strips[0], { kind: 'unreachable', readAt: 1_000 });
  assert.deepEqual(
    strips.map((strip) => BottomEdge.tone(strip)),
    ['bad', 'warning', 'plain'],
  );
  assert.deepEqual(BottomEdge.strips({ ...well, failure: 'incompatible' }), [
    { kind: 'incompatible' },
  ]);
});

test('a dismissed recorder notice stays away until it says something new', () => {
  const lost = { ...well, recorder: '12 calls not recorded' };
  const dismissed = new Set(['12 calls not recorded']);
  assert.deepEqual(BottomEdge.strips({ ...lost, dismissed }), []);
  assert.deepEqual(BottomEdge.strips({ ...lost, recorder: '13 calls not recorded', dismissed }), [
    { kind: 'recorder', said: '13 calls not recorded' },
  ]);
  // Read-only is a setup, not a notice: it cannot be dismissed.
  assert.equal(BottomEdge.strips({ ...well, readOnly: true, dismissed }).length, 1);
});
