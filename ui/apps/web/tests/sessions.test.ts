import test from 'node:test';
import assert from 'node:assert/strict';
import {
  RunsClient,
  Sessions,
  type RunSnapshot,
  type SessionBody,
  type RunId,
  type SessionId,
} from '../src/lib/runs.ts';

const sessionA = '0190f3a0-0000-7000-8000-00000000a001' as SessionId;
const sessionB = '0190f3a0-0000-7000-8000-00000000b001' as SessionId;
const sessionC = '0190f3a0-0000-7000-8000-00000000c001' as SessionId;
const runA1 = '0190f3a0-0000-7000-8000-00000000a011' as RunId;
const runA2 = '0190f3a0-0000-7000-8000-00000000a012' as RunId;
const runB = '0190f3a0-0000-7000-8000-00000000b011' as RunId;
const thread = '0190f3a0-0000-7000-8000-0000000000cc';

function recorded(id: RunId, startedAt: number, ended: boolean) {
  return {
    id,
    harness: 'codex',
    workdir: '/work/acme',
    started_at: startedAt,
    thread,
    recording: null,
    ended_at: ended ? startedAt + 10 : null,
    end: ended ? 'stopped' : null,
    exit: null,
    archived_at: null,
    model: null,
    effort: null,
    approvals: null,
  };
}

/** What the daemon answers for the listing: stored, served, or both. */
const listing = {
  api_version: 1,
  sessions: [
    {
      session: sessionA,
      stored: {
        id: sessionA,
        harness: 'codex',
        workdir: '/work/acme',
        created_at: 100,
        model: 'gpt-deep',
        effort: 'high',
        approvals: null,
        archived_at: null,
        first_turn_at: 120,
        first_prompt: 'fix the order list sorting',
        runs: [recorded(runA2, 300, false), recorded(runA1, 100, true)],
      },
      served: null,
    },
    {
      session: sessionB,
      stored: {
        id: sessionB,
        harness: 'claude_code',
        workdir: '/work/acme',
        created_at: 50,
        model: null,
        effort: null,
        approvals: null,
        archived_at: null,
        first_turn_at: null,
        first_prompt: null,
        runs: [recorded(runB, 50, true)],
      },
      served: null,
    },
    {
      // Just made: served, not stored yet.
      session: sessionC,
      stored: null,
      served: {
        session: sessionC,
        harness: 'pi',
        workdir: '/scratch/scratch-1',
        settings: { model: null, effort: null, approvals: null },
        state: { shelf: 'open', phase: 'draft' },
        process: null,
      },
    },
  ],
};

async function sessions(): Promise<SessionBody[]> {
  const client = new RunsClient((async () => Response.json(listing)) as unknown as typeof fetch);
  return client.sessions();
}

test('sessions read as stored and served, and group by folder with the busiest first', async () => {
  const bodies = await sessions();
  const live: RunSnapshot[] = [
    {
      run: runA2,
      harness: 'codex',
      workdir: '/work/acme',
      state: {
        phase: 'working',
        thread: thread as never,
        turn: null,
        approvals: [{ approval: 'ap-1' as never, ask: { kind: 'file_change', reason: null } }],
        queued: null,
      },
      settings: { model: null, effort: null, approvals: null },
    },
  ];
  // Drafts never asked anything are left out, unless open.
  assert.deepEqual(
    Sessions.folders(bodies, live, '/scratch').map((folder) => [
      folder.name,
      folder.sessions.map((entry) => entry.session),
    ]),
    [['acme', [sessionA]]],
  );
  const folders = Sessions.folders(bodies, live, '/scratch', sessionB);
  assert.deepEqual(
    folders.map((folder) => [folder.name, folder.sessions.map((entry) => entry.session)]),
    [['acme', [sessionA, sessionB]]],
  );
  const [a, b] = folders[0]!.sessions;
  assert.equal(folders[0]!.waiting, 1);
  // Its newest process, live; its settings and phase as stored.
  assert.equal(a!.run, runA2);
  assert.equal(a!.state?.phase, 'working');
  assert.equal(a!.phase, 'active');
  assert.equal(a!.settings.effort, 'high');
  assert.equal(Sessions.running(a!), true);
  // Not held: its newest run's recorded end; never prompted, a draft.
  assert.deepEqual(b!.state, { phase: 'ended', end: 'stopped', exit: null });
  assert.equal(b!.phase, 'draft');
  assert.equal(Sessions.running(b!), false);
  const scratchFolders = Sessions.folders(bodies, live, '/scratch', sessionC);
  assert.deepEqual(
    scratchFolders.map((folder) => folder.name),
    ['acme', 'Scratch'],
  );
  const fresh = scratchFolders[1]!.sessions[0]!;
  assert.equal(fresh.run, null);
  assert.equal(fresh.state, null);
  assert.equal(fresh.activeAt, null);
});

test('a session is named by what it was first asked, as stored', async () => {
  const bodies = await sessions();
  const [a, b] = [sessionA, sessionB].map((id) =>
    Sessions.desk(
      (bodies as SessionBody[]).find((body) => body.session === id)!,
      [],
    ),
  );
  assert.equal(a!.asked, 'fix the order list sorting');
  assert.equal(b!.asked, null);
});

test('a new session reuses the newest draft never asked anything, in any folder', async () => {
  const bodies = await sessions();
  // The served draft not yet stored is the newest; acme's stored one is next.
  assert.equal(Sessions.unused(bodies, [])?.session, sessionC);
  const withoutC = bodies.filter((body) => body.session !== sessionC);
  assert.equal(Sessions.unused(withoutC, [])?.session, sessionB);
  // An archived draft is not reused, nor a session asked something.
  const archived = withoutC.map((body) =>
    body.session === sessionB && body.stored
      ? { ...body, stored: { ...body.stored, archived_at: 1 as never } }
      : body,
  );
  assert.equal(Sessions.unused(archived, []), null);
});

test('a new session starts where the last session asked something worked', async () => {
  const bodies = await sessions();
  // acme's session was asked something; the drafts were not.
  assert.equal(Sessions.start(bodies, [], '/scratch'), '/work/acme');
  const drafts = bodies.filter((body) => body.session !== sessionA);
  // With none asked anything: a scratch folder of its own, where there is scratch.
  assert.equal(Sessions.start(drafts, [], '/scratch'), null);
  assert.equal(Sessions.start(drafts, [], null), undefined);
  // The last one asked worked in scratch: a scratch folder of its own again.
  const inScratch = bodies.map((body) =>
    body.session === sessionA && body.stored
      ? { ...body, stored: { ...body.stored, workdir: '/scratch/scratch-9' } }
      : body,
  );
  assert.equal(Sessions.start(inScratch, [], '/scratch'), null);
  assert.equal(Sessions.inScratch('/scratch/scratch-9', '/scratch'), true);
  assert.equal(Sessions.inScratch('/scratch', '/scratch'), false);
  assert.equal(Sessions.inScratch('/work/acme', null), false);
});

test('a run leads to its session', async () => {
  const bodies = await sessions();
  assert.equal(Sessions.of(bodies, runA1), sessionA);
  assert.equal(Sessions.of(bodies, runB), sessionB);
  assert.equal(Sessions.of(bodies, '0190f3a0-0000-7000-8000-00000000ffff' as RunId), null);
});

test('session commands go to their routes', async () => {
  const seen: string[] = [];
  const client = new RunsClient((async (input: RequestInfo | URL, init?: RequestInit) => {
    seen.push(`${init?.method} ${String(input)} ${init?.body ?? ''}`);
    return Response.json({ api_version: 1, session: sessionA });
  }) as typeof fetch);
  assert.equal(await client.createSession('codex', '/work/acme', { effort: 'high' }), sessionA);
  await client.toSession(sessionA, { kind: 'prompt', text: 'go', delivery: 'queue' });
  await client.toSession(sessionA, { kind: 'client', harness: 'pi' });
  await client.toSession(sessionA, { kind: 'archive' });
  await client.toSession(sessionA, { kind: 'delete' });
  assert.deepEqual(seen, [
    'POST /control/sessions {"harness":"codex","dir":"/work/acme","effort":"high"}',
    `POST /control/sessions/${sessionA}/prompt {"text":"go","delivery":"queue"}`,
    `POST /control/sessions/${sessionA}/client {"harness":"pi"}`,
    `POST /control/sessions/${sessionA}/archive {}`,
    `POST /control/sessions/${sessionA}/delete {}`,
  ]);
});
