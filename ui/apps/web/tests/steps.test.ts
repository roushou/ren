import test from 'node:test';
import assert from 'node:assert/strict';
import type { CommandIntent, Entry, ItemId } from '../src/lib/runs.ts';
import { Steps } from '../src/lib/steps.ts';

const WORK = '/work/acme';
const id = (text: string) => text as ItemId;

function command(
  item: string,
  text: string,
  intent: CommandIntent = { said: 'unsaid' },
  status = 'completed',
  exit: number | null = 0,
): Entry {
  return { kind: 'command', item: id(item), command: text, intent, status, output: null, exit };
}

function read(item: string, path: string, done = true): Entry {
  return {
    kind: 'tool',
    item: id(item),
    name: 'Read',
    effects: [{ kind: 'read_file', target: `${WORK}/${path}` }],
    done,
  };
}

const said = (text: string): Entry => ({ kind: 'message', item: id(text), text, done: true });

test('steps between two things said gather into one group, keyed by its first step', () => {
  const blocks = Steps.blocks(
    [
      said('Looking at the tests.'),
      read('r1', 'tests/a.rs'),
      { kind: 'reasoning', item: id('th'), done: true },
      command('c1', 'cargo test'),
      said('Fixing it.'),
      { kind: 'file_change', item: id('f1'), paths: [`${WORK}/src/lib.rs`], status: 'completed' },
    ],
    WORK,
  );
  assert.deepEqual(
    blocks.map((block) =>
      block.kind === 'steps' ? `steps ${block.key} ${block.steps.length}` : block.entry.kind,
    ),
    ['message', 'steps r1 2', 'message', 'steps f1 1'],
  );
  // Thinking still under way shows, and parts a group.
  const thinking = Steps.blocks(
    [read('r1', 'a.rs'), { kind: 'reasoning', item: id('th'), done: false }, read('r2', 'b.rs')],
    WORK,
  );
  assert.equal(thinking.length, 3);
});

test('a group runs while a step runs, and failed once one failed', () => {
  const state = (entries: Entry[]) => {
    const [block] = Steps.blocks(entries, WORK);
    assert.equal(block?.kind, 'steps');
    return block.kind === 'steps' ? block.state : null;
  };
  assert.equal(state([read('r1', 'a.rs'), command('c1', 'ls')]), 'done');
  assert.equal(state([read('r1', 'a.rs'), read('r2', 'b.rs', false)]), 'running');
  assert.equal(
    state([command('c1', 'cargo test', undefined, 'completed', 101), read('r1', 'a.rs')]),
    'failed',
  );
  assert.equal(state([command('c1', 'rm -rf x', undefined, 'declined', null)]), 'done');
});

test('a command reads as its client said it, and as written only when it said nothing', () => {
  const described = Steps.read(
    command('c1', `cd ${WORK} && cargo test`, {
      said: 'described',
      text: 'Run the tests',
    }) as never,
    WORK,
  );
  assert.deepEqual(
    [described.act, described.targets, described.said],
    ['run', ['cargo test'], 'Run the tests'],
  );
  // Codex's parts, all one act: that act on each part's target.
  const parted = Steps.read(
    command('c2', 'sed -n 1,9p a.rs; cat b.rs', {
      said: 'parts',
      parts: [
        { part: 'read', command: 'sed -n 1,9p a.rs', path: `${WORK}/a.rs`, name: 'a.rs' },
        { part: 'read', command: 'cat b.rs', path: `${WORK}/src/b.rs`, name: 'b.rs' },
      ],
    }) as never,
    WORK,
  );
  assert.deepEqual([parted.act, parted.targets], ['read', ['a.rs', 'src/b.rs']]);
  // Mixed or unreadable parts: the command as written.
  const mixed = Steps.read(
    command('c3', 'rg todo && make', {
      said: 'parts',
      parts: [
        { part: 'search', command: 'rg todo', query: 'todo', path: null },
        { part: 'unknown', command: 'make' },
      ],
    }) as never,
    WORK,
  );
  assert.deepEqual([mixed.act, mixed.targets], ['run', ['rg todo && make']]);
  const failed = Steps.read(command('c4', 'cargo test', undefined, 'failed', 101) as never, WORK);
  assert.deepEqual([failed.state, failed.failure], ['failed', 'exit 101']);
  const bare = Steps.read(command('c5', 'rm -rf build', undefined, 'failed', null) as never, WORK);
  assert.deepEqual(
    [bare.state, bare.failure],
    ['failed', null],
    'a failure with no exit code says nothing more than its mark',
  );
});

test('a tool reads as its effects, and as its name without any', () => {
  const tool = Steps.read(read('r1', 'src/lib.rs') as never, WORK);
  assert.deepEqual([tool.act, tool.targets], ['read', ['src/lib.rs']]);
  const named = Steps.read(
    { kind: 'tool', item: id('t1'), name: 'TodoWrite plan', effects: [], done: false } as never,
    WORK,
  );
  assert.deepEqual(
    [named.act, named.targets, named.state],
    ['tool', ['TodoWrite plan'], 'running'],
  );
  const unknown = Steps.read(
    {
      kind: 'tool',
      item: id('t2'),
      name: 'Teleport',
      effects: [{ kind: 'teleport' as never, target: 'mars' }],
      done: true,
    } as never,
    WORK,
  );
  assert.deepEqual([unknown.act, unknown.targets], ['tool', ['mars']]);
});

test('a group reads in a line, by kind, files read again counted once', () => {
  const steps = [
    read('r1', 'tests/supervisor.rs'),
    read('r2', 'tests/supervisor.rs'),
    read('r3', 'src/task.rs'),
    command('c1', 'rg -n adopt', {
      said: 'parts',
      parts: [{ part: 'search', command: 'rg -n adopt', query: 'fn adopt', path: null }],
    }),
    command('c2', 'cargo test'),
    command('c3', 'cargo clippy'),
    {
      kind: 'file_change',
      item: id('f1'),
      paths: [`${WORK}/a.rs`, `${WORK}/b.rs`, `${WORK}/c.rs`],
      status: 'completed',
    },
  ].map((entry) => Steps.read(entry as never, WORK));
  assert.equal(
    Steps.summary(steps),
    'Read supervisor.rs ×2, task.rs · Searched “fn adopt” · Ran 2 commands · Edited 3 files',
  );
  // One command its client described reads as its words.
  assert.equal(
    Steps.summary([
      Steps.read(
        command('c9', 'cargo test', { said: 'described', text: 'Run the tests' }) as never,
        WORK,
      ),
    ]),
    'Run the tests',
  );
});

test('a step’s time reads in tenths, then seconds, then minutes', () => {
  assert.deepEqual(
    [0, 420, 2_500, 9_949, 12_400, 59_400, 64_000, 3_725_000].map((ms) => Steps.time(ms)),
    ['0.0 s', '0.4 s', '2.5 s', '9.9 s', '12 s', '59 s', '1:04', '62:05'],
  );
});

test('picked steps quote as a prompt’s list, and copy as commands with their output', () => {
  const steps = [
    Steps.read(command('c1', 'cargo test', undefined, 'completed', 101) as never, WORK),
    Steps.read(read('r1', 'src/lib.rs') as never, WORK),
  ];
  assert.equal(
    Steps.quote(steps),
    'About these steps:\n- Run `cargo test` (failed, exit 101)\n- Read `src/lib.rs`',
  );
  assert.equal(Steps.quote(steps.slice(1)), 'About this step:\n- Read `src/lib.rs`');
  assert.equal(Steps.text(steps), '$ cargo test\n\nRead src/lib.rs');
});
