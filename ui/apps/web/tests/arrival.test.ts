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

test('a read is handed over pending only when streaming and late', async () => {
  const read = Promise.resolve(snapshot('a', 'rows'));
  // Not streaming (a first load): read in full.
  assert.deepEqual(await Arrival.over('a', read, false), {
    state: 'ready',
    snapshot: snapshot('a', 'rows'),
  });
  // Streaming, and in within the grace period: ready, so the page renders once.
  assert.deepEqual(await Arrival.over('a', read, true, 50), {
    state: 'ready',
    snapshot: snapshot('a', 'rows'),
  });
  // Streaming, and late: pending, for the page to draw its outline.
  const pending = await Arrival.over('a', new Promise<Snapshot<string>>(() => {}), true, 0);
  assert.ok(pending.state === 'pending' && pending.key === 'a');
});

test('a read is in within its wait, or late', async () => {
  assert.deepEqual(await Arrival.within(Promise.resolve(1), 0), { state: 'in', value: 1 });
  assert.deepEqual(await Arrival.within(new Promise(() => {}), 0), { state: 'late' });
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

test('the browser keeps each key newest read with a value, and only so many', () => {
  const snapshot = (key: string, value: number | null) => ({
    key,
    value,
    error: null,
    updated: value === null ? null : 1,
  });
  Arrival.keep(snapshot('kept:a', 1));
  Arrival.keep(snapshot('kept:a', null));
  assert.equal(Arrival.newest<number>('kept:a')?.value, 1);
  assert.equal(
    Arrival.value<number>({ state: 'pending', key: 'kept:a', snapshot: new Promise(() => {}) }),
    1,
  );
  assert.equal(Arrival.value<number>({ state: 'ready', snapshot: snapshot('kept:a', 2) }), 2);
  assert.equal(Arrival.newest('kept:never'), null);
  for (let index = 0; index <= Arrival.KEPT; index++)
    Arrival.keep(snapshot(`kept:${index}`, index));
  // The oldest kept gives way.
  assert.equal(Arrival.newest('kept:a'), null);
  assert.equal(Arrival.newest<number>(`kept:${Arrival.KEPT}`)?.value, Arrival.KEPT);
});
