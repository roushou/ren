import test from 'node:test';
import assert from 'node:assert/strict';
import {
  decisions,
  Desk,
  Memory,
  Compose,
  Models,
  Setup,
  RunCommandError,
  Runs,
  RunsClient,
  Transcript,
  runUpdate,
  type ModelId,
  type RunModel,
  type Effort,
  type DriveEvent,
  type RecordedRun,
  type RunSnapshot,
} from '../src/lib/runs.ts';
import { Steps } from '../src/lib/steps.ts';

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
        queued: null,
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
      queued: null,
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
    interrupt: 'none',
    stop: true,
    approvals: [],
  });
  const working = Runs.controls({
    phase: 'working',
    thread: thread as never,
    turn: null,
    approvals: [],
    queued: null,
  });
  assert.equal(working.prompt, false);
  assert.equal(working.interrupt, 'pending', 'no turn to interrupt until the agent names it');
  const named = Runs.controls({
    phase: 'working',
    thread: thread as never,
    turn: 'turn-1' as never,
    approvals: [],
    queued: null,
  });
  assert.equal(named.interrupt, 'ready', 'a named turn can be interrupted');
  const ended = Runs.controls({ phase: 'ended', end: 'exited', exit: { kind: 'code', value: 1 } });
  assert.deepEqual(ended, { prompt: false, interrupt: 'none', stop: false, approvals: [] });
  assert.deepEqual(
    Runs.mark({
      phase: 'working',
      thread: thread as never,
      turn: null,
      approvals: [{ approval: 'a' as never, ask: { kind: 'file_change', reason: null } }],
      queued: null,
    }),
    {
      label: 'Needs you',
      mark: 'yours',
    },
  );
});

test('the transcript streams a reply, then commands and file changes as items update', () => {
  let transcript = new Transcript();
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
    { kind: 'prompt', item: 'u1', text: 'fix the failing test', images: [] },
    {
      kind: 'command',
      item: 'c1',
      command: 'cargo test',
      intent: { said: 'unsaid' },
      status: 'completed',
      output: 'ok',
      exit: 0,
    },
    { kind: 'file_change', item: 'f1', paths: ['src/lib.rs'], status: 'completed' },
    { kind: 'message', item: 'm1', text: 'Fixed it.', done: false },
    { kind: 'turn_end', turn: 'turn-1', end: { kind: 'completed' } },
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

test('items of two processes keep apart though a client names them alike, stored or live', () => {
  const first = '0190f3a0-0000-7000-8000-0000000000a1' as never;
  const second = '0190f3a0-0000-7000-8000-0000000000a2' as never;
  const prompt = (text: string) =>
    event({
      kind: 'item',
      phase: 'completed',
      item: { id: 'user-1', kind: 'user_message', read: { as: 'prompt', text } },
    });
  const reply = (text: string) =>
    event({
      kind: 'item',
      phase: 'completed',
      item: { id: 'msg-1', kind: 'agent_message', read: { as: 'reply', text } },
    });
  // A session's second process names its first items as the first did.
  const stored = new Transcript().load({
    entries: [
      {
        run: first,
        seq: 1,
        at: 0 as never,
        started_at: 0 as never,
        event: prompt('explain the parser'),
      },
      {
        run: first,
        seq: 2,
        at: 0 as never,
        started_at: 0 as never,
        event: reply('It reads tokens.'),
      },
      {
        run: second,
        seq: 1,
        at: 0 as never,
        started_at: 0 as never,
        event: prompt('and the lexer?'),
      },
    ],
    earlier: false,
  });
  const live = stored.apply(reply('It splits words.'), second);
  const shown = (transcript: Transcript) =>
    transcript.entries.map((entry) =>
      entry.kind === 'prompt' || entry.kind === 'message' ? entry.text : entry.kind,
    );
  assert.deepEqual(shown(live), [
    'explain the parser',
    'It reads tokens.',
    'and the lexer?',
    'It splits words.',
  ]);
  // The same item reported again by its own process is still one entry.
  assert.deepEqual(shown(live.apply(reply('It splits words, then tokens.'), second)), [
    'explain the parser',
    'It reads tokens.',
    'and the lexer?',
    'It splits words, then tokens.',
  ]);
});

test('a tool keeps what it does as Ren read it, and an older one reads with none', () => {
  const read = event({
    kind: 'item',
    phase: 'started',
    item: {
      id: 't1',
      kind: 'Read',
      read: {
        as: 'tool',
        name: 'Read',
        effects: [{ kind: 'read_file', target: '/work/acme/a.rs' }],
        status: 'in_progress',
      },
    },
  });
  const older = event({
    kind: 'item',
    phase: 'completed',
    item: {
      id: 't2',
      kind: 'Grep',
      read: { as: 'tool', name: 'Grep TODO', status: 'completed' },
    },
  });
  assert.deepEqual(new Transcript().apply(read).apply(older).entries, [
    {
      kind: 'tool',
      item: 't1',
      name: 'Read',
      effects: [{ kind: 'read_file', target: '/work/acme/a.rs' }],
      done: false,
    },
    { kind: 'tool', item: 't2', name: 'Grep TODO', effects: [], done: true },
  ]);
});

test('an echoed prompt keeps what each image sent with it is', () => {
  const image = {
    name: 'shot.png',
    kind: { kind: 'image', format: 'png', dimensions: { width: 4, height: 3 } },
    size: 9,
    sha256: 'ab',
  };
  const echo = event({
    kind: 'item',
    phase: 'completed',
    item: { id: 'u2', kind: 'user_message', read: { as: 'prompt', text: '', images: [image] } },
  });
  assert.deepEqual(new Transcript().apply(echo).entries, [
    {
      kind: 'prompt',
      item: 'u2',
      text: '',
      images: [{ name: 'shot.png', format: 'png', width: 4, height: 3, size: 9, sha256: 'ab' }],
    },
  ]);
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
      recording: null,
      ended_at: 3000 as never,
      end: 'stopped',
      exit: { kind: 'signal', value: 9 },
      archived_at: null,
      model: null,
      effort: null,
      approvals: null,
    },
    {
      id: run as never,
      harness: 'codex',
      workdir: '/work/acme',
      started_at: 1000 as never,
      thread: thread as never,
      recording: null,
      ended_at: null,
      end: null,
      exit: null,
      archived_at: null,
      model: null,
      effort: null,
      approvals: null,
    },
  ];
  const live: RunSnapshot[] = [
    {
      run: run as never,
      session: 'session' as never,
      harness: 'codex',
      workdir: '/work/acme',
      state: {
        phase: 'working',
        thread: thread as never,
        turn: null,
        approvals: [{ approval: 'a' as never, ask: { kind: 'file_change', reason: null } }],
        queued: null,
      },
      settings: { model: null, effort: null, approvals: null },
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
    if (url === '/control/sessions' && init?.method === 'POST') {
      return Response.json({ api_version: 1, session: run }, { status: 201 });
    }
    return Response.json({ api_version: 1 }, { status: 202 });
  }) as typeof fetch);
  assert.equal(await client.createSession('codex', '/work/acme'), run);
  assert.equal(
    await client.createSession('claude_code', '/work/web', { model: 'small' as ModelId }),
    run,
  );
  await client.toSession(run as never, {
    kind: 'answer',
    approval: 'ap/1' as never,
    decision: 'accept_for_session',
  });
  await assert.rejects(client.toSession(run as never, { kind: 'stop' }), (error: unknown) => {
    assert.ok(error instanceof RunCommandError);
    assert.equal(error.code, 'ended');
    assert.equal(error.message, 'The run is stopping or has ended.');
    return true;
  });
  assert.deepEqual(sent, [
    'POST /control/sessions {"harness":"codex","dir":"/work/acme"}',
    'POST /control/sessions {"harness":"claude_code","dir":"/work/web","model":"small"}',
    `POST /control/sessions/${run}/approvals/ap%2F1 {"decision":"accept_for_session"}`,
    `POST /control/sessions/${run}/stop {}`,
  ]);
});

test('the stored conversation and the live feed merge into one, nothing cut', () => {
  const long = 'a very long reply '.repeat(500);
  const reply = (id: string, text: string) =>
    event({
      kind: 'item',
      phase: 'completed',
      item: { id, kind: 'agent_message', read: { as: 'reply', text } },
    });
  const prompt = (id: string, text: string) =>
    event({
      kind: 'item',
      phase: 'completed',
      item: { id, kind: 'user_message', read: { as: 'prompt', text } },
    });
  // Live, before the read lands: a prompt's echo, and the next reply
  // streaming.
  const live = new Transcript()
    .apply(prompt('u2', 'and the docs'), run as never)
    .apply(event({ kind: 'delta', item: 'm2', text: 'Upd' }), run as never);
  const loaded = live.load({
    entries: [
      prompt('u1', 'fix the test'),
      reply('m1', long),
      event({ kind: 'turn_ended', turn: 't1', end: { kind: 'completed' } }),
      prompt('u2', 'and the docs'),
    ].map((event, index) => ({
      run: run as never,
      seq: index + 1,
      at: 0 as never,
      started_at: 0 as never,
      event,
    })),
    earlier: false,
  });
  assert.equal(loaded.read, true);
  assert.deepEqual(
    loaded.entries.map((entry) => entry.kind),
    ['prompt', 'message', 'turn_end', 'prompt', 'message'],
  );
  assert.deepEqual(loaded.entries[1], {
    kind: 'message',
    item: `${run}/m1`,
    text: long,
    done: true,
  });
  // It goes on streaming and completes in place.
  const done = loaded
    .apply(event({ kind: 'delta', item: 'm2', text: 'ated.' }), run as never)
    .apply(reply('m2', 'Updated.'), run as never);
  assert.equal(done.entries.length, 5);
  assert.equal(done.firstPrompt(), 'fix the test');
  // Turns start at their prompts, all of them kept.
  assert.deepEqual(
    Runs.turns(done.entries).map((turn) => turn.length),
    [3, 2],
  );
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
  assert.equal(Runs.drivable('opencode'), 'opencode');
  assert.equal(Runs.drivable('gpt' as never), null);
});

test('projects narrow as a name or path is typed, and say their branch', () => {
  const projects = [
    {
      path: '/home/op/dev/acme',
      name: 'acme',
      shown: '~/dev/acme',
      kind: 'project',
      git: { kind: 'branch', name: 'main' },
    },
    {
      path: '/home/op/dev/notes',
      name: 'notes',
      shown: '~/dev/notes',
      kind: 'project',
      git: { kind: 'none' },
    },
    {
      path: '/home/op/ren',
      name: 'ren',
      shown: '~/ren',
      kind: 'project',
      git: { kind: 'detached', commit: 'abc1234' },
    },
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
        {
          harness: 'codex',
          status: 'ready',
          approvals: ['ask', 'edits', 'auto'],
          midway: ['steer'],
        },
        {
          harness: 'claude_code',
          status: 'not_installed',
          approvals: ['ask', 'later'],
          midway: ['steer'],
        },
        { harness: 'pi', status: 'not_drivable', approvals: [], midway: ['steer'] },
        { harness: 'opencode', status: 'paused', approvals: [], midway: ['steer'] },
      ],
    })) as unknown as typeof fetch);
  const clients = await client.clients();
  assert.deepEqual(
    clients.map((entry) => Runs.unavailable(entry.status)),
    [null, 'not installed on the daemon’s host', 'not_drivable', 'paused'],
  );
  // A policy a newer daemon names is kept as it was sent.
  assert.deepEqual(clients[1]?.approvals, ['ask', 'later']);
});

test('projects are grouped by the folder they are in, and the newest runs name the recent ones', () => {
  const project = (path: string) => ({
    path,
    name: path.split('/').at(-1)!,
    shown: path.replace(/^\/home\/op/, '~'),
    kind: 'project' as const,
    git: { kind: 'none' as const },
  });
  const offered = [
    project('/srv/ren'),
    project('/home/op/dev/web'),
    project('/home/op/dev/api'),
    project('/home/op/dev/tools/cli'),
  ];
  // Grouped by the folder they are in, as the operator reads it.
  assert.deepEqual(
    Runs.places(offered).map((place) => [place.parent, place.projects.map((p) => p.name)]),
    [
      ['/srv', ['ren']],
      ['~/dev', ['api', 'web']],
      ['~/dev/tools', ['cli']],
    ],
  );
  const desk = (workdir: string, started_at: number) => ({
    run: run as never,
    harness: 'codex' as const,
    workdir,
    started_at: started_at as never,
    state: null,
    recorded: null,
    settings: { model: null, effort: null, approvals: null },
  });
  const runs = [
    desk('/home/op/dev/web', 3),
    desk('/srv/ren', 5),
    desk('/home/op/dev/web', 9),
    desk('/gone', 10),
  ];
  assert.deepEqual(
    Desk.recent(runs, offered).map((project) => project.name),
    ['web', 'ren'],
  );
});

test('a folder lists its runs that have not ended before those that have', () => {
  const live: RunSnapshot = {
    run: run as never,
    session: 'session' as never,
    harness: 'codex',
    workdir: '/work/acme',
    state: { phase: 'idle', thread: thread as never },
    settings: { model: null, effort: null, approvals: null },
  };
  const ended: RecordedRun = {
    id: other as never,
    harness: 'codex',
    workdir: '/work/acme',
    started_at: 1 as never,
    thread: null,
    recording: null,
    ended_at: 2 as never,
    end: 'stopped',
    exit: null,
    archived_at: null,
    model: null,
    effort: null,
    approvals: null,
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

test('history edits and the archived shelf map to their routes and words', async () => {
  const seen: string[] = [];
  const client = new RunsClient((async (url: string, init?: RequestInit) => {
    seen.push(`${init?.method} ${url}`);
    if (url.includes('/delete')) return Response.json({ error: 'run_running' }, { status: 409 });
    return url.startsWith('/api/runs')
      ? Response.json({
          api_version: 1,
          runs: [
            {
              id: other,
              harness: 'codex',
              workdir: '/work/acme',
              started_at: 1,
              thread: null,
              recording: null,
              ended_at: 2,
              end: 'stopped',
              exit: null,
              archived_at: 3,
              model: 'gpt-deep',
              effort: 'high',
              approvals: null,
            },
          ],
        })
      : Response.json({ api_version: 1, change: 'archive' }, { status: 202 });
  }) as unknown as typeof fetch);
  await client.edit({ kind: 'run', id: run as never }, 'archive');
  await client.edit({ kind: 'recording', id: thread as never }, 'unarchive');
  const [shelved] = await client.recorded(5, 'archived');
  assert.equal(shelved?.archived_at, 3);
  assert.deepEqual(
    [shelved?.model, shelved?.effort, shelved?.approvals],
    ['gpt-deep', 'high', null],
  );
  await assert.rejects(
    client.edit({ kind: 'run', id: run as never }, 'delete'),
    (error: unknown) =>
      error instanceof RunCommandError && error.message.includes('stop it before deleting'),
  );
  assert.deepEqual(seen, [
    `POST /control/history/runs/${run}/archive`,
    `POST /control/history/recordings/${thread}/unarchive`,
    'GET /api/runs?limit=5&shelf=archived',
    `POST /control/history/runs/${run}/delete`,
  ]);
});

test('an archived run the daemon still lists as ended is left out, a running one is not', () => {
  const ended: RunSnapshot = {
    run: run as never,
    session: 'session' as never,
    harness: 'codex',
    workdir: '/work/acme',
    state: { phase: 'ended', end: 'stopped', exit: null },
    settings: { model: null, effort: null, approvals: null },
  };
  const running: RunSnapshot = {
    run: other as never,
    session: 'session' as never,
    harness: 'codex',
    workdir: '/work/acme',
    state: { phase: 'idle', thread: thread as never },
    settings: { model: null, effort: null, approvals: null },
  };
  const hidden = new Set([run, other] as never[]);
  const [project] = Desk.projects([ended, running], [], hidden);
  assert.deepEqual(
    project?.runs.map((entry) => entry.run),
    [other],
  );
});

test('a client’s models read as listed, grouped by provider, the default known', async () => {
  const client = new RunsClient((async (url: string) => {
    assert.equal(url, '/control/runs/clients/pi/models');
    return Response.json({
      api_version: 1,
      harness: 'pi',
      models: [
        {
          id: 'anthropic/big',
          name: 'Big',
          description: null,
          provider: 'anthropic',
          choice: 'other',
          efforts: ['low', 'medium', 'high'],
          effort: 'medium',
        },
        {
          id: 'openai/fast',
          name: 'Fast',
          description: 'Quick.',
          provider: 'openai',
          choice: 'default',
          efforts: [],
          effort: null,
        },
        {
          id: 'anthropic/small',
          name: 'Small',
          description: null,
          provider: 'anthropic',
          choice: 'later',
          efforts: [],
          effort: null,
        },
      ],
    });
  }) as typeof fetch);
  const models: RunModel[] = await client.models('pi');
  assert.deepEqual(
    Models.groups(models).map((group) => [group.provider, group.models.map((model) => model.id)]),
    [
      ['anthropic', ['anthropic/big', 'anthropic/small']],
      ['openai', ['openai/fast']],
    ],
  );
  assert.equal(Models.fallback(models)?.id, 'openai/fast');
  // An unknown choice label is kept, not taken for the default.
  assert.equal(models[2]!.choice, 'later');
  assert.equal(Models.keep(models, 'anthropic/big'), 'anthropic/big');
  assert.equal(Models.keep(models, 'gone/model'), null);
  assert.equal(Models.keep(models, null), null);
  // The efforts a choice takes: the default's are its model's.
  assert.deepEqual(Setup.efforts(models, 'anthropic/big' as ModelId)?.efforts, [
    'low',
    'medium',
    'high',
  ]);
  assert.equal(Setup.efforts(models, null)?.id, 'openai/fast');
  assert.deepEqual(
    Setup.words({ model: 'openai/fast' as ModelId, effort: 'high', approvals: 'edits' }),
    ['openai/fast', 'high effort', 'edits alone'],
  );
  assert.deepEqual(Setup.words({ model: null, effort: null, approvals: null }), []);
  // The client's own choice names what it is, when the client says.
  assert.equal(Setup.own('Opus 5.5'), 'Opus 5.5 (default)');
  // Starting again, a label this UI does not know is left to the client.
  assert.deepEqual(Setup.known({ model: null, effort: 'ludicrous' as never, approvals: 'auto' }), {
    model: null,
    effort: null,
    approvals: 'auto',
  });
});

test('scratch runs share one group, whatever folder each got', () => {
  const live = (id: string, workdir: string): RunSnapshot => ({
    run: id as never,
    session: 'session' as never,
    harness: 'codex',
    workdir,
    state: { phase: 'idle', thread: thread as never },
    settings: { model: null, effort: null, approvals: null },
  });
  const scratch = '/home/op/.local/share/ren/scratch';
  const projects = Desk.projects(
    [
      live(run, `${scratch}/scratch-1a2b3c4d`),
      live(other, `${scratch}/scratch-5e6f7a8b`),
      live('0190f3a0-0000-7000-8000-0000000000dd', '/home/op/dev/acme'),
    ],
    [],
    new Set(),
    scratch,
  );
  assert.deepEqual(projects.map((project) => [project.name, project.runs.length]).sort(), [
    ['Scratch', 2],
    ['acme', 1],
  ]);
});

test('a written path reads from inside its session’s folder', () => {
  assert.equal(Runs.within('/work/acme/src/lib.rs', '/work/acme'), 'src/lib.rs');
  assert.equal(Runs.within('/work/acme/src/lib.rs', '/work/acme/'), 'src/lib.rs');
  // Outside the folder, or a sibling sharing its prefix, stays whole.
  assert.equal(Runs.within('/work/acme-api/x.rs', '/work/acme'), '/work/acme-api/x.rs');
  assert.equal(Runs.within('/etc/hosts', '/work/acme'), '/etc/hosts');
  assert.equal(Runs.within('src/lib.rs', null), 'src/lib.rs');
});

test('the remembered settings read back from their cookie, and junk is the default', () => {
  const remembered = {
    harness: 'claude_code' as const,
    settings: { claude_code: { model: 'opus', effort: 'high' } },
  };
  const cookie = Memory.cookie(remembered);
  // The Sessions page's own path, so the server reads it for that page.
  assert.match(cookie, /^ren_runs=[^;]+; Path=\/sessions; Max-Age=31536000; SameSite=Lax$/);
  const value = decodeURIComponent(cookie.slice('ren_runs='.length, cookie.indexOf(';')));
  assert.deepEqual(Memory.parse(value), {
    harness: 'claude_code',
    settings: {
      claude_code: { model: 'opus', effort: 'high', approvals: undefined },
    },
  });
  for (const junk of [null, '', '{', '[]', '{"harness":"gpt"}']) {
    assert.equal(Memory.parse(junk).harness, 'claude_code', String(junk));
  }
  // A client this UI does not drive, or a value that is not text, is left out.
  assert.deepEqual(
    Memory.parse('{"harness":"pi","settings":{"gpt":{"model":"x"},"pi":{"model":7}}}'),
    {
      harness: 'pi',
      settings: {
        pi: { model: undefined, effort: undefined, approvals: undefined },
      },
    },
  );
});

test('a new session starts with the settings kept for its client, where they still apply', () => {
  const listed = (id: string, efforts: Effort[], choice = 'other'): RunModel => ({
    id: id as ModelId,
    name: id,
    description: null,
    provider: null,
    choice: choice as RunModel['choice'],
    efforts,
    effort: null,
  });
  const models = [listed('fast', [], 'default'), listed('deep', ['low', 'high'])];
  let remembered = Memory.chose(Memory.NONE, 'claude_code');
  remembered = Memory.set(remembered, 'claude_code', {
    model: 'deep' as ModelId,
    effort: 'high',
    approvals: 'ask',
  });
  assert.equal(remembered.harness, 'claude_code');
  assert.deepEqual(Memory.settings(remembered, 'claude_code', models, ['ask', 'auto']), {
    model: 'deep',
    effort: 'high',
    approvals: 'ask',
  });
  // A model no longer listed is the client's own, and so is an effort the
  // default does not take; a policy the client does not run under is dropped.
  assert.deepEqual(Memory.settings(remembered, 'claude_code', [models[0]!], []), {
    model: null,
    effort: null,
    approvals: null,
  });
  // Another client's settings are its own.
  assert.deepEqual(Memory.settings(remembered, 'codex', models, ['ask']), {
    model: null,
    effort: null,
    approvals: null,
  });
});

test('the page’s opening reads each stand on their own', async () => {
  const client = new RunsClient((async (url: string) => {
    if (url === '/control/runs/clients') return Response.json({ error: 'busy' }, { status: 503 });
    if (url === '/control/runs') return Response.json({ api_version: 1, runs: [] });
    if (url === '/control/runs/places') {
      return Response.json({ api_version: 1, places: [], scratch: '/s' });
    }
    if (url === '/control/runs/clients/codex/models') {
      return Response.json({ api_version: 1, harness: 'codex', models: [] });
    }
    return Response.json({ api_version: 1, runs: [] });
  }) as typeof fetch);
  const opening = await client.opening();
  assert.deepEqual(opening.live, { state: 'read', value: [] });
  assert.deepEqual(opening.places, { state: 'read', value: { places: [], scratch: '/s' } });
  assert.equal(opening.clients.state, 'failed');
  assert.equal(opening.recorded.state, 'read');
});

test('a run’s state marks it: only an approval waits on the operator', () => {
  const mark = (state: unknown) => Runs.mark(state as never).mark;
  assert.equal(mark({ phase: 'starting' }), 'waiting');
  assert.equal(mark({ phase: 'idle', thread: null, turn: null, queued: null }), 'idle');
  assert.equal(
    mark({ phase: 'working', thread: null, turn: null, approvals: [], queued: null }),
    'responding',
  );
  assert.equal(Runs.endMark('stopped'), 'idle');
  assert.equal(Runs.endMark('exited', { kind: 'code', value: 0 }), 'idle');
  assert.equal(Runs.endMark('exited', { kind: 'code', value: 2 }), 'warning');
  assert.equal(Runs.endMark('recovered'), 'warning');
  assert.equal(Runs.endMark('spawn_failed'), 'failed');
  assert.equal(mark({ phase: 'unrecognized', text: 'paused' }), 'unknown');
});

test('how a run ended reads in words; the process’s exit is a detail', () => {
  const ended = (end: string, exit: unknown = null) =>
    Runs.mark({ phase: 'ended', end: end as never, exit: exit as never }).label;
  assert.equal(ended('drained', { kind: 'signal', value: 9 }), 'Stopped when Ren shut down');
  assert.equal(ended('recovered'), 'Lost when Ren restarted');
  assert.equal(ended('stopped'), 'Stopped');
  assert.equal(ended('spawn_failed'), 'Couldn’t start');
  assert.equal(ended('exited', { kind: 'code', value: 0 }), 'Ended');
  assert.equal(ended('exited', { kind: 'code', value: 2 }), 'Ended with an error');
  assert.equal(ended('exited', { kind: 'signal', value: 11 }), 'Killed');
  assert.equal(Runs.exitDetail({ kind: 'signal', value: 9 }), 'killed by SIGKILL (signal 9)');
  assert.equal(Runs.exitDetail({ kind: 'signal', value: 40 }), 'killed by signal 40');
  assert.equal(Runs.exitDetail({ kind: 'code', value: 1 }), 'exit code 1');
  assert.equal(Runs.exitDetail(null), null);
});

test('where a run works reads as Scratch or under the home as ~/', () => {
  const offered = [
    {
      path: '/home/op/dev/acme',
      name: 'acme',
      shown: '~/dev/acme',
      kind: 'project' as const,
      git: { kind: 'none' as const },
    },
  ];
  const scratch = '/home/op/.local/share/ren/scratch';
  assert.equal(Runs.place(`${scratch}/scratch-1a2b`, scratch, offered), 'Scratch');
  assert.equal(Runs.place(scratch, scratch, offered), 'Scratch');
  assert.equal(Runs.place('/home/op/dev/acme', scratch, offered), '~/dev/acme');
  assert.equal(Runs.place('/home/op/notes/drafts', scratch, offered), '~/notes/drafts');
  assert.equal(Runs.place('/srv/ren', scratch, offered), '/srv/ren');
  assert.equal(Runs.place('/home/op/dev/acme', null, []), '/home/op/dev/acme');
});

test('the composer starts, queues or steers, or continues, by how the run stands', () => {
  const working = {
    phase: 'working' as const,
    thread: thread as never,
    turn: null,
    approvals: [],
    queued: 'then the docs',
  };
  assert.deepEqual(Compose.mode({ phase: 'idle', thread: thread as never }, [], false), {
    kind: 'start',
  });
  assert.deepEqual(Compose.mode(working, ['steer', 'model'], false), {
    kind: 'during',
    steer: true,
    queued: 'then the docs',
  });
  assert.deepEqual(Compose.mode(working, ['model'], false), {
    kind: 'during',
    steer: false,
    queued: 'then the docs',
  });
  const ended = { phase: 'ended' as const, end: 'stopped' as const, exit: null };
  assert.deepEqual(Compose.mode(ended, [], true), { kind: 'continue' });
  assert.equal(Compose.mode(ended, [], false).kind, 'closed');
  assert.equal(Compose.mode({ phase: 'starting' }, [], false).kind, 'closed');
});

test('↑ recalls the run’s earlier prompts, newest first, and ↓ comes back', () => {
  const prompts = ['first', 'second', 'third'];
  let at: number | null = null;
  const step = (older: boolean) => {
    const next = Compose.recall(prompts, at, older);
    if (next) at = next.at;
    return next?.text ?? null;
  };
  assert.equal(step(true), 'third');
  assert.equal(step(true), 'second');
  assert.equal(step(true), 'first');
  assert.equal(step(true), null, 'nothing older');
  assert.equal(step(false), 'second');
  assert.equal(step(false), 'third');
  assert.equal(step(false), '', 'back past the newest: the draft again');
  assert.equal(at, null);
  assert.equal(Compose.recall([], null, true), null);
});

test('a prompt goes with how to deliver it; the queue and settings have their own routes', async () => {
  const sent: string[] = [];
  const client = new RunsClient((async (url: string, init?: RequestInit) => {
    sent.push(`${url} ${init?.body ?? ''}`);
    return Response.json({ api_version: 1 }, { status: 202 });
  }) as typeof fetch);
  await client.toSession(run as never, { kind: 'prompt', text: 'go', delivery: 'steer' });
  await client.toSession(run as never, { kind: 'unqueue' });
  await client.toSession(run as never, {
    kind: 'settings',
    model: null,
    effort: 'high',
    approvals: null,
  });
  assert.deepEqual(sent, [
    `/control/sessions/${run}/prompt {"text":"go","delivery":"steer"}`,
    `/control/sessions/${run}/unqueue {}`,
    `/control/sessions/${run}/settings {"model":null,"effort":"high","approvals":null}`,
  ]);
});

test('every decision has distinct words, and only allowing reads as passed', () => {
  const words = decisions.map((decision) => Runs.decision(decision));
  assert.equal(new Set(words.map((said) => said.label)).size, decisions.length);
  assert.equal(new Set(words.map((said) => said.done)).size, decisions.length);
  assert.deepEqual(
    words.map((said) => said.mark),
    ['passed', 'passed', 'idle', 'idle'],
  );
});

test('a stored item keeps when it ran; a step reads how long, and a group adds it up', () => {
  const run = 'run-1' as never;
  const command = (id: string, phase: 'started' | 'completed') =>
    ({
      kind: 'item',
      phase,
      item: {
        id,
        kind: 'command_execution',
        read: {
          as: 'command',
          command: 'cargo test',
          intent: { said: 'unsaid' },
          status: phase === 'completed' ? 'completed' : 'in_progress',
          output: null,
          exit_code: phase === 'completed' ? 0 : null,
        },
      },
    }) as never;
  const transcript = new Transcript().load({
    entries: [
      {
        run,
        seq: 1,
        at: 1_000 as never,
        started_at: 1_000 as never,
        event: command('c1', 'started'),
      },
      {
        run,
        seq: 2,
        at: 3_500 as never,
        started_at: 1_000 as never,
        event: command('c1', 'completed'),
      },
      {
        run,
        seq: 3,
        at: 4_000 as never,
        started_at: 4_000 as never,
        event: command('c2', 'started'),
      },
    ],
    earlier: false,
  });
  assert.deepEqual([...transcript.spans], [['run-1/c1', { from: 1_000, to: 3_500 }]]);
  // The time survives the live feed's next event.
  const later = transcript.apply(command('c3', 'started'), run);
  assert.deepEqual(later.spans.get('run-1/c1' as never), { from: 1_000, to: 3_500 });
  const [group] = Steps.blocks(later.entries, null, later.spans);
  assert.equal(group?.kind, 'steps');
  if (group?.kind !== 'steps') return;
  assert.deepEqual(
    group.steps.map((step) => step.took),
    [2_500, null, null],
  );
  assert.equal(Steps.took(group.steps), 2_500);
  assert.equal(Steps.took([]), null);
  // The window the picked steps ran over, from what is known of them.
  assert.deepEqual(Steps.window(group.steps), { from: 1_000, to: 3_500 });
  assert.equal(Steps.window(group.steps.slice(1)), null);
});

test('an approval asks its question by its client and what it asks for', () => {
  assert.equal(
    Runs.question({ kind: 'command', command: 'git push', cwd: null, reason: null }, 'Codex'),
    'Allow Codex to run this command?',
  );
  assert.equal(
    Runs.question({ kind: 'tool', tool: 'WebFetch', detail: null }, 'Claude Code'),
    'Allow Claude Code to use WebFetch?',
  );
  assert.deepEqual(
    decisions.map((decision) => Runs.decision(decision).label),
    ['Allow once', 'Allow for this session', 'Decline', 'Decline and stop'],
  );
});
