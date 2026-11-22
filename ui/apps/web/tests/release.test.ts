import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { decisions, ManagementApi, type ReleaseView } from '../src/lib/api.ts';
import {
  Previewing,
  Promotion,
  PullChips,
  PullWatch,
  ReleaseFigures,
  ReleaseLine,
  ReleasePage,
  ReleaseStrip,
} from '../src/lib/release.ts';
import type { Pull } from '../src/lib/api.ts';
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

test('a release draws as its pipeline and the word for what promotion decided', async () => {
  const release = await new ManagementApi(pinned).release();
  assert.ok(release);
  const strip = ReleaseStrip.of(release);
  assert.equal(strip.canary, 'passed');
  assert.deepEqual(strip.production, { kind: 'behind', commits: 1 });
  assert.equal(strip.promotion.word, 'Waiting on you');
  assert.equal(strip.promotion.tone, 'warning');
  assert.equal(strip.label, 'Production 1 behind main · canary passed · Waiting on you');

  const live = ReleaseStrip.of({ ...release, behind: 0, decision: 'current' });
  assert.deepEqual(live.production, { kind: 'on_main' });
  assert.equal(live.promotion.word, 'Live');
  const lost = ReleaseStrip.of({ ...release, behind: null, canary: null, decision: 'unchecked' });
  assert.deepEqual(lost.production, { kind: 'uncounted' });
  assert.equal(lost.canary, 'none');
  const failed = ReleaseStrip.of({
    ...release,
    canary: { ...release.canary!, passed: false },
    decision: 'failed',
  });
  assert.equal(failed.canary, 'failed');
  assert.equal(failed.promotion.word, 'Blocked');
  assert.equal(failed.promotion.tone, 'bad');
});

test('every decision has its own word, and an unknown one keeps its text', () => {
  const words = new Set(decisions.map((decision) => Promotion.of(decision).word));
  assert.equal(words.size, decisions.length);
  const later = Promotion.of('held_for_review' as ReleaseView['decision']);
  assert.equal(later.word, 'held_for_review');
  assert.equal(later.tone, 'warning');
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

const pinnedFactory: typeof fetch = async (input) => {
  const path = new URL(String(input), 'http://ui.test').pathname;
  return Response.json(fixture(path.endsWith('/pulls') ? 'project_pulls' : 'project_previews'));
};

test("a project's pull requests and previews decode, and read as rows", async () => {
  const api = new ManagementApi(pinnedFactory);
  const watch = await api.projectPulls('ren');
  const previews = await api.projectPreviews('ren');
  const pulls = PullWatch.pulls(watch);
  assert.equal(PullWatch.repository(watch), 'acme/ren');
  assert.equal(PullWatch.note(watch), null);
  assert.equal(pulls.length, 1);
  const [pull] = pulls as [Pull];
  // Checks still running come before being behind its base.
  assert.deepEqual(PullChips.of(pull, 'status', previews.previews), {
    mark: 'running',
    label: 'Checks running',
  });
  assert.deepEqual(PullChips.of(pull, 'checks', previews.previews), {
    mark: 'running',
    label: '1/2 checks passed',
  });
  assert.deepEqual(PullChips.of(pull, 'preview', previews.previews), {
    mark: 'passed',
    label: 'Up',
  });
  assert.deepEqual(
    Previewing.rows(previews.previews, pulls, 'ren.example').map((row) => [
      row.slot,
      row.chip.mark,
      row.of,
      row.action,
    ]),
    [
      [
        0,
        'passed',
        '#12 feat: a landing page',
        { kind: 'open', href: 'https://ren.example:9451/' },
      ],
      [1, 'failed', '333333333333', { kind: 'retry', commit: '3'.repeat(40) }],
    ],
  );
});

test('a pull request stands by what blocks it most', () => {
  const pull = (overrides: Partial<Pull>): Pull => ({
    number: 1,
    title: 't',
    url: 'u',
    author: null,
    readiness: 'ready',
    head_branch: 'b',
    head: 'h',
    base_branch: 'main',
    merge: 'clean',
    review: 'not_required',
    checks: 'passing',
    check_list: [],
    work_items: [],
    ...overrides,
  });
  const status = (overrides: Partial<Pull>) => PullChips.status(pull(overrides)).label;
  assert.equal(status({}), 'Ready to merge');
  assert.equal(status({ readiness: 'draft', checks: 'failing' }), 'Draft');
  assert.equal(status({ checks: 'failing', merge: 'conflicting' }), 'Checks failing');
  assert.equal(status({ merge: 'conflicting' }), 'Conflicts with its base');
  assert.equal(status({ review: 'review_required', merge: 'blocked' }), 'Review required');
  assert.equal(PullChips.checks(pull({ checks: 'none' })).label, 'No checks');
  assert.equal(PullWatch.note({ watch: 'off' }), 'No GitHub repository watched');
});

test('the figures under the flow', async () => {
  const release = await new ManagementApi(pinned).release();
  assert.ok(release);
  const figures = ReleaseFigures.of(new ReleasePage(release, null), []);
  assert.deepEqual(
    figures.map((item) => [item.label, item.value]),
    [
      ['Production', '1 behind main'],
      ['Canary', `${release.canary?.checks.length}/${release.canary?.checks.length}`],
      ['Policy', 'manual'],
      ['Open pull requests', '0'],
      ['Next', `Promote ${release.canary?.commit.slice(0, 7)}`],
    ],
  );
});
