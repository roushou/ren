import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { PageEvidence, Work, WorkApi, type WorkItem } from '../src/lib/work.ts';

const pinned: typeof fetch = async () =>
  Response.json(
    JSON.parse(readFileSync(new URL('./fixtures/api/work.json', import.meta.url), 'utf8')),
  );

const page = {
  path: '/recordings',
  title: 'Ren - Recordings',
  width: 1440,
  height: 900,
  theme: 'light' as const,
};
const mark = (id: number, note: string) => ({
  id,
  note,
  target: { selector: 'main > table', text: 'acme-api', source: null, width: 640.4, height: 28 },
});

test("the daemon's work items decode", async () => {
  const [item] = await new WorkApi(pinned).list();
  assert.ok(item);
  assert.equal(item.state, 'queued');
  assert.equal(item.title, "Show each recording's turn count");
  assert.deepEqual(item.order.page.revision, {
    kind: 'commit',
    commit: '62e9e9e0000000000000000000000000000000ff',
    tree: 'clean',
  });
  assert.deepEqual(item.order.evidence.console, ['TypeError: turns is undefined']);
});

test('an order keeps only noted marks, and its revision as the daemon writes it', () => {
  const order = Work.order(
    page,
    {
      route: '/recordings',
      revision: { kind: 'commit', commit: 'a'.repeat(40), tree: 'modified' },
    },
    [mark(1, ' Show the turn count '), mark(2, '  ')],
    { console: [], reads: ['/api/recordings?limit=100'] },
  );
  assert.equal(order.marks.length, 1);
  assert.equal(order.marks[0]?.note, 'Show the turn count');
  assert.equal(order.marks[0]?.width, 640);
  assert.equal(order.page.revision, `${'a'.repeat(40)}-dirty`);
  assert.equal(order.page.route, '/recordings');
});

test('a brief names its work item and what else was on screen', async () => {
  const [item] = await new WorkApi(pinned).list();
  assert.ok(item);
  const brief = Work.brief(item.id, Work.wire(item.order), 'workspace');
  assert.match(brief, /^Redline: Show each recording's turn count\n/);
  assert.match(brief, new RegExp(`This is work item \`${item.id}\`\\.`));
  assert.match(brief, /The page logged an error: `TypeError: turns is undefined`\./);
  assert.match(brief, /`\/api\/recordings\?limit=100`/);
  assert.match(brief, /workspace of its own/);
  const preview = Work.brief(null, Work.wire(item.order), 'workspace');
  assert.doesNotMatch(preview, /This is work item/);
  // The change names the item in its commits and pull request, so Ren
  // knows when it lands.
  assert.match(brief, new RegExp(`End each commit message with \`Work-Item: ${item.id}\``));
  assert.match(preview, /the work item's `Work-Item:` line/);
  assert.match(brief, /run `cargo xtask preview up --publish` in the workspace/);
  assert.equal(Work.previewed(item), null);
  assert.equal(
    Work.previewed({
      ...item,
      preview: { ui: 'https://ren.example.ts.net:20201', revision: { kind: 'unknown' } },
    }),
    'https://ren.example.ts.net:20201/recordings',
  );
});

test('several items make one brief, each under its page, closed once', async () => {
  const [first] = await new WorkApi(pinned).list();
  assert.ok(first);
  const second: WorkItem = {
    ...first,
    id: '00000000-0000-0000-0000-00000000000c' as WorkItem['id'],
    title: 'Move the archive button',
    order: {
      ...first.order,
      page: { ...first.order.page, path: '/sessions', route: '/sessions' },
      marks: [{ ...first.order.marks[0]!, note: 'Move the archive button' }],
      evidence: { console: [], reads: [] },
    },
  };
  const brief = Work.batch([first, second], 'workspace');
  assert.match(brief, /^Redline: 2 work items\n/);
  assert.match(brief, /## 1\. Show each recording's turn count \(work item `[0-9a-f-]+`\)/);
  assert.match(brief, /## 2\. Move the archive button/);
  assert.match(brief, /Marked on `\/sessions`/);
  assert.equal(brief.match(/Don't install or restart anything/g)?.length, 1);
  assert.match(brief, new RegExp(`\`Work-Item: ${first.id}\` and \`Work-Item: ${second.id}\``));
});

test("a page's evidence is bounded, deduplicated, and of its own origin's API", () => {
  const evidence = new PageEvidence();
  for (let n = 0; n < PageEvidence.KEEP + 10; n += 1) evidence.error(`error ${n}`);
  evidence.read('/api/calls?limit=100');
  evidence.read('/api/calls?limit=100');
  const kept = evidence.snapshot();
  assert.equal(kept.console.length, PageEvidence.KEEP);
  assert.equal(kept.console[0], 'error 10');
  assert.deepEqual(kept.reads, ['/api/calls?limit=100']);
  evidence.reset();
  assert.deepEqual(evidence.snapshot(), { console: [], reads: [] });
  const origin = 'https://ren.example';
  assert.equal(
    PageEvidence.readOf(`${origin}/api/usage?from_ms=1`, origin),
    '/api/usage?from_ms=1',
  );
  assert.equal(PageEvidence.readOf(`${origin}/api/live?after=3`, origin), null);
  assert.equal(PageEvidence.readOf(`${origin}/_app/immutable/x.js`, origin), null);
  assert.equal(PageEvidence.readOf('https://elsewhere.example/api/usage', origin), null);
});

test('a change to an item not yet stored is tried again', async () => {
  let tries = 0;
  const settling: typeof fetch = async () => {
    tries += 1;
    return tries < 3
      ? Response.json({ api_version: 1, error: 'not_found' }, { status: 404 })
      : Response.json(
          { api_version: 1, id: '00000000-0000-0000-0000-00000000000b', state: 'sent' },
          { status: 202 },
        );
  };
  const state = await new WorkApi(settling).move('00000000-0000-0000-0000-00000000000b' as never, {
    kind: 'send',
    session: '00000000-0000-0000-0000-0000000000aa' as never,
  });
  assert.equal(state, 'sent');
  assert.equal(tries, 3);
  // Any other refusal is not tried again.
  let refused = 0;
  const conflict: typeof fetch = async () => {
    refused += 1;
    return Response.json({ api_version: 1, error: 'work_state' }, { status: 409 });
  };
  await assert.rejects(
    new WorkApi(conflict).move('00000000-0000-0000-0000-00000000000b' as never, {
      kind: 'dismiss',
    }),
  );
  assert.equal(refused, 1);
});

test("an item for another project is briefed as that project's page", async () => {
  const [item] = await new WorkApi(pinned).list();
  assert.ok(item);
  const ren = Work.brief(item.id, Work.wire(item.order), 'workspace', { kind: 'ren' });
  assert.match(
    ren,
    /In Ren's UI this page is `ui\/apps\/web\/src\/routes\/recordings\/\+page\.svelte`\./,
  );
  const other = Work.batch([item], 'workspace', { kind: 'other', name: 'acme-web' });
  assert.doesNotMatch(other, /Ren's UI this page is|ui\/apps\/web/);
  assert.doesNotMatch(other, /served by a build of commit/);
  assert.match(other, /It is a page of the project `acme-web`, not of Ren's own UI\./);
  assert.match(other, /run `ren project preview up --publish` in the workspace/);
  assert.doesNotMatch(other, /cargo xtask preview/);
});

test("an item's project decides its brief, and a batch is one project's", () => {
  const project = (id: string, name: string, factory: 'ren' | 'contract') =>
    ({ id, name, factory }) as never;
  const projects = [project('ren-id', 'ren', 'ren'), project('acme-id', 'acme-web', 'contract')];
  const item = (id: string, owner: string | null) =>
    ({ id, project: owner }) as unknown as WorkItem;
  assert.deepEqual(Work.projectOf(item('a', 'acme-id'), projects), {
    kind: 'other',
    name: 'acme-web',
  });
  // Ren's own, none, and one not listed are all Ren's.
  for (const owner of ['ren-id', null, 'gone-id']) {
    assert.deepEqual(Work.projectOf(item('b', owner), projects), { kind: 'ren' });
  }
  assert.deepEqual(Work.oneProject([item('a', 'acme-id'), item('c', 'acme-id')], projects), {
    kind: 'other',
    name: 'acme-web',
  });
  assert.deepEqual(Work.oneProject([item('b', null), item('d', 'ren-id')], projects), {
    kind: 'ren',
  });
  assert.equal(Work.oneProject([item('a', 'acme-id'), item('b', null)], projects), null);
});

test("a project's workspaces decode, its sessions' worktrees with their ports", async () => {
  const seen: string[] = [];
  const answer = {
    api_version: 1,
    workspaces: [
      {
        name: 'acme-1',
        folder: '/data/worktrees/acme-1',
        data: '/data/worktrees/.data/acme-1',
        ports: [20000, 20099],
        session: '0190f3a0-0000-7000-8000-00000000a001',
        prompt: 'fix the order list sorting',
        running: true,
        work: [
          {
            id: '0190f3a0-0000-7000-8000-00000000000b',
            title: 'Sort orders by date',
            state: 'sent',
            preview: 'http://127.0.0.1:20000',
          },
        ],
      },
    ],
  };
  const fetcher = (async (input: RequestInfo | URL) => {
    seen.push(String(input));
    return Response.json(answer);
  }) as typeof fetch;
  const [workspace] = await new WorkApi(fetcher).workspaces('acme web');
  assert.deepEqual(seen, ['/api/projects/acme%20web/workspaces']);
  assert.deepEqual(workspace?.ports, [20000, 20099]);
  assert.equal(workspace?.work[0]?.state, 'sent');
  const malformed = { ...answer, workspaces: [{ ...answer.workspaces[0], ports: [20000] }] };
  await assert.rejects(
    new WorkApi((async () => Response.json(malformed)) as typeof fetch).workspaces('acme'),
  );
});
