import test from 'node:test';
import assert from 'node:assert/strict';
import { Arrival, type Arriving } from '../src/lib/arrival.ts';
import type { Snapshot } from '../src/lib/snapshots.ts';

const snapshot = (key: string, value: string | null): Snapshot<string> => ({
  key,
  value,
  error: value === null ? { kind: 'unreachable', message: 'down' } : null,
  updated: value === null ? null : 1,
});

test('a read is handed over pending only when streaming', async () => {
  const read = Promise.resolve(snapshot('a', 'rows'));
  const ready = await Arrival.over('a', read, false);
  assert.deepEqual(ready, { state: 'ready', snapshot: snapshot('a', 'rows') });
  const pending = await Arrival.over('a', read, true);
  assert.equal(pending.state, 'pending');
  assert.ok(pending.state === 'pending' && pending.key === 'a');
});

test('a pending read shows its outline, or the same query’s previous rows', () => {
  const pending: Arriving<string> = {
    state: 'pending',
    key: 'traffic?model=x',
    snapshot: new Promise(() => {}),
  };
  // A first visit, or another query: the outline.
  assert.deepEqual(Arrival.shown(pending, null, null), { state: 'pending' });
  assert.deepEqual(Arrival.shown(pending, null, snapshot('traffic?model=y', 'other rows')), {
    state: 'pending',
  });
  // A refresh of the same query keeps its rows.
  assert.deepEqual(Arrival.shown(pending, null, snapshot('traffic?model=x', 'rows')), {
    state: 'ready',
    snapshot: snapshot('traffic?model=x', 'rows'),
  });
  // Once in, what it read; a failed read keeps the same query's rows, with the error.
  assert.deepEqual(Arrival.shown(pending, snapshot('traffic?model=x', 'new'), null), {
    state: 'ready',
    snapshot: snapshot('traffic?model=x', 'new'),
  });
  const failed = Arrival.shown(
    pending,
    snapshot('traffic?model=x', null),
    snapshot('traffic?model=x', 'rows'),
  );
  assert.ok(failed.state === 'ready');
  assert.equal(failed.snapshot.value, 'rows');
  assert.equal(failed.snapshot.error?.kind, 'unreachable');
});
