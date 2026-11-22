import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { ManagementApi, type Project, type ProjectId } from '../src/lib/api.ts';
import { ProjectBoard } from '../src/lib/projects.ts';
import type { WorkItem } from '../src/lib/work.ts';

const pinned: typeof fetch = async () =>
  Response.json(
    JSON.parse(readFileSync(new URL('./fixtures/api/projects.json', import.meta.url), 'utf8')),
  );

/** A project id, as the API's would read. */
const pid = (id: string) => id as ProjectId;
const project = (name: string, overrides: Partial<Project> = {}): Project => ({
  id: pid(`id-${name}`),
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
      project('acme', { id: pid('acme-id') }),
      project('old', { archived_at: 5 }),
    ],
    [
      item('a', 'queued', { project: pid('acme-id') }),
      item('b', 'queued', { project: pid('acme-id') }),
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
  assert.equal(board.waiting, 4, 'one to promote, one preview, two projects with work to send');
  assert.equal(new ProjectBoard([], []).pending, false);
  assert.equal(new ProjectBoard([], []).waiting, 0);
});

test("a project's open work reads in words", () => {
  assert.equal(
    ProjectBoard.openWork(
      project('acme', {
        work: { queued: 2, sent: 1, landed: 0, shipped: 9, dismissed: 0, other: 0 },
      }),
    ),
    '2 queued · 1 sent',
  );
  assert.equal(ProjectBoard.openWork(project('acme')), 'none open');
  assert.equal(ProjectBoard.href('acme-web'), '/projects/acme-web/sessions');
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
        project: pid('acme-id'),
        first_prompt: 'Fix the pricing table',
      },
    },
  ] as never;
  const board = new ProjectBoard(
    [project('acme-web', { id: pid('acme-id') })],
    [],
    [working, idle] as never,
    sessions,
  );
  assert.deepEqual(board.approvals, [
    {
      session: 'session-1',
      approval: 'approval-1',
      ask: { kind: 'command', command: 'cargo test', cwd: null, reason: null },
      harness: 'codex',
      project: 'acme-web',
      prompt: 'Fix the pricing table',
    },
  ]);
  assert.ok(board.pending);
  // A run whose session is not listed still asks, for no project.
  assert.equal(new ProjectBoard([], [], [working] as never, []).approvals[0]?.project, null);
  // A project's own board holds its sessions' approvals alone, and its work.
  const acme = project('acme-web', { id: pid('acme-id') });
  const other = project('acme-api', { id: pid('api-id') });
  assert.equal(ProjectBoard.of(acme, [], [working] as never, sessions).approvals.length, 1);
  assert.equal(ProjectBoard.of(other, [], [working] as never, sessions).approvals.length, 0);
  const item = (id: string, owner: string | null) =>
    ({ id, project: owner, state: 'queued' }) as never;
  assert.deepEqual(
    ProjectBoard.of(acme, [item('a', 'acme-id'), item('b', 'api-id')], [], []).work.map(
      (entry) => entry.id,
    ),
    ['a'],
  );
});

test('news is what newly waits: a new approval, a newly passed canary; nothing at first sight', () => {
  const asking = (approval: string) =>
    ({
      run: 'run-1',
      session: 'session-1',
      harness: 'codex',
      workdir: '/work/acme-web',
      state: {
        phase: 'working',
        thread: 'thread',
        turn: null,
        approvals: [
          {
            approval,
            ask: { kind: 'command', command: 'git push', cwd: null, reason: null },
          },
        ],
        queued: null,
      },
      settings: { model: null, effort: null, approvals: null },
    }) as never;
  const sessions = [
    {
      session: 'session-1',
      served: null,
      stored: { id: 'session-1', project: pid('acme-id'), first_prompt: 'Ship the fix' },
    },
  ] as never;
  const canary = (commit: string | null) =>
    ({
      decision: commit ? 'awaiting_operator' : 'current',
      policy: 'manual',
      canary: commit
        ? {
            commit,
            checks: [
              { check: 'build', passed: true },
              { check: 'pages', passed: true },
            ],
          }
        : null,
      behind: commit ? 6 : 0,
    }) as unknown as Project['release'];
  const board = (approval: string | null, commit: string | null) =>
    new ProjectBoard(
      [project('acme-web', { id: pid('acme-id'), release: canary(commit) })],
      [],
      approval ? [asking(approval)] : [],
      sessions,
    );

  const quiet = board(null, null);
  assert.deepEqual(ProjectBoard.news(quiet, quiet), []);
  assert.deepEqual(ProjectBoard.news(quiet, board('approval-1', 'a3f9c21deadbeef0')), [
    {
      mark: 'yours',
      text: 'Run a command',
      detail: 'acme-web · Ship the fix',
      link: { label: 'Open session', href: '/projects/acme-web/sessions/session-1' },
    },
    {
      mark: 'passed',
      text: 'Canary a3f9c21deadb 2/2',
      detail: 'acme-web · production ↓6',
      link: { label: 'Release', href: '/projects/acme-web/release' },
    },
  ]);
  // What waited already is not news again; what stopped waiting says nothing.
  const waiting = board('approval-1', 'a3f9c21deadbeef0');
  assert.deepEqual(ProjectBoard.news(waiting, waiting), []);
  assert.deepEqual(ProjectBoard.news(waiting, quiet), []);
  // A new ask from the same run, or a newer canary, is news.
  assert.deepEqual(
    ProjectBoard.news(waiting, board('approval-2', 'b0b0b0b0b0b0b0b0')).map((said) => said.text),
    ['Run a command', 'Canary b0b0b0b0b0b0 2/2'],
  );
});

test('a client signed out on the host waits on the operator, and counts in Pending', () => {
  const client = (harness: string, kind: string) =>
    ({
      harness,
      status: 'ready',
      approvals: [],
      midway: [],
      sign_in: { kind },
      sign_in_methods: [],
    }) as never;
  const board = new ProjectBoard([], [], [], [], null, [
    client('codex', 'signed_out'),
    client('claude_code', 'signed_in'),
  ]);
  assert.deepEqual(
    board.signedOut.map((one) => one.harness),
    ['codex'],
  );
  assert.equal(board.waiting, 1);
  assert.ok(board.pending);
});

test('a canary that passed where Ren promotes offers Promote… from its toast', () => {
  const project_ = project('ren', {
    factory: 'ren',
    release: {
      decision: 'awaiting_operator',
      policy: 'manual',
      canary: { commit: 'abc123' },
    } as never,
  });
  const [said] = ProjectBoard.news(new ProjectBoard([], []), new ProjectBoard([project_], []));
  assert.deepEqual(said?.link, {
    label: 'Promote…',
    href: '/projects/ren/release?promote=abc123',
    emphasis: 'primary',
  });
});

test('a project at a glance: waiting on you first, then a failed canary, then running', () => {
  const release = (decision: string) =>
    ({ decision, policy: 'manual', canary: { commit: 'c', checks: [] }, behind: 1 }) as never;
  const glance = (overrides: Partial<Project>) =>
    new ProjectBoard([project('acme', overrides)], []).glance(project('acme', overrides));
  assert.deepEqual(
    [glance({ running: 2 }).mark, glance({ running: 2 }).action],
    ['responding', null],
  );
  assert.deepEqual(
    [
      glance({ release: release('failed'), running: 1 }).mark,
      glance({ release: release('failed') }).action,
    ],
    ['failed', { kind: 'release' }],
  );
  assert.deepEqual(glance({ release: release('awaiting_operator'), factory: 'ren' }).action, {
    kind: 'promote',
  });
  assert.equal(glance({}).active, null);
});

test('work in flight: queued, in a session, or landed, the most recently changed first', () => {
  const board = new ProjectBoard(
    [project('acme', { id: pid('acme-id') })],
    [
      item('a', 'queued', { updated_at: 10 }),
      item('b', 'shipped', { updated_at: 50 }),
      item('c', 'landed', { updated_at: 30 }),
      item('d', 'sent', { updated_at: 20 }),
      item('e', 'dismissed', { updated_at: 40 }),
    ],
  );
  assert.deepEqual(
    board.inFlight.map((work) => work.id),
    ['c', 'd', 'a'],
  );
});

test('a project is unknown only when the board read and has none by that name', () => {
  const acme = project('acme-web', { id: pid('acme-id') });
  const read = { projects: [acme], failed: null };
  assert.equal(ProjectBoard.unknown(read, acme.name), false);
  assert.equal(ProjectBoard.unknown(read, 'nowhere'), true);
  assert.equal(ProjectBoard.unknown({ projects: [], failed: 'down' }, 'nowhere'), false);
  assert.equal(ProjectBoard.unknown(null, 'nowhere'), false);
  assert.equal(ProjectBoard.known(read, acme.name), acme);
  assert.equal(ProjectBoard.known(null, acme.name), null);
});

test('a list that failed to read is unread, not empty', () => {
  const board = ProjectBoard.read({
    projects: [],
    work: { state: 'read', value: [] },
    live: { state: 'failed', reason: 'down' },
    sessions: { state: 'read', value: [] },
    clients: { state: 'failed', reason: 'down' },
  });
  assert.deepEqual(board.unread, ['runs', 'clients']);
  assert.equal(board.pending, false);
  assert.deepEqual(ProjectBoard.read(null).unread, []);
});
