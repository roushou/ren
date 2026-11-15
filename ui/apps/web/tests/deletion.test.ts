import test from 'node:test';
import assert from 'node:assert/strict';
import { Deletions, type Deletion, type SessionId } from '../src/lib/runs.ts';

const a = '0190f3a0-0000-7000-8000-00000000a001' as SessionId;
const b = '0190f3a0-0000-7000-8000-00000000b001' as SessionId;

function step(state: Deletion, ...events: Parameters<typeof Deletions.step>[1][]): Deletion {
  return events.reduce((at, event) => {
    const next = Deletions.step(at, event);
    assert.ok(!Deletions.illegal(next), `${event.kind} refused while ${at.phase}`);
    return next;
  }, state);
}

test('a delete is asked, confirmed, sent, and done when the session is gone', () => {
  const asked = step(Deletions.idle, { kind: 'ask', session: a, from: 'list' });
  assert.deepEqual(asked, { phase: 'confirming', session: a, from: 'list' });
  const sent = step(asked, { kind: 'confirm' });
  assert.deepEqual(sent, { phase: 'deleting', session: a, from: 'list' });
  assert.deepEqual(step(sent, { kind: 'gone' }), { phase: 'idle' });
});

test('cancelling a question sends nothing', () => {
  const state = step(
    Deletions.idle,
    { kind: 'ask', session: a, from: 'header' },
    { kind: 'cancel' },
  );
  assert.deepEqual(state, { phase: 'idle' });
});

test('a refused delete keeps its reason and can be sent again or dropped', () => {
  const refused = step(
    Deletions.idle,
    { kind: 'ask', session: a, from: 'list' },
    { kind: 'confirm' },
    { kind: 'refused', reason: 'A call of it is still open.' },
  );
  assert.deepEqual(refused, {
    phase: 'failed',
    session: a,
    from: 'list',
    reason: 'A call of it is still open.',
  });
  assert.equal(step(refused, { kind: 'confirm' }).phase, 'deleting');
  assert.deepEqual(step(refused, { kind: 'cancel' }), { phase: 'idle' });
});

test('asking about another session moves the question there', () => {
  const state = step(
    Deletions.idle,
    { kind: 'ask', session: a, from: 'list' },
    { kind: 'ask', session: b, from: 'header' },
  );
  assert.deepEqual(state, { phase: 'confirming', session: b, from: 'header' });
});

test('a delete under way can be neither cancelled, nor asked over, nor sent twice', () => {
  const sent = step(Deletions.idle, { kind: 'ask', session: a, from: 'list' }, { kind: 'confirm' });
  for (const event of [
    { kind: 'cancel' },
    { kind: 'confirm' },
    { kind: 'ask', session: b, from: 'list' },
  ] as const) {
    assert.deepEqual(Deletions.step(sent, event), {
      illegal: true,
      state: 'deleting',
      event: event.kind,
    });
  }
});

test('nothing asked cannot be confirmed, cancelled, refused, or finished', () => {
  for (const event of [
    { kind: 'confirm' },
    { kind: 'cancel' },
    { kind: 'refused', reason: 'no' },
    { kind: 'gone' },
  ] as const) {
    assert.ok(Deletions.illegal(Deletions.step(Deletions.idle, event)));
  }
  const asked = step(Deletions.idle, { kind: 'ask', session: a, from: 'list' });
  assert.ok(Deletions.illegal(Deletions.step(asked, { kind: 'refused', reason: 'no' })));
});

test('each place shows only the delete asked from it', () => {
  const asked = step(Deletions.idle, { kind: 'ask', session: a, from: 'list' });
  assert.equal(Deletions.of(asked, a, 'list'), asked);
  assert.equal(Deletions.of(asked, a, 'header'), null);
  assert.equal(Deletions.of(asked, b, 'list'), null);
  assert.equal(Deletions.of(Deletions.idle, a, 'list'), null);
});
