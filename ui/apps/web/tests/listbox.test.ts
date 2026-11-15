import test from 'node:test';
import assert from 'node:assert/strict';
import {
  Listbox,
  type ListboxContext,
  type ListboxEvent,
  type ListboxState,
  type ListKey,
} from '../src/lib/listbox.ts';

const labels = ['All providers', 'Anthropic', 'Codex', 'OpenAI'];
const context = (selected = 0, list: readonly string[] = labels): ListboxContext => ({
  labels: list,
  selected,
});
const open = (active: number): ListboxState => ({ kind: 'open', active, search: null });
const key = (key: ListKey): ListboxEvent => ({ kind: 'key', key });
const type = (char: string, at: number): ListboxEvent => ({ kind: 'type', char, at });

/** Run events from a state; the final state and every committed index. */
function run(state: ListboxState, events: ListboxEvent[], ctx = context()) {
  const commits: number[] = [];
  for (const event of events) {
    const step = Listbox.step(state, event, ctx);
    state = step.state;
    if (step.kind === 'moved')
      for (const action of step.actions) if (action.kind === 'commit') commits.push(action.index);
  }
  return { state, commits };
}

const press = (key: string, extra: Partial<KeyboardEvent> = {}) =>
  Listbox.key({ key, altKey: false, ctrlKey: false, metaKey: false, timeStamp: 7, ...extra });

test('keys parse into listbox events, and shortcuts with modifiers are left alone', () => {
  assert.deepEqual(press('ArrowDown'), key('next'));
  assert.deepEqual(press('ArrowDown', { altKey: true }), key('reveal'));
  assert.deepEqual(press('ArrowUp', { altKey: true }), key('accept_close'));
  assert.deepEqual(press('PageUp'), key('page_previous'));
  assert.deepEqual(press('Enter'), key('accept'));
  assert.deepEqual(press('Escape'), { kind: 'dismiss' });
  assert.deepEqual(press('Tab'), { kind: 'leave' });
  assert.deepEqual(press('o'), type('o', 7));
  assert.deepEqual(press(' '), type(' ', 7));
  assert.equal(press('o', { ctrlKey: true }), null);
  assert.equal(press('ArrowDown', { metaKey: true }), null);
  assert.equal(press('Shift'), null);
  assert.equal(press('F2'), null);
});

test('opening highlights the chosen option, or the first when none is chosen', () => {
  for (const event of [key('next'), key('previous'), key('accept'), key('reveal'), type(' ', 0)])
    assert.deepEqual(Listbox.step(Listbox.initial, event, context(2)).state, open(2));
  assert.deepEqual(Listbox.step(Listbox.initial, { kind: 'toggle' }, context(-1)).state, open(0));
  assert.deepEqual(Listbox.step(Listbox.initial, key('first'), context(2)).state, open(0));
  assert.deepEqual(Listbox.step(Listbox.initial, key('last'), context(0)).state, open(3));
  // A stale selection past the end is no selection.
  assert.deepEqual(Listbox.step(Listbox.initial, { kind: 'toggle' }, context(9)).state, open(0));
});

test('arrows, Home, End, and paging move the highlight and stop at the ends', () => {
  assert.deepEqual(run(open(0), [key('previous')]).state, open(0));
  assert.deepEqual(run(open(0), [key('next'), key('next')]).state, open(2));
  assert.deepEqual(run(open(3), [key('next')]).state, open(3));
  assert.deepEqual(run(open(1), [key('last'), key('first')]).state, open(0));
  const long = Array.from({ length: 25 }, (_, index) => `Option ${index}`);
  assert.deepEqual(run(open(3), [key('page_next')], context(0, long)).state, open(13));
  assert.deepEqual(run(open(20), [key('page_next')], context(0, long)).state, open(24));
  assert.deepEqual(run(open(4), [key('page_previous')], context(0, long)).state, open(0));
});

test('accepting commits the highlighted option and closes; the current one commits nothing', () => {
  assert.deepEqual(run(open(1), [key('next'), key('accept')]), {
    state: Listbox.initial,
    commits: [2],
  });
  assert.deepEqual(run(open(3), [key('accept_close')]).commits, [3]);
  assert.deepEqual(run(open(2), [type(' ', 0)]).commits, [2]);
  assert.deepEqual(run(open(0), [key('accept')]), { state: Listbox.initial, commits: [] });
});

test('pointing highlights, and clicking an option chooses it', () => {
  assert.deepEqual(run(open(0), [{ kind: 'point', index: 2 }]).state, open(2));
  assert.deepEqual(run(open(0), [{ kind: 'choose', index: 3 }]), {
    state: Listbox.initial,
    commits: [3],
  });
  // Out-of-range indexes and events on a closed list do nothing.
  assert.equal(Listbox.step(open(0), { kind: 'point', index: 9 }, context()).kind, 'ignored');
  assert.equal(
    Listbox.step(Listbox.initial, { kind: 'choose', index: 1 }, context()).kind,
    'ignored',
  );
});

test('Escape and clicks outside close without choosing; Tab chooses, as a native select does', () => {
  assert.deepEqual(run(open(2), [{ kind: 'dismiss' }]), { state: Listbox.initial, commits: [] });
  assert.deepEqual(run(open(2), [{ kind: 'leave' }]), { state: Listbox.initial, commits: [2] });
  // On a closed control they are not the listbox's, so the page keeps them.
  for (const event of [{ kind: 'dismiss' }, { kind: 'leave' }] as const)
    assert.equal(Listbox.step(Listbox.initial, event, context()).kind, 'ignored');
  assert.equal(Listbox.step(Listbox.initial, key('page_next'), context()).kind, 'ignored');
});

test('typing jumps to the next label that starts with the text', () => {
  // `a` from "All providers" goes past it to "Anthropic", and again wraps back.
  assert.equal(run(open(0), [type('a', 0)]).state.kind, 'open');
  assert.deepEqual(active(run(open(0), [type('a', 0)]).state), 1);
  assert.deepEqual(active(run(open(0), [type('a', 0), type('a', 100)]).state), 0);
  // A longer search may stay on the current option.
  assert.deepEqual(active(run(open(0), [type('a', 0), type('l', 100)]).state), 0);
  assert.deepEqual(active(run(open(0), [type('o', 0), type('P', 100)]).state), 3);
  // Past the window, a character starts a new search.
  assert.deepEqual(active(run(open(0), [type('a', 0), type('c', 900)]).state), 2);
  // No match keeps the highlight.
  assert.deepEqual(active(run(open(2), [type('z', 0)]).state), 2);
  // Space inside a search is part of it.
  const spaced = ['All', 'All providers', 'Codex'];
  const step = run(open(2), [type('a', 0), type('l', 50), type('l', 90), type(' ', 120)], {
    labels: spaced,
    selected: 2,
  });
  assert.deepEqual(step, {
    state: { kind: 'open', active: 1, search: { text: 'all ', at: 120 } },
    commits: [],
  });
});

test('typing on a closed control opens it at the match, starting after the chosen option', () => {
  assert.deepEqual(active(run(Listbox.initial, [type('c', 0)], context(0)).state), 2);
  assert.deepEqual(active(run(Listbox.initial, [type('a', 0)], context(1)).state), 0);
  // With nothing chosen, the first option counts.
  assert.deepEqual(active(run(Listbox.initial, [type('a', 0)], context(-1)).state), 0);
});

test('an empty list never opens, and a list emptied while open closes', () => {
  const empty = context(-1, []);
  for (const event of [{ kind: 'toggle' }, key('next'), type('a', 0)] as const)
    assert.equal(Listbox.step(Listbox.initial, event, empty).kind, 'ignored');
  assert.deepEqual(Listbox.step(open(2), key('next'), empty).state, Listbox.initial);
});

test('a highlight past a shortened list is clamped before it moves or commits', () => {
  const short = context(0, ['One', 'Two']);
  assert.deepEqual(run(open(5), [key('previous')], short).state, open(0));
  assert.deepEqual(run(open(5), [key('accept')], short).commits, [1]);
});

function active(state: ListboxState): number {
  assert.equal(state.kind, 'open');
  return state.kind === 'open' ? state.active : -1;
}
