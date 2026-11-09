import test from 'node:test';
import assert from 'node:assert/strict';
import { LiveFollow, type LiveEvent, type LiveState, type LiveStep } from '../src/lib/live.ts';
import type { ObservationSeq } from '../src/lib/api.ts';
import { callId, uuid } from './fixtures.ts';

const seq = (n: number) => n as ObservationSeq;
/** Run events from a state, returning every step. */
function run(state: LiveState, ...events: LiveEvent[]): LiveStep[] {
  const steps: LiveStep[] = [];
  for (const event of events) {
    const step = LiveFollow.step(state, event);
    steps.push(step);
    state = step.state;
  }
  return steps;
}
const actions = (step: LiveStep) => (step.kind === 'moved' ? step.actions.map((a) => a.kind) : []);
const following = (last: number | null): LiveState => ({
  kind: 'following',
  last: last === null ? null : seq(last),
});

test('enabling connects without a position, then refreshes once the feed opens', () => {
  const [enable, opened] = run(LiveFollow.initial, { kind: 'enable' }, { kind: 'opened' });
  assert.equal(enable.state.kind, 'connecting');
  assert.deepEqual(enable.kind === 'moved' && enable.actions, [
    { kind: 'connect', after: null, delayMs: 0 },
  ]);
  assert.deepEqual(opened.state, following(null));
  assert.deepEqual(actions(opened), ['refresh']);
});

test('observations and lag refresh; replayed positions are ignored', () => {
  const [first, replay, older, lagged] = run(
    following(null),
    { kind: 'observation', seq: seq(5) },
    { kind: 'observation', seq: seq(5) },
    { kind: 'observation', seq: seq(3) },
    { kind: 'lagged' },
  );
  assert.deepEqual(first.state, following(5));
  assert.deepEqual(actions(first), ['refresh']);
  assert.equal(replay.kind, 'ignored');
  assert.equal(older.kind, 'ignored');
  assert.deepEqual(lagged.state, following(5));
  assert.deepEqual(actions(lagged), ['refresh']);
  // Malformed observation data is still a change.
  const [malformed] = run(following(5), { kind: 'observation', seq: null });
  assert.deepEqual(malformed.state, following(5));
  assert.deepEqual(actions(malformed), ['refresh']);
});

test('a pruned position closes the feed, reloads everything, and reconnects fresh', () => {
  const [gone, reloaded] = run(following(9), { kind: 'gone' }, { kind: 'reloaded' });
  assert.deepEqual(gone.state, { kind: 'reloading' });
  assert.deepEqual(actions(gone), ['disconnect', 'reload']);
  assert.equal(reloaded.state.kind, 'connecting');
  assert.deepEqual(reloaded.kind === 'moved' && reloaded.actions, [
    { kind: 'connect', after: null, delayMs: 0 },
  ]);
  // Late events from the closed feed do not interrupt the reload.
  assert.equal(
    LiveFollow.step({ kind: 'reloading' }, { kind: 'observation', seq: seq(10) }).kind,
    'ignored',
  );
});

test('drops wait for the browser to reconnect, or reopen with backoff from the last position', () => {
  const [browser] = run(following(4), { kind: 'dropped', retry: 'browser' });
  assert.deepEqual(browser.state, {
    kind: 'connecting',
    last: seq(4),
    failures: 1,
    problem: 'disconnected',
  });
  assert.deepEqual(actions(browser), []);
  const steps = run(
    following(4),
    { kind: 'dropped', retry: 'closed' },
    { kind: 'dropped', retry: 'closed' },
    { kind: 'dropped', retry: 'closed' },
  );
  const delays = steps.map((step) =>
    step.kind === 'moved' ? step.actions.find((a) => a.kind === 'connect') : undefined,
  );
  assert.deepEqual(delays, [
    { kind: 'connect', after: seq(4), delayMs: 1000 },
    { kind: 'connect', after: seq(4), delayMs: 2000 },
    { kind: 'connect', after: seq(4), delayMs: 4000 },
  ]);
  assert.equal(LiveFollow.delay(20), LiveFollow.maxDelayMs);
  const [reopened] = run(steps[2].state, { kind: 'opened' });
  assert.deepEqual(reopened.state, following(4));
  assert.deepEqual(actions(reopened), ['refresh']);
});

test('store errors are reported until the feed reopens', () => {
  const [error, dropped, opened] = run(
    following(2),
    { kind: 'store_error' },
    { kind: 'dropped', retry: 'browser' },
    { kind: 'opened' },
  );
  assert.equal(error.state.kind === 'connecting' && error.state.problem, 'store_unavailable');
  assert.equal(dropped.state.kind === 'connecting' && dropped.state.problem, 'store_unavailable');
  assert.match(LiveFollow.describe(dropped.state), /Storage unavailable/);
  assert.equal(opened.state.kind, 'following');
});

test('disabling closes from any state; events after closing are ignored', () => {
  for (const state of [
    following(1),
    { kind: 'reloading' } as const,
    { kind: 'connecting', last: null, failures: 2, problem: 'disconnected' } as const,
  ]) {
    const [disable] = run(state, { kind: 'disable' });
    assert.deepEqual(disable.state, LiveFollow.initial);
    assert.deepEqual(actions(disable), ['disconnect']);
  }
  for (const event of [
    { kind: 'opened' },
    { kind: 'observation', seq: seq(1) },
    { kind: 'lagged' },
    { kind: 'gone' },
    { kind: 'dropped', retry: 'closed' },
    { kind: 'reloaded' },
    { kind: 'disable' },
  ] as const)
    assert.equal(LiveFollow.step(LiveFollow.initial, event).kind, 'ignored');
  assert.equal(LiveFollow.step(following(1), { kind: 'enable' }).kind, 'ignored');
  const [unsupported] = run(LiveFollow.initial, { kind: 'enable' }, { kind: 'unsupported' }).slice(
    1,
  );
  assert.deepEqual(unsupported.state, { kind: 'closed', reason: 'unsupported' });
  assert.equal(LiveFollow.step(unsupported.state, { kind: 'enable' }).kind, 'ignored');
});

test('SSE messages parse into events; observation positions come from validated data', () => {
  const observation = JSON.stringify({
    seq: 12,
    at: 1000,
    kind: 'call_finished',
    channel: uuid(1),
    call: callId(2),
    attempt: null,
  });
  assert.deepEqual(LiveFollow.message('observation', observation, '12'), {
    kind: 'observation',
    seq: 12,
  });
  assert.deepEqual(LiveFollow.message('observation', observation, '13'), {
    kind: 'observation',
    seq: null,
  });
  assert.deepEqual(LiveFollow.message('observation', '{"seq":"x"}', '12'), {
    kind: 'observation',
    seq: null,
  });
  // Unknown observation kinds are kept, not rejected.
  assert.deepEqual(
    LiveFollow.message('observation', observation.replace('call_finished', 'future'), '12'),
    { kind: 'observation', seq: 12 },
  );
  assert.deepEqual(LiveFollow.message('gone', '{"oldest_retained":50}', ''), { kind: 'gone' });
  assert.deepEqual(LiveFollow.message('lagged', '{"resume_from":4}', ''), { kind: 'lagged' });
  assert.deepEqual(LiveFollow.message('error', 'store unavailable', ''), { kind: 'store_error' });
  assert.equal(LiveFollow.message('future_event', '{}', ''), null);
});

test('the status text shows connection state beside the last update', () => {
  assert.equal(LiveFollow.summary(LiveFollow.initial, 'Updated 10:00:00'), 'Updated 10:00:00');
  assert.equal(LiveFollow.summary(following(1), 'Updated 10:00:00'), 'Live · Updated 10:00:00');
  assert.equal(
    LiveFollow.summary({ kind: 'connecting', last: null, failures: 0, problem: 'none' }, null),
    'Connecting…',
  );
  assert.match(LiveFollow.summary({ kind: 'reloading' }, null), /reloading/);
});

test('progress events parse at the boundary and keep a bounded, newest-last set', async () => {
  const { Streaming } = await import('../src/lib/streaming.ts');
  const call = '00000000-0000-4000-8000-000000000001';
  const update = Streaming.parse(
    JSON.stringify({ call, reply: 'Hi', cut: false, tools: ['Bash'] }),
  );
  assert.equal(update?.reply, 'Hi');
  // A daemon that predates `thinking` leaves it out: not thinking.
  assert.equal(update?.thinking, false);
  const thinking = Streaming.parse(
    JSON.stringify({ call, reply: '', cut: false, tools: [], thinking: true }),
  );
  assert.equal(thinking?.thinking, true);
  assert.equal(
    Streaming.parse(JSON.stringify({ call, reply: '', cut: false, tools: [], thinking: 'yes' })),
    null,
  );
  assert.equal(Streaming.parse('{"call":"nope","reply":"x","cut":false,"tools":[]}'), null);
  assert.equal(Streaming.parse('not json'), null);
  const calls = new Map();
  for (let n = 0; n < Streaming.MAX_CALLS + 5; n++) {
    Streaming.keep(calls, {
      call: callId(n),
      reply: String(n),
      cut: false,
      tools: [],
      thinking: false,
    });
  }
  assert.equal(calls.size, Streaming.MAX_CALLS);
  // The oldest went first; a call updated again moves to the end.
  assert.equal(calls.has(callId(0)), false);
  assert.equal([...calls.keys()].at(-1), callId(Streaming.MAX_CALLS + 4));
});
