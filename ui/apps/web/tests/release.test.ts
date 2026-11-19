import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { ManagementApi, type ReleaseView } from '../src/lib/api.ts';
import { ReleaseLine, ReleasePage } from '../src/lib/release.ts';
import type { WorkItem } from '../src/lib/work.ts';

const fixture = (name: string) =>
  JSON.parse(readFileSync(new URL(`./fixtures/api/${name}.json`, import.meta.url), 'utf8'));
/** The pinned answers: Ren's project's release, and the project list. */
const pinned: typeof fetch = async (input) => {
  const path = new URL(String(input), 'http://ui.test').pathname;
  return path === '/api/projects/ren/release'
    ? Response.json(fixture('project_release'))
    : Response.json(fixture('projects'));
};

test("the daemon's release decodes and reads as one status line", async () => {
  const release = await new ManagementApi(pinned).release();
  assert.ok(release);
  const line = ReleaseLine.of(release);
  assert.equal(line.text, 'Production 1 behind main · canary passed');
  assert.equal(line.tone, 'warning');
  assert.match(
    line.detail,
    /^main 62e9e9e00000\nproduction 07f4bb7c1e2a\ncanary 62e9e9e00000\n  doctor passed/,
  );
  assert.match(
    line.detail,
    /promotion \(manual\): passed; waiting for the operator to promote it$/,
  );
});

test('a failed canary and an unknown decision read as such', () => {
  const release: ReleaseView = {
    main: '62e9e9e0000000000000000000000000000000ff',
    production: {
      kind: 'commit',
      commit: '62e9e9e0000000000000000000000000000000ff',
      tree: 'clean',
    },
    behind: 0,
    canary: {
      commit: '62e9e9e0000000000000000000000000000000ff',
      passed: false,
      checks: [
        { check: 'pages', name: null, passed: false, detail: 'the sessions page did not load' },
      ],
    },
    policy: 'automatic',
    decision: 'failed',
  };
  const line = ReleaseLine.of(release);
  assert.equal(line.text, 'Production on main · canary failed');
  assert.equal(line.tone, 'bad');
  assert.match(line.detail, /pages failed: the sessions page did not load/);
  const later = ReleaseLine.of({
    ...release,
    decision: 'held_for_review' as ReleaseView['decision'],
  });
  assert.equal(later.tone, 'warning');
  assert.match(later.detail, /held_for_review$/);
});

test('the Release view promotes only a passed build production does not run, once', () => {
  const release: ReleaseView = {
    main: 'b'.repeat(40),
    production: { kind: 'commit', commit: 'a'.repeat(40), tree: 'clean' },
    behind: 2,
    canary: { commit: 'b'.repeat(40), passed: true, checks: [] },
    policy: 'manual',
    decision: 'awaiting_operator',
  };
  const page = new ReleasePage(release, null);
  assert.equal(page.promotable, true);
  assert.equal(page.asked, false);
  assert.equal(page.behind, '2 commits behind main');
  assert.equal(new ReleasePage(release, 'b'.repeat(40)).asked, true);
  assert.equal(new ReleasePage({ ...release, decision: 'current' }, null).promotable, false);
  assert.equal(
    new ReleasePage({ ...release, canary: { ...release.canary!, passed: false } }, null).promotable,
    false,
  );
  const item = (state: WorkItem['state'], shipped: string | null): WorkItem =>
    ({
      id: 'x',
      state,
      shipped: shipped ? { kind: 'commit', commit: shipped, tree: 'clean' } : null,
    }) as unknown as WorkItem;
  const items = [
    item('shipped', 'a'.repeat(40)),
    item('shipped', 'c'.repeat(40)),
    item('landed', null),
  ];
  assert.equal(page.shipped(items).length, 1);
  assert.equal(ReleasePage.waiting(items).length, 1);
});

test("Ren's release is read as `ren`'s, else as the project registered as Ren", async () => {
  const asked: string[] = [];
  const renamed: typeof fetch = async (input) => {
    const path = new URL(String(input), 'http://ui.test').pathname;
    asked.push(path);
    if (path === '/api/projects/ren/release') {
      return Response.json({ api_version: 1, error: 'not_found' }, { status: 404 });
    }
    if (path === '/api/projects') {
      const listed = fixture('projects');
      for (const project of listed.projects) {
        if (project.factory === 'ren') project.name = 'ren-main';
      }
      return Response.json(listed);
    }
    return Response.json(fixture('project_release'));
  };
  const release = await new ManagementApi(renamed).release();
  assert.ok(release);
  assert.deepEqual(asked, [
    '/api/projects/ren/release',
    '/api/projects',
    '/api/projects/ren-main/release',
  ]);
});
