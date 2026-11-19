import test from 'node:test';
import assert from 'node:assert/strict';
import type { Mark, PageContext } from 'redline';
import { Briefing, type RenPage } from '../src/lib/briefing.ts';
import type {
  DeskSession,
  RunClient,
  RunId,
  RunSnapshot,
  SessionBody,
  SessionId,
} from '../src/lib/runs.ts';

const page: PageContext = {
  path: '/sessions?session=s1',
  title: 'Ren - Sessions',
  width: 1400,
  height: 820,
  theme: 'dark',
};

const ren: RenPage = {
  route: '/sessions',
  revision: { kind: 'commit', commit: '97aa93b'.padEnd(40, '0'), tree: 'clean' },
};

function mark(id: number, note: string): Mark {
  return {
    id,
    note,
    target: {
      selector: '[aria-label="Run"] div.entry.tool > span.chip.success',
      text: 'completed',
      source: id === 1 ? 'src/routes/sessions/+page.svelte:861' : null,
      width: 62.4,
      height: 18,
    },
  };
}

test('the brief lists each noted mark with what it points at', () => {
  const brief = Briefing.brief(
    page,
    ren,
    [mark(1, 'Move it to the end.'), mark(2, '  ')],
    'session',
  );
  assert.equal(
    brief,
    [
      'Redline: Move it to the end.',
      '',
      'Marked on `/sessions?session=s1` (Ren - Sessions), 1400×820, dark theme.',
      "In Ren's UI this page is `ui/apps/web/src/routes/sessions/+page.svelte`.",
      'This page was served by a build of commit `97aa93b00000`; the checkout may have moved on since.',
      '',
      '1. Move it to the end.',
      '   - Element: `[aria-label="Run"] div.entry.tool > span.chip.success`, “completed”',
      '   - Source: `src/routes/sessions/+page.svelte:861`',
      '   - Size: 62×18',
      '',
      "Answer what the notes ask without changing anything. If a note asks for a change, make it on a new branch, `redline/move-it-to-the-end`, in a git worktree of this repository unless you already work in one, never in a checkout other sessions share, and run the project's checks. Don't install or restart anything: I'll review the branch.",
    ].join('\n'),
  );
  assert.equal(Briefing.routeFile('/'), 'ui/apps/web/src/routes/+page.svelte');
});

test('a new session works on its brief in the worktree Ren made for it', () => {
  const brief = Briefing.brief(page, ren, [mark(1, 'Move it to the end.')], 'worktree');
  assert.equal(
    brief.split('\n').at(-1),
    "Answer what the notes ask without changing anything. If a note asks for a change, make it here: this session works in a worktree of its own, made from the repository on a branch of its own. Run the project's checks and commit on that branch. Don't install or restart anything: I'll review the branch.",
  );
});

test('a new session gets a worktree, and an open one works where it is', () => {
  const session = { session: 'a' as SessionId } as DeskSession;
  assert.equal(Briefing.place({ kind: 'new' }), 'worktree');
  assert.equal(Briefing.place({ kind: 'session', session }), 'session');
});

test('the build is named when the daemon says what it was built from', () => {
  assert.equal(
    Briefing.built({ kind: 'commit', commit: 'a'.repeat(40), tree: 'modified' }),
    'This page was served by a build of commit `aaaaaaaaaaaa` with uncommitted changes; the checkout may have moved on since.',
  );
  assert.equal(Briefing.built({ kind: 'unknown' }), null);
  assert.equal(Briefing.built(null), null);
});

test('the source checkout is an absolute path, or absent', () => {
  assert.equal(Briefing.checkout(undefined), undefined);
  assert.equal(Briefing.checkout(''), undefined);
  assert.equal(Briefing.checkout('/home/you/ren/'), '/home/you/ren');
  assert.equal(Briefing.checkout('/'), '/');
  assert.throws(() => Briefing.checkout('ren'), /absolute/);
});

function stored(id: string, workdir: string, createdAt: number, archived = false): SessionBody {
  return {
    session: id as SessionId,
    served: null,
    worktree: null,
    stored: {
      id: id as SessionId,
      harness: 'codex',
      workdir,
      project: null,
      created_at: createdAt as never,
      model: null,
      effort: null,
      approvals: null,
      archived_at: archived ? (createdAt as never) : null,
      first_turn_at: createdAt as never,
      first_prompt: `asked ${id}`,
      runs: [],
    },
  };
}

test('the sessions offered are those running in the checkout, newest first', () => {
  const ids = [1, 2, 3, 4, 5].map((n) => `0190f3a0-0000-7000-8000-00000000a00${n}`);
  const bodies = [
    stored(ids[0]!, '/home/you/ren', 100),
    stored(ids[1]!, '/home/you/ren', 300),
    stored(ids[2]!, '/home/you/ren', 400, true),
    stored(ids[3]!, '/home/you/ren-other', 500),
    // Ended: its client is not running, so it is not offered.
    stored(ids[4]!, '/home/you/ren', 600),
  ];
  const live = ids.slice(0, 4).map((id, index) => {
    const body = bodies[index]!;
    const run = `0190f3a0-0000-7000-8000-00000000b00${index}` as RunId;
    body.stored!.runs = [
      {
        id: run,
        harness: 'codex',
        workdir: body.stored!.workdir,
        started_at: body.stored!.created_at,
        thread: null,
        recording: null,
        operator_session: id,
        ended_at: null,
        end: null,
        exit: null,
        archived_at: null,
        model: null,
        effort: null,
        approvals: null,
      } as never,
    ];
    return {
      run,
      session: 'session' as never,
      harness: 'codex',
      workdir: body.stored!.workdir,
      state: { phase: 'idle', thread: 'thread' as never },
      settings: { model: null, effort: null, approvals: null },
    } satisfies RunSnapshot;
  });
  assert.deepEqual(
    Briefing.offered(bodies, live, '/home/you/ren').map((session) => session.session),
    [ids[1], ids[0]],
  );
  assert.deepEqual(Briefing.offered(bodies, live, '/nowhere'), []);
});

test('a brief starts a turn, or is queued after the running one', () => {
  assert.deepEqual(Briefing.handoff({ kind: 'start' }, 'B'), {
    kind: 'send',
    delivery: 'start',
    text: 'B',
  });
  assert.deepEqual(Briefing.handoff({ kind: 'continue' }, 'B').kind, 'send');
  // Queued after what was queued before, as one message: a queue replaces.
  assert.deepEqual(Briefing.handoff({ kind: 'during', steer: true, queued: 'A' }, 'B'), {
    kind: 'send',
    delivery: 'queue',
    text: 'A\n\nB',
  });
  assert.deepEqual(Briefing.handoff({ kind: 'during', steer: false, queued: null }, 'B'), {
    kind: 'send',
    delivery: 'queue',
    text: 'B',
  });
  assert.deepEqual(Briefing.handoff({ kind: 'closed', reason: 'stopping' }, 'B'), {
    kind: 'wait',
    reason: 'stopping',
  });
});

test('without a route or a build, the brief says nothing of either', () => {
  const brief = Briefing.brief(
    page,
    { route: null, revision: null },
    [mark(1, 'Move it.')],
    'worktree',
  );
  assert.ok(brief.includes('dark theme.\n\n1. Move it.'));
});

test('a signed-out client is never switched from on its own, only offered another', () => {
  const client = (harness: RunClient['harness'], sign_in: RunClient['sign_in']): RunClient => ({
    harness,
    status: 'ready',
    approvals: [],
    midway: [],
    sign_in,
    sign_in_methods: [],
  });
  const clients = [
    client('claude_code', { kind: 'signed_out' }),
    client('codex', { kind: 'not_checked' }),
    client('pi', { kind: 'signed_out' }),
  ];
  assert.equal(Briefing.instead(['claude_code', 'pi', 'codex'], clients, 'claude_code'), 'codex');
  assert.equal(Briefing.instead(['claude_code', 'pi'], clients, 'claude_code'), null);
});
