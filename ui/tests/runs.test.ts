import test from 'node:test';
import assert from 'node:assert/strict';
import {
  Desk,
  RunCommandError,
  Runs,
  RunsClient,
  Transcript,
  runUpdate,
  type DriveEvent,
  type RecordedRun,
  type RunSnapshot,
} from '../src/lib/runs.ts';

const run = '0190f3a0-0000-7000-8000-0000000000aa';
const other = '0190f3a0-0000-7000-8000-0000000000bb';
const thread = '0190f3a0-0000-7000-8000-0000000000cc';

function event(value: unknown): DriveEvent {
  const update = runUpdate({ kind: 'event', run, event: value }, '');
  assert.equal(update.kind, 'event');
  return update.event;
}

test('run states parse as the control API sends them, keeping newer phases and labels', () => {
  const waiting = runUpdate(
    {
      kind: 'state',
      run,
      state: {
        phase: 'working',
        thread,
        turn: null,
        approvals: [
          {
            approval: 'ap-1',
            ask: { kind: 'command', command: 'cargo test', cwd: '/work/acme', reason: null },
          },
        ],
      },
    },
    '',
  );
  assert.deepEqual(waiting, {
    kind: 'state',
    run,
    state: {
      phase: 'working',
      thread,
      turn: null,
      approvals: [
        {
          approval: 'ap-1',
          ask: { kind: 'command', command: 'cargo test', cwd: '/work/acme', reason: null },
        },
      ],
    },
  });
  const ended = runUpdate(
    {
      kind: 'state',
      run,
      state: { phase: 'ended', end: 'stopped', exit: { kind: 'signal', value: 9 } },
    },
    '',
  );
  assert.deepEqual(ended.kind === 'state' && ended.state, {
    phase: 'ended',
    end: 'stopped',
    exit: { kind: 'signal', value: 9 },
  });
  const newer = runUpdate({ kind: 'state', run, state: { phase: 'paused' } }, '');
  assert.deepEqual(newer.kind === 'state' && newer.state, {
    phase: 'unrecognized',
    text: 'paused',
  });
  const lost = runUpdate(
    { kind: 'state', run, state: { phase: 'ended', end: 'vanished', exit: null } },
    '',
  );
  assert.equal(
    lost.kind === 'state' && lost.state.phase === 'ended' && Runs.endLabel(lost.state.end),
    'vanished',
  );
  assert.throws(() => runUpdate({ kind: 'state', run: 'nope', state: { phase: 'starting' } }, ''));
});

test('controls follow the state: prompt when idle, answer and interrupt while working', () => {
  assert.deepEqual(Runs.controls({ phase: 'idle', thread: thread as never }), {
    prompt: true,
    interrupt: false,
    stop: true,
    approvals: [],
  });
  const working = Runs.controls({
    phase: 'working',
    thread: thread as never,
    turn: null,
    approvals: [],
  });
  assert.equal(working.prompt, false);
  assert.equal(working.interrupt, false, 'no turn to interrupt until the agent names it');
  const ended = Runs.controls({ phase: 'ended', end: 'exited', exit: { kind: 'code', value: 1 } });
  assert.deepEqual(ended, { prompt: false, interrupt: false, stop: false, approvals: [] });
  assert.deepEqual(
    Runs.mark({
      phase: 'working',
      thread: thread as never,
      turn: null,
      approvals: [{ approval: 'a' as never, ask: { kind: 'file_change', reason: null } }],
    }),
    {
      label: 'needs you',
      tone: 'warn',
    },
  );
});

test('the transcript streams a reply, then commands and file changes as items update', () => {
  let transcript = new Transcript().prompted('fix the failing test');
  for (const value of [
    { kind: 'turn_started', turn: 'turn-1' },
    {
      kind: 'item',
      phase: 'started',
      item: {
        id: 'u1',
        kind: 'user_message',
        read: { as: 'prompt', text: 'fix the failing test' },
      },
    },
    {
      kind: 'item',
      phase: 'started',
      item: {
        id: 'c1',
        kind: 'command_execution',
        read: {
          as: 'command',
          command: 'cargo test',
          status: 'in_progress',
          output: null,
          exit_code: null,
        },
      },
    },
    {
      kind: 'item',
      phase: 'completed',
      item: {
        id: 'c1',
        kind: 'command_execution',
        read: {
          as: 'command',
          command: 'cargo test',
          status: 'completed',
          output: 'ok',
          exit_code: 0,
        },
      },
    },
    {
      kind: 'item',
      phase: 'completed',
      item: {
        id: 'f1',
        kind: 'file_change',
        read: { as: 'file_change', paths: ['src/lib.rs'], status: 'completed' },
      },
    },
    { kind: 'delta', item: 'm1', text: 'Fixed ' },
    { kind: 'delta', item: 'm1', text: 'it.' },
    { kind: 'approval', approval: 'ap-1', ask: { kind: 'file_change', reason: null } },
    { kind: 'turn_ended', turn: 'turn-1', end: { kind: 'completed' } },
    { kind: 'brand_new', anything: true },
  ]) {
    transcript = transcript.apply(event(value));
  }
  assert.equal(transcript.turn, 'turn-1');
  assert.deepEqual(transcript.entries, [
    { kind: 'prompt', text: 'fix the failing test' },
    {
      kind: 'command',
      item: 'c1',
      command: 'cargo test',
      status: 'completed',
      output: 'ok',
      exit: 0,
    },
    { kind: 'file_change', item: 'f1', paths: ['src/lib.rs'], status: 'completed' },
    { kind: 'message', item: 'm1', text: 'Fixed it.', done: false },
    { kind: 'turn_end', end: { kind: 'completed' } },
  ]);
  // The completed message keeps the streamed text when the client sends none.
  const done = transcript.apply(
    event({
      kind: 'item',
      phase: 'completed',
      item: { id: 'm1', kind: 'agent_message', read: { as: 'reply', text: '' } },
    }),
  );
  assert.deepEqual(done.entries[3], { kind: 'message', item: 'm1', text: 'Fixed it.', done: true });
});

test('a prompt shows once, whether the echo or the local copy comes first', () => {
  const echo = event({
    kind: 'item',
    phase: 'completed',
    item: { id: 'u1', kind: 'user_message', read: { as: 'prompt', text: 'go' } },
  });
  const ended = event({ kind: 'turn_ended', turn: 't1', end: { kind: 'completed' } });
  assert.deepEqual(new Transcript().apply(echo).prompted('go').entries, [
    { kind: 'prompt', text: 'go' },
  ]);
  assert.deepEqual(new Transcript().prompted('go').apply(echo).entries, [
    { kind: 'prompt', text: 'go' },
  ]);
  // The same words in a later turn are a new prompt.
  const again = new Transcript().prompted('go').apply(ended).prompted('go');
  assert.equal(again.entries.filter((entry) => entry.kind === 'prompt').length, 2);
  assert.equal(again.ended('t1' as never), false);
  assert.equal(
    new Transcript()
      .prompted('go')
      .apply(ended)
      .ended('t1' as never),
    true,
  );
});

test('a transcript joined mid-turn learns the turn when it ends', () => {
  const transcript = new Transcript()
    .apply(event({ kind: 'delta', item: 'm1', text: 'Done.' }))
    .apply(event({ kind: 'turn_ended', turn: 'turn-9', end: { kind: 'completed' } }));
  assert.equal(transcript.turn, 'turn-9');
});

test('errors become notices, retried ones as warnings', () => {
  const transcript = new Transcript()
    .apply(event({ kind: 'error', message: 'stream lost', retry: 'will_retry' }))
    .apply(event({ kind: 'error', message: 'quota', retry: 'final' }));
  assert.deepEqual(transcript.entries, [
    { kind: 'notice', tone: 'warn', text: 'stream lost (retrying)' },
    { kind: 'notice', tone: 'danger', text: 'quota' },
  ]);
});

test('the sidebar groups runs by folder, waiting and live ones first', () => {
  const recorded: RecordedRun[] = [
    {
      id: other as never,
      harness: 'codex',
      workdir: '/work/web',
      started_at: 2000 as never,
      thread: null,
      session: null,
      ended_at: 3000 as never,
      end: 'stopped',
      exit: { kind: 'signal', value: 9 },
    },
    {
      id: run as never,
      harness: 'codex',
      workdir: '/work/acme',
      started_at: 1000 as never,
      thread: thread as never,
      session: null,
      ended_at: null,
      end: null,
      exit: null,
    },
  ];
  const live: RunSnapshot[] = [
    {
      run: run as never,
      harness: 'codex',
      workdir: '/work/acme',
      state: {
        phase: 'working',
        thread: thread as never,
        turn: null,
        approvals: [{ approval: 'a' as never, ask: { kind: 'file_change', reason: null } }],
      },
    },
  ];
  const projects = Desk.projects(live, recorded);
  assert.deepEqual(
    projects.map((project) => [project.name, project.waiting, project.runs.map((r) => r.run)]),
    [
      ['acme', 1, [run]],
      ['web', 0, [other]],
    ],
  );
  assert.deepEqual(Desk.state(projects[1]!.runs[0]!), {
    phase: 'ended',
    end: 'stopped',
    exit: { kind: 'signal', value: 9 },
  });
});

test('the client sends commands as JSON and reports refusals by their code', async () => {
  const sent: string[] = [];
  const client = new RunsClient((async (url: string, init?: RequestInit) => {
    sent.push(`${init?.method} ${url} ${init?.body ?? ''}`);
    if (url.endsWith('/stop')) return Response.json({ error: 'ended' }, { status: 409 });
    if (url === '/control/runs' && init?.method === 'POST') {
      return Response.json({ api_version: 1, run }, { status: 201 });
    }
    return Response.json({ api_version: 1 }, { status: 202 });
  }) as typeof fetch);
  assert.equal(await client.start('codex', '/work/acme'), run);
  assert.equal(await client.start('claude_code', '/work/web'), run);
  await client.answer(run as never, 'ap/1' as never, 'accept_for_session');
  await assert.rejects(client.stop(run as never), (error: unknown) => {
    assert.ok(error instanceof RunCommandError);
    assert.equal(error.code, 'ended');
    assert.equal(error.message, 'The run is stopping or has ended.');
    return true;
  });
  assert.deepEqual(sent, [
    'POST /control/runs {"harness":"codex","dir":"/work/acme"}',
    'POST /control/runs {"harness":"claude_code","dir":"/work/web"}',
    `POST /control/runs/${run}/approvals/ap%2F1 {"decision":"accept_for_session"}`,
    `POST /control/runs/${run}/stop {}`,
  ]);
});

test('a turn is matched to its record by id, or by its prompt where the ids differ', () => {
  const ended = (turn: string, prompt: string, known: string[] = []) =>
    new Transcript()
      .prompted(prompt)
      .apply(event({ kind: 'turn_started', turn }))
      .knowing(known)
      .apply(event({ kind: 'turn_ended', turn, end: { kind: 'completed' } }));
  const excerpt = (text: string, omitted_chars = 0) => ({ text, omitted_chars });
  // Codex: the live turn id is the recorded one.
  assert.equal(
    ended('t1', 'go').recordedIn([{ native_id: 't1', prompt: excerpt('something else') }]),
    true,
  );
  // Claude Code: a new recorded turn carrying the prompt.
  const claude = ended('live-1', 'fix the build', ['old']);
  assert.equal(claude.recordedIn([{ native_id: 'old', prompt: excerpt('fix the build') }]), false);
  assert.equal(
    claude.recordedIn([
      { native_id: 'old', prompt: excerpt('fix the build') },
      { native_id: 'new', prompt: excerpt('fix the', 6) },
    ]),
    true,
  );
  assert.equal(claude.recordedIn([{ native_id: 'new', prompt: excerpt('other') }]), false);
  // A page that joined mid-turn, after the prompt: any turn new since it read.
  const joined = new Transcript([], null, ['old']).apply(
    event({ kind: 'turn_ended', turn: 'live-2', end: { kind: 'completed' } }),
  );
  assert.equal(joined.recordedIn([{ native_id: 'old', prompt: excerpt('x') }]), false);
  assert.equal(joined.recordedIn([{ native_id: 'new', prompt: excerpt('x') }]), true);
  // Nothing read yet: only an id can match.
  const unread = new Transcript().apply(
    event({ kind: 'turn_ended', turn: 'live-3', end: { kind: 'completed' } }),
  );
  assert.equal(unread.recordedIn([{ native_id: 'new', prompt: excerpt('x') }]), false);
  // A turn still running is not replaced by its record.
  const running = new Transcript()
    .prompted('go')
    .apply(event({ kind: 'turn_started', turn: 't2' }));
  assert.equal(running.recordedIn([{ native_id: 't2', prompt: excerpt('go') }]), false);
});

test('an approval to use another tool says which, and only drivable clients resume', () => {
  assert.deepEqual(Runs.ask({ kind: 'tool', tool: 'WebFetch', detail: 'https://example.com' }), {
    action: 'Use WebFetch',
    detail: 'https://example.com',
    reason: null,
  });
  assert.equal(Runs.drivable('claude_code'), 'claude_code');
  assert.equal(Runs.drivable('codex'), 'codex');
  assert.equal(Runs.drivable('pi'), 'pi');
  assert.equal(Runs.drivable('opencode'), null);
});

test('projects narrow as a name or path is typed, and say their branch', () => {
  const projects = [
    { path: '/home/op/dev/acme', name: 'acme', git: { kind: 'branch', name: 'main' } },
    { path: '/home/op/dev/notes', name: 'notes', git: { kind: 'none' } },
    { path: '/home/op/ren', name: 'ren', git: { kind: 'detached', commit: 'abc1234' } },
  ] as const;
  assert.equal(Runs.choose([...projects], '').length, 3);
  assert.deepEqual(
    Runs.choose([...projects], 'ACME').map((project) => project.name),
    ['acme'],
  );
  assert.deepEqual(
    projects.map((project) => Runs.branch(project.git)),
    ['main', 'no git', 'at abc1234'],
  );
});

test('clients say whether a run can start with each, keeping newer statuses', async () => {
  const client = new RunsClient((async () =>
    Response.json({
      api_version: 1,
      clients: [
        { harness: 'codex', status: 'ready' },
        { harness: 'claude_code', status: 'not_installed' },
        { harness: 'pi', status: 'not_drivable' },
        { harness: 'opencode', status: 'paused' },
      ],
    })) as unknown as typeof fetch);
  const clients = await client.clients();
  assert.deepEqual(
    clients.map((entry) => Runs.unavailable(entry.status)),
    [null, 'not installed on the daemon’s host', 'Ren cannot drive it yet', 'paused'],
  );
});

test('projects are grouped by the folder they are in, and the newest runs name the recent ones', () => {
  const project = (path: string) => ({
    path,
    name: path.split('/').at(-1)!,
    git: { kind: 'none' as const },
  });
  const offered = [
    project('/srv/ren'),
    project('/dev/web'),
    project('/dev/api'),
    project('/dev/tools/cli'),
  ];
  assert.deepEqual(
    Runs.places(offered).map((place) => [place.parent, place.projects.map((p) => p.name)]),
    [
      ['/dev/', ['api', 'web']],
      ['/dev/tools/', ['cli']],
      ['/srv/', ['ren']],
    ],
  );
  const desk = (workdir: string, started_at: number) => ({
    run: run as never,
    harness: 'codex' as const,
    workdir,
    started_at: started_at as never,
    state: null,
    recorded: null,
  });
  const runs = [desk('/dev/web', 3), desk('/srv/ren', 5), desk('/dev/web', 9), desk('/gone', 10)];
  assert.deepEqual(
    Desk.recent(runs, offered).map((project) => project.name),
    ['web', 'ren'],
  );
});

test('a folder lists its runs that have not ended before those that have', () => {
  const live: RunSnapshot = {
    run: run as never,
    harness: 'codex',
    workdir: '/work/acme',
    state: { phase: 'idle', thread: thread as never },
  };
  const ended: RecordedRun = {
    id: other as never,
    harness: 'codex',
    workdir: '/work/acme',
    started_at: 1 as never,
    thread: null,
    session: null,
    ended_at: 2 as never,
    end: 'stopped',
    exit: null,
  };
  const [project] = Desk.projects([live], [ended]);
  const { open, ended: gone } = Desk.split(project!);
  assert.deepEqual(
    open.map((entry) => entry.run),
    [run],
  );
  assert.deepEqual(
    gone.map((entry) => entry.run),
    [other],
  );
});
