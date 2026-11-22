import test from 'node:test';
import assert from 'node:assert/strict';
import {
  ChangeKinds,
  GenerationStates,
  InstalledPackages,
  Origins,
  PackageHistory,
  Platforms,
  RowActions,
  Standings,
  Supplies,
  GenerationChanges,
  PackageFilters,
  PackageGroups,
  PackagePlan,
  PackagesApi,
  PackagesError,
  type PackageListing,
} from '../src/lib/packages.ts';

/** A listing as the daemon answers it: invented paths and versions. */
const wire = {
  api_version: 1,
  platform: 'linux_x64',
  generation: 3,
  bin: '/data/tools/generations/3/bin',
  packages: [
    {
      id: 'claude',
      origin: 'builtin',
      kind: 'client',
      program: 'claude',
      release: null,
      installed: null,
      system: { path: '/opt/acme/bin/claude', version: '2.1.0', unread: null },
      needed_by: ['claude_code'],
    },
    {
      id: 'node',
      origin: 'builtin',
      kind: 'runtime',
      program: 'node',
      release: '26.13.1',
      installed: {
        version: '24.1.0',
        sha256: 'ab'.repeat(32),
        dir: 'node/24.1.0-abababababababab',
      },
      system: null,
      needed_by: ['pi'],
    },
    {
      id: 'jq',
      origin: 'builtin',
      kind: 'utility',
      program: 'jq',
      release: '1.8.2',
      installed: { version: '1.8.2', sha256: 'cd'.repeat(32), dir: 'jq/1.8.2-cdcdcdcdcdcdcdcd' },
      system: { path: '/usr/bin/jq', version: null, unread: 'jq built by acme' },
      needed_by: [],
    },
    {
      id: 'fd',
      origin: 'builtin',
      kind: 'utility',
      program: 'fd',
      release: '10.5.2',
      installed: null,
      system: null,
      needed_by: [],
    },
    {
      id: 'git',
      origin: 'builtin',
      kind: 'utility',
      program: 'git',
      release: null,
      installed: null,
      system: null,
      needed_by: [],
    },
    {
      id: 'zig',
      origin: 'operator',
      kind: 'compiler',
      program: 'zig',
      release: null,
      installed: null,
      system: null,
      needed_by: [],
    },
    {
      id: 'acme-lint',
      origin: null,
      kind: null,
      program: 'acme-lint',
      release: null,
      installed: {
        version: '0.3.0',
        sha256: 'ef'.repeat(32),
        dir: 'acme-lint/0.3.0-efefefefefefefef',
      },
      system: null,
      needed_by: [],
    },
  ],
  generations: [
    {
      generation: 3,
      state: 'current',
      installs: [
        { package: 'jq', version: '1.8.2' },
        { package: 'node', version: '24.1.0' },
      ],
    },
    { generation: 2, state: 'in_use', installs: [{ package: 'node', version: '24.1.0' }] },
  ],
};

const respond =
  (status: number, body: unknown, seen: string[] = []): typeof fetch =>
  async (input, init) => {
    seen.push(`${init?.method} ${String(input)} ${init?.body ?? ''}`);
    return Response.json(body, { status });
  };

async function listing(): Promise<PackageListing> {
  return new PackagesApi(respond(200, wire)).listing();
}

test('a listing decodes, keeping kinds this UI does not know', async () => {
  const read = await listing();
  assert.equal(read.generation, 3);
  assert.equal(read.platform, 'linux_x64');
  assert.equal(read.packages[5]?.kind, 'compiler');
  assert.equal(read.packages[5]?.origin, 'operator');
  assert.equal(read.packages[6]?.kind, null);
  assert.equal(read.generations[1]?.state, 'in_use');
  await assert.rejects(new PackagesApi(respond(200, { ...wire, api_version: 2 })).listing());
});

test('each package has one standing', async () => {
  const read = await listing();
  assert.deepEqual(
    read.packages.map((p) => Standings.of(p)),
    [
      { kind: 'system' },
      { kind: 'outdated', installed: '24.1.0', release: '26.13.1' },
      { kind: 'installed', version: '1.8.2' },
      { kind: 'available', release: '10.5.2' },
      { kind: 'missing' },
      { kind: 'missing' },
      { kind: 'uncatalogued', installed: '0.3.0' },
    ],
  );
});

test('packages group by kind, runtimes first, unknown kinds and uncatalogued installs last', async () => {
  const groups = PackageGroups.of(await listing());
  assert.deepEqual(
    groups.map((g) => [g.title, g.packages.map((p) => p.id)]),
    [
      ['Runtimes', ['node']],
      ['Utilities', ['jq', 'fd', 'git']],
      ['Clients', ['claude']],
      ['compiler', ['zig']],
      ['Not in the catalog', ['acme-lint']],
    ],
  );
});

test('a plan stages one change per package and previews it in listing order', async () => {
  const read = await listing();
  const [claude, node, jq, fd] = read.packages;
  assert.ok(claude && node && jq && fd);
  let plan = PackagePlan.EMPTY.toggle(fd).toggle(node).toggle(jq).toggle(claude);
  assert.deepEqual(plan.changes(read), [
    { change: 'replace', id: 'node', from: '24.1.0', to: '26.13.1' },
    { change: 'remove', id: 'jq', from: '1.8.2', to: null },
    { change: 'add', id: 'fd', from: null, to: '10.5.2' },
  ]);
  assert.deepEqual(plan.body(), { add: ['fd', 'node'], remove: ['jq'] });
  // An outdated install can be removed instead of updated.
  plan = plan.toggleRemove(node);
  assert.deepEqual(plan.body(), { add: ['fd'], remove: ['jq', 'node'] });
  // An install the catalog no longer names can only be removed.
  const acme = read.packages[6];
  assert.ok(acme);
  assert.deepEqual(PackagePlan.EMPTY.toggle(acme).changes(read), [
    { change: 'remove', id: 'acme-lint', from: '0.3.0', to: null },
  ]);
  // Toggling again unstages.
  plan = plan.toggle(fd).toggle(jq).toggleRemove(node);
  assert.ok(plan.empty);
});

test('a plan keeps only what still applies after the listing changed', async () => {
  const read = await listing();
  const [, node, , fd] = read.packages;
  assert.ok(node && fd);
  const plan = PackagePlan.EMPTY.toggle(fd).toggle(node);
  const after: PackageListing = {
    ...read,
    packages: read.packages.map((p) =>
      p.id === 'fd'
        ? { ...p, installed: { version: p.release ?? ('' as never), sha256: '', dir: '' } }
        : p,
    ),
  };
  assert.deepEqual(plan.within(after).body(), { add: ['node'], remove: [] });
});

test('changes and collections post their bodies; refusals read as sentences', async () => {
  const seen: string[] = [];
  const api = new PackagesApi(
    respond(
      200,
      {
        api_version: 1,
        outcome: 'activated',
        generation: 4,
        changes: [{ change: 'add', package: 'fd', from: null, to: '10.5.2' }],
        failures: [],
      },
      seen,
    ),
  );
  const changed = await api.change(new PackagePlan(['fd' as never]));
  assert.equal(changed.outcome, 'activated');
  await api.rollBack(2 as never);
  assert.deepEqual(seen, [
    'POST /control/packages/change {"add":["fd"],"remove":[]}',
    'POST /control/packages/rollback {"generation":2}',
  ]);

  const refused = new PackagesApi(
    respond(422, { error: 'packages_refused', message: 'fd is not installed by Ren' }),
  );
  await assert.rejects(refused.change(new PackagePlan([], ['fd' as never])), (error: unknown) => {
    assert.ok(error instanceof PackagesError);
    assert.equal(error.message, 'fd is not installed by Ren.');
    return true;
  });
  assert.equal(
    PackagesError.describe(404, 'runs_off', null),
    'Set REN_CONTROL_TOKEN_FILE on the UI server to manage packages here.',
  );
  assert.equal(PackagesError.describe(418, null, null), 'The request was refused (418).');
});

test('where runs get each package, and the header tally', async () => {
  const read = await listing();
  const [claude, node, jq, fd, git, , acme] = read.packages;
  assert.ok(claude && node && jq && fd && git && acme);
  assert.deepEqual(Supplies.of(claude), {
    kind: 'system',
    system: { path: '/opt/acme/bin/claude', version: '2.1.0', unread: null },
  });
  assert.deepEqual(Supplies.of(node), { kind: 'managed', version: '24.1.0', shadows: null });
  assert.deepEqual(Supplies.of(jq), {
    kind: 'managed',
    version: '1.8.2',
    shadows: { path: '/usr/bin/jq', version: null, unread: 'jq built by acme' },
  });
  assert.deepEqual(Supplies.of(fd), { kind: 'none' });
  assert.deepEqual(
    [claude, node, fd, git].map((p) => Supplies.tone(p)),
    ['system', 'managed', 'none', 'none'],
  );
  assert.equal(Supplies.tone({ ...fd, needed_by: ['pi'] }), 'missing');
  assert.deepEqual(Supplies.tally(read), { managed: 3, system: 1, missing: 0, updates: 1 });
  assert.deepEqual(Supplies.mark(claude), { mark: 'idle', word: 'On the host’s PATH' });
  assert.deepEqual(Supplies.mark(fd), { mark: 'idle', word: 'Not installed' });
  assert.deepEqual(Supplies.mark({ ...fd, needed_by: ['pi'] }), {
    mark: 'failed',
    word: 'Missing',
  });
  const marks = read.packages.map((p) => Supplies.mark(p).mark);
  assert.equal(marks.filter((mark) => mark === 'warning').length, 1, 'the one update needs a look');
  assert.equal(marks.filter((mark) => mark === 'passed').length, 2, 'the other installs pass');
  assert.equal(Platforms.label('linux_x64'), 'Linux x64');
});

test('each row stages one change, named with the version it brings', async () => {
  const read = await listing();
  assert.deepEqual(
    read.packages.map((p) => RowActions.of(p)),
    [
      null,
      { kind: 'update', version: '26.13.1' },
      { kind: 'remove' },
      { kind: 'install', version: '10.5.2' },
      null,
      null,
      { kind: 'remove' },
    ],
  );
});

test('a staged change shows the version runs use before and after it', async () => {
  const read = await listing();
  const [claude, node, jq, fd] = read.packages;
  assert.ok(claude && node && jq && fd);
  const plan = PackagePlan.EMPTY.toggle(node).toggle(jq).toggle(fd);
  assert.deepEqual(plan.effects(read), [
    { change: 'replace', id: 'node', from: '24.1.0', to: '26.13.1', toKind: 'managed' },
    // Removing Ren's jq leaves runs the PATH's, whose version was not read.
    { change: 'remove', id: 'jq', from: '1.8.2', to: '?', toKind: 'system' },
    { change: 'add', id: 'fd', from: null, to: '10.5.2', toKind: 'managed' },
  ]);
  // Installing over a PATH copy starts from the PATH's version.
  const onPath = {
    ...fd,
    system: { path: '/usr/bin/fd', version: '9.0.0' as never, unread: null },
  };
  assert.deepEqual(
    Supplies.effect(onPath, { change: 'add', id: fd.id, from: null, to: '10.5.2' as never }),
    {
      change: 'add',
      id: 'fd',
      from: '9.0.0',
      to: '10.5.2',
      toKind: 'managed',
    },
  );
});

test('the view narrows by name, program, or path, and by how packages stand', async () => {
  const read = await listing();
  const ids = (query: string, only: Parameters<typeof PackageFilters.keeps>[1]['only']) =>
    PackageGroups.of(read, { query, only }).flatMap((g) => g.packages.map((p) => p.id));
  assert.deepEqual(ids('', 'managed'), ['node', 'jq', 'acme-lint']);
  assert.deepEqual(ids('', 'system'), ['claude']);
  assert.deepEqual(ids('', 'updates'), ['node']);
  assert.deepEqual(ids('', 'missing'), []);
  assert.deepEqual(ids('/opt/acme', null), ['claude']);
  assert.deepEqual(ids('J', null), ['jq']);
  assert.deepEqual(ids('jq', 'system'), []);
});

test('a change says what it came to in a toast', () => {
  const base = { generation: 15 as never, changes: [], failures: [] };
  assert.deepEqual(
    GenerationChanges.said({
      ...base,
      outcome: 'activated',
      changes: [
        {
          change: 'replace',
          package: 'node' as never,
          from: '24.0.0' as never,
          to: '24.1.0' as never,
        },
      ],
    }),
    { mark: 'passed', text: 'Generation 15 is current', detail: 'node 24.1.0' },
  );
  assert.equal(GenerationChanges.said({ ...base, outcome: 'unchanged' }).mark, 'idle');
  assert.deepEqual(
    GenerationChanges.said({
      ...base,
      outcome: 'failed',
      failures: [
        { package: 'fd' as never, version: '10.2.0' as never, stage: 'verify', reason: 'sha256' },
      ],
    }),
    { mark: 'failed', text: '1 install failed', detail: 'fd · verify' },
  );
});

test('the history sets the newest generations side by side, marking where a package changed', () => {
  const generation = (n: number, installs: [string, string][]) => ({
    generation: n as never,
    state: 'idle' as never,
    installs: installs.map(([pkg, version]) => ({
      package: pkg as never,
      version: version as never,
    })),
  });
  const history = new PackageHistory(
    {
      generations: [
        generation(12, [['rg', '14.0']]),
        generation(14, [
          ['rg', '14.1'],
          ['fd', '10.2'],
        ]),
        generation(13, [['rg', '14.1']]),
      ],
    } as never,
    2,
  );
  assert.deepEqual(
    history.generations.map((g) => g.generation),
    [14, 13],
  );
  assert.deepEqual(
    [history.version('rg' as never, 0), history.version('rg' as never, 1)],
    ['14.1', '14.1'],
  );
  // 14 added fd; 13 updated rg from 12, the generation past the columns.
  assert.equal(history.changed('fd' as never, 0), true);
  assert.equal(history.changed('rg' as never, 0), false);
  assert.equal(history.changed('rg' as never, 1), true);
  assert.equal(history.version('fd' as never, 1), null);
});

test('a row button names its change, and Undo once staged', async () => {
  const read = await listing();
  assert.deepEqual(
    read.packages.map((p) => RowActions.label(p, PackagePlan.EMPTY)),
    [null, 'Update 26.13.1', 'Remove', 'Install 10.5.2', null, null, 'Remove'],
  );
  const [, node, , fd] = read.packages;
  assert.ok(node && fd);
  const plan = PackagePlan.EMPTY.toggle(node);
  assert.equal(RowActions.label(node, plan), 'Undo');
  assert.equal(RowActions.label(fd, plan), 'Install 10.5.2');
});

test('origins, generation states and change kinds read as the view words them', async () => {
  const read = await listing();
  assert.deepEqual(
    read.packages.map((p) => Origins.label(p.origin)),
    ['builtin', 'builtin', 'builtin', 'builtin', 'builtin', 'operator', 'removed'],
  );
  assert.equal(Origins.label('vendor' as never), 'vendor');
  assert.deepEqual(
    (['current', 'in_use', 'idle', 'retired'] as never[]).map((s) => GenerationStates.label(s)),
    ['current', 'in use', '', 'retired'],
  );
  assert.deepEqual(
    (['add', 'remove', 'replace'] as const).map((k) => ChangeKinds.sign(k)),
    ['+', '−', '↑'],
  );
});

test('an install hash shows its first twelve digits', async () => {
  const read = await listing();
  const installed = read.packages[1]?.installed;
  assert.ok(installed);
  assert.equal(InstalledPackages.shortSha(installed), `${'ab'.repeat(6)}…`);
});

test('restore offers the newest generation older than the current one', async () => {
  const read = await listing();
  assert.equal(PackageHistory.previous(read)?.generation, 2);
  assert.equal(PackageHistory.previous({ ...read, generations: [] }), null);
});

test('a failed request says its message, whatever it threw', () => {
  assert.equal(
    PackagesError.say(new PackagesError(409, null, null)),
    'The request was refused (409).',
  );
  assert.equal(PackagesError.say(new Error('offline')), 'offline');
  assert.equal(PackagesError.say('gone'), 'gone');
});
