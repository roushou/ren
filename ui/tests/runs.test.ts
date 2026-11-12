import test from 'node:test';
import assert from 'node:assert/strict';
import {
  Desk,
  Memory,
  Models,
  Setup,
  RunCommandError,
  Runs,
  RunsClient,
  Transcript,
  runUpdate,
  type ModelId,
  type RunModel,
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
    { kind: 'prompt', item: 'u1', text: 'fix the failing test' },
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

test('a prompt shows once, whether the echo or the local copy comes first', () => {
  const echo = event({
    kind: 'item',
    phase: 'completed',
    item: { id: 'u1', kind: 'user_message', read: { as: 'prompt', text: 'go' } },
  });
  const ended = event({ kind: 'turn_ended', turn: 't1', end: { kind: 'completed' } });
  assert.deepEqual(new Transcript().apply(echo).prompted('go').entries, [
    { kind: 'prompt', item: 'u1', text: 'go' },
  ]);
  assert.deepEqual(new Transcript().prompted('go').apply(echo).entries, [
    { kind: 'prompt', item: 'u1', text: 'go' },
  ]);
  // The same words in a later turn are a new prompt.
  const again = new Transcript().prompted('go').apply(ended).prompted('go');
  assert.equal(again.entries.filter((entry) => entry.kind === 'prompt').length, 2);
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
      session: null,
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
      harness: 'codex',
      workdir: '/work/acme',
      state: {
        phase: 'working',
        thread: thread as never,
        turn: null,
        approvals: [{ approval: 'a' as never, ask: { kind: 'file_change', reason: null } }],
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
    if (url === '/control/runs' && init?.method === 'POST') {
      return Response.json({ api_version: 1, run }, { status: 201 });
    }
    return Response.json({ api_version: 1 }, { status: 202 });
  }) as typeof fetch);
  assert.equal(await client.start({ harness: 'codex', dir: '/work/acme' }), run);
  assert.equal(
    await client.start({
      harness: 'claude_code',
      dir: '/work/web',
      prompt: 'fix it',
      model: 'small' as ModelId,
    }),
    run,
  );
  await client.answer(run as never, 'ap/1' as never, 'accept_for_session');
  await assert.rejects(client.stop(run as never), (error: unknown) => {
    assert.ok(error instanceof RunCommandError);
    assert.equal(error.code, 'ended');
    assert.equal(error.message, 'The run is stopping or has ended.');
    return true;
  });
  assert.deepEqual(sent, [
    'POST /control/runs {"harness":"codex","dir":"/work/acme"}',
    'POST /control/runs {"harness":"claude_code","dir":"/work/web","prompt":"fix it","model":"small"}',
    `POST /control/runs/${run}/approvals/ap%2F1 {"decision":"accept_for_session"}`,
    `POST /control/runs/${run}/stop {}`,
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
  // Live, before the read lands: a prompt sent from here, its echo, and the
  // next reply streaming.
  const live = new Transcript()
    .prompted('and the docs')
    .apply(prompt('u2', 'and the docs'))
    .apply(event({ kind: 'delta', item: 'm2', text: 'Upd' }));
  const loaded = live.load({
    entries: [
      prompt('u1', 'fix the test'),
      reply('m1', long),
      event({ kind: 'turn_ended', turn: 't1', end: { kind: 'completed' } }),
      prompt('u2', 'and the docs'),
    ].map((event, index) => ({ run: run as never, seq: index + 1, event })),
    earlier: false,
  });
  assert.equal(loaded.read, true);
  assert.deepEqual(
    loaded.entries.map((entry) => entry.kind),
    ['prompt', 'message', 'turn_end', 'prompt', 'message'],
  );
  assert.deepEqual(loaded.entries[1], { kind: 'message', item: 'm1', text: long, done: true });
  // It goes on streaming and completes in place.
  const done = loaded
    .apply(event({ kind: 'delta', item: 'm2', text: 'ated.' }))
    .apply(reply('m2', 'Updated.'));
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
  assert.equal(Runs.drivable('opencode'), null);
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
        { harness: 'codex', status: 'ready', approvals: ['ask', 'edits', 'auto'] },
        { harness: 'claude_code', status: 'not_installed', approvals: ['ask', 'later'] },
        { harness: 'pi', status: 'not_drivable', approvals: [] },
        { harness: 'opencode', status: 'paused', approvals: [] },
      ],
    })) as unknown as typeof fetch);
  const clients = await client.clients();
  assert.deepEqual(
    clients.map((entry) => Runs.unavailable(entry.status)),
    [null, 'not installed on the daemon’s host', 'Ren cannot drive it yet', 'paused'],
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
    session: null,
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
              session: null,
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
  await client.edit({ kind: 'session', id: thread as never }, 'unarchive');
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
    `POST /control/history/sessions/${thread}/unarchive`,
    'GET /api/runs?limit=5&shelf=archived',
    `POST /control/history/runs/${run}/delete`,
  ]);
});

test('an archived run the daemon still lists as ended is left out, a running one is not', () => {
  const ended: RunSnapshot = {
    run: run as never,
    harness: 'codex',
    workdir: '/work/acme',
    state: { phase: 'ended', end: 'stopped', exit: null },
    settings: { model: null, effort: null, approvals: null },
  };
  const running: RunSnapshot = {
    run: other as never,
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

test('the remembered settings read back from their cookie, and junk is the default', () => {
  const remembered = {
    harness: 'claude_code' as const,
    settings: { claude_code: { model: 'opus', modelName: 'Opus', effort: 'high' } },
  };
  const cookie = Memory.cookie(remembered);
  assert.match(cookie, /^ren_runs=[^;]+; Path=\/runs; Max-Age=31536000; SameSite=Lax$/);
  const value = decodeURIComponent(cookie.slice('ren_runs='.length, cookie.indexOf(';')));
  assert.deepEqual(Memory.parse(value), {
    harness: 'claude_code',
    settings: {
      claude_code: { model: 'opus', modelName: 'Opus', effort: 'high', approvals: undefined },
    },
  });
  for (const junk of [null, '', '{', '[]', '{"harness":"gpt"}']) {
    assert.equal(Memory.parse(junk).harness, 'codex', String(junk));
  }
  // A client this UI does not drive, or a value that is not text, is left out.
  assert.deepEqual(
    Memory.parse('{"harness":"pi","settings":{"opencode":{"model":"x"},"pi":{"model":7}}}'),
    {
      harness: 'pi',
      settings: {
        pi: { model: undefined, modelName: undefined, effort: undefined, approvals: undefined },
      },
    },
  );
});

test('the page’s opening reads each stand on their own', async () => {
  const client = new RunsClient((async (url: string) => {
    if (url === '/control/runs/clients') return Response.json({ error: 'busy' }, { status: 503 });
    if (url === '/control/runs') return Response.json({ api_version: 1, runs: [] });
    if (url === '/control/runs/projects') {
      return Response.json({ api_version: 1, projects: [], scratch: '/s' });
    }
    if (url === '/control/runs/clients/codex/models') {
      return Response.json({ api_version: 1, harness: 'codex', models: [] });
    }
    return Response.json({ api_version: 1, runs: [] });
  }) as typeof fetch);
  const opening = await client.opening('codex');
  assert.deepEqual(opening.live, { state: 'read', value: [] });
  assert.deepEqual(opening.places, { state: 'read', value: { projects: [], scratch: '/s' } });
  assert.equal(opening.clients.state, 'failed');
  assert.equal(opening.recorded.state, 'read');
  // The opening client's models come with them.
  assert.deepEqual(opening.models, { harness: 'codex', read: { state: 'read', value: [] } });
  assert.equal((await client.opening()).models, null);
});
