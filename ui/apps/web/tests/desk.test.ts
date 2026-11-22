import test from 'node:test';
import assert from 'node:assert/strict';
import { Desk } from '../src/lib/desk.ts';
import { ProjectBoard } from '../src/lib/projects.ts';

const project = {
  id: 'acme-id',
  name: 'acme-web',
  archived_at: null,
  factory: 'contract',
} as never;
const run = (session: string, phase: string, approvals: unknown[] = []) =>
  ({
    run: `run-${session}`,
    session,
    harness: 'codex',
    workdir: '/work',
    state:
      phase === 'working'
        ? { phase, thread: 't', turn: null, approvals, queued: null }
        : { phase, thread: 't' },
    settings: { model: null, effort: null, approvals: null },
  }) as never;
const stored = (
  session: string,
  prompt: string,
  started: number,
  ended: number | null,
  end: string | null = null,
) =>
  ({
    session,
    served: null,
    worktree: null,
    stored: {
      id: session,
      project: 'acme-id',
      first_prompt: prompt,
      runs: [{ started_at: started, ended_at: ended, end, exit: null }],
    },
  }) as never;

test('what is under way puts the sessions waiting on the operator first, then the newest', () => {
  const board = new ProjectBoard(
    [project],
    [],
    [
      run('s1', 'working'),
      run('s2', 'working', [
        { approval: 'a', ask: { kind: 'command', command: 'ls', cwd: null, reason: null } },
      ]),
      run('s3', 'ended'),
    ],
    [
      stored('s1', 'Paginate', 100, null),
      stored('s2', 'Retry', 50, null),
      stored('s3', 'Old', 10, 20),
    ],
  );
  const rows = Desk.underWay(board);
  assert.deepEqual(
    rows.map((row) => [row.title, row.mark, row.project]),
    [
      ['Retry', 'yours', 'acme-web'],
      ['Paginate', 'responding', 'acme-web'],
    ],
  );
  assert.equal(rows[0]?.href, '/projects/acme-web/sessions?session=s2');
});

test('outcomes since a time: sessions that ended and work that landed or shipped, newest first', () => {
  const board = new ProjectBoard(
    [project],
    [
      { id: 'w1', title: 'Promo banner', state: 'landed', updated_at: 300, project: 'acme-id' },
      { id: 'w2', title: 'Old work', state: 'shipped', updated_at: 50, project: 'acme-id' },
      { id: 'w3', title: 'Still queued', state: 'queued', updated_at: 400, project: 'acme-id' },
    ] as never,
    [],
    [
      stored('s1', 'Explain the queue', 100, 250, 'idled'),
      stored('s2', 'Crashed run', 100, 350, 'spawn_failed'),
      stored('s3', 'Ended before', 10, 90, 'idled'),
    ],
  );
  const outcomes = Desk.outcomes(board, 100);
  assert.deepEqual(
    outcomes.map((one) => [one.title, one.mark, one.happened]),
    [
      ['Crashed run', 'failed', 'Ended short'],
      ['Promo banner', 'passed', 'Landed on main'],
      ['Explain the queue', 'passed', 'Ended'],
    ],
  );
  assert.deepEqual(Desk.counts(outcomes), { ended: 2, landed: 1, shipped: 0, failed: 1 });
});

test('when the operator looked reads as a phrase', () => {
  const now = 10 * 86_400_000;
  const day = () => 'Sat';
  assert.equal(Desk.looked(now - 10_000, now, day), 'just now');
  assert.equal(Desk.looked(now - 3 * 3_600_000, now, day), '3h ago');
  assert.equal(Desk.looked(now - 2 * 86_400_000, now, day), 'since Sat');
});
