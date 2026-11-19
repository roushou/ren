import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { ManagementApi, type Project } from '../src/lib/api.ts';
import { ProjectBoard } from '../src/lib/projects.ts';
import type { WorkItem } from '../src/lib/work.ts';

const pinned: typeof fetch = async () =>
  Response.json(
    JSON.parse(readFileSync(new URL('./fixtures/api/projects.json', import.meta.url), 'utf8')),
  );

const project = (name: string, overrides: Partial<Project> = {}): Project => ({
  id: `id-${name}`,
  name,
  repository: `/home/op/dev/${name}`,
  factory: 'contract',
  created_at: 0,
  archived_at: null,
  work: { queued: 0, sent: 0, landed: 0, shipped: 0, dismissed: 0, other: 0 },
  sessions: 0,
  running: 0,
  release: null,
  ...overrides,
});

const item = (id: string, state: WorkItem['state'], extra: Partial<WorkItem> = {}): WorkItem =>
  ({
    id,
    created_at: 0,
    updated_at: 0,
    title: id,
    order: { page: { path: '/' }, marks: [], evidence: { console: [], reads: [] } },
    state,
    project: null,
    session: null,
    landed: null,
    shipped: null,
    preview: null,
    ...extra,
  }) as unknown as WorkItem;

test("the daemon's projects decode, and Ren itself is the one registered as Ren", async () => {
  const projects = await new ManagementApi(pinned).projects();
  assert.deepEqual(
    projects.map((project) => [project.name, project.factory]),
    [
      ['acme-web', 'contract'],
      ['ren', 'ren'],
    ],
  );
  assert.equal(ManagementApi.ren(projects)?.name, 'ren');
  assert.equal(ManagementApi.ren([]), null);
  // Two registered as Ren, or Ren archived, names none.
  const ren = project('ren', { factory: 'ren' });
  assert.equal(ManagementApi.ren([ren, project('ren2', { factory: 'ren' })]), null);
  assert.equal(ManagementApi.ren([{ ...ren, archived_at: 1 }]), null);
});

test('the board puts Ren first, and gathers what waits on the operator', () => {
  const release = (decision: string) =>
    ({ decision, policy: 'manual', canary: null }) as unknown as Project['release'];
  const board = new ProjectBoard(
    [
      project('zeta', { release: release('awaiting_operator') }),
      project('ren', { factory: 'ren', release: release('current') }),
      project('acme', { id: 'acme-id' }),
      project('old', { archived_at: 5 }),
    ],
    [
      item('a', 'queued', { project: 'acme-id' }),
      item('b', 'queued', { project: 'acme-id' }),
      item('c', 'queued'),
      item('d', 'sent', {
        preview: { ui: 'https://acme.example:20100', revision: { kind: 'unknown' } },
      } as Partial<WorkItem>),
      item('e', 'sent'),
    ],
  );
  assert.deepEqual(
    board.active.map((project) => project.name),
    ['ren', 'acme', 'zeta'],
  );
  assert.deepEqual(
    board.archived.map((project) => project.name),
    ['old'],
  );
  assert.deepEqual(
    board.toPromote.map((project) => project.name),
    ['zeta'],
  );
  assert.deepEqual(
    board.previews.map((item) => item.id),
    ['d'],
  );
  // An item from before projects is Ren's.
  assert.deepEqual(board.queued, [
    { project: 'acme', count: 2 },
    { project: 'ren', count: 1 },
  ]);
  assert.ok(board.pending);
  assert.equal(new ProjectBoard([], []).pending, false);
});

test("a project's work groups by state, open first, and its open work reads in words", () => {
  const groups = ProjectBoard.groups([
    item('s', 'shipped'),
    item('q', 'queued'),
    item('l', 'landed'),
  ]);
  assert.deepEqual(
    groups.map((group) => [group.state, group.items.length]),
    [
      ['queued', 1],
      ['landed', 1],
      ['shipped', 1],
    ],
  );
  assert.equal(
    ProjectBoard.openWork(
      project('acme', {
        work: { queued: 2, sent: 1, landed: 0, shipped: 9, dismissed: 0, other: 0 },
      }),
    ),
    '2 queued · 1 sent',
  );
  assert.equal(ProjectBoard.openWork(project('acme')), 'none open');
  assert.equal(ProjectBoard.href('acme-web'), '/projects/acme-web');
  assert.equal(
    ProjectBoard.releaseHref(project('ren', { factory: 'ren' })),
    '/projects/ren/release',
  );
  assert.equal(ProjectBoard.releaseHref(project('acme')), '/projects/acme/release');
});

test('approvals come from working runs, each named by its session and project', () => {
  const working = {
    run: 'run-1' as never,
    session: 'session-1' as never,
    harness: 'codex',
    workdir: '/work/acme-web',
    state: {
      phase: 'working',
      thread: 'thread' as never,
      turn: null,
      approvals: [
        {
          approval: 'approval-1' as never,
          ask: { kind: 'command', command: 'cargo test', cwd: null, reason: null },
        },
      ],
      queued: null,
    },
    settings: { model: null, effort: null, approvals: null },
  } as const;
  const idle = {
    ...working,
    run: 'run-2' as never,
    session: 'session-2' as never,
    state: { phase: 'idle', thread: 'thread' as never },
  } as const;
  const sessions = [
    {
      session: 'session-1' as never,
      served: null,
      stored: {
        id: 'session-1' as never,
        project: 'acme-id',
        first_prompt: 'Fix the pricing table',
      },
    },
  ] as never;
  const board = new ProjectBoard(
    [project('acme-web', { id: 'acme-id' })],
    [],
    [working, idle] as never,
    sessions,
  );
  assert.deepEqual(board.approvals, [
    {
      session: 'session-1',
      approval: 'approval-1',
      ask: { kind: 'command', command: 'cargo test', cwd: null, reason: null },
      project: 'acme-web',
      prompt: 'Fix the pricing table',
    },
  ]);
  assert.ok(board.pending);
  // A run whose session is not listed still asks, for no project.
  assert.equal(new ProjectBoard([], [], [working] as never, []).approvals[0]?.project, null);
});
