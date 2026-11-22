import test from 'node:test';
import assert from 'node:assert/strict';
import { Views } from '../src/lib/view.ts';

test('each route is its view, and an unknown one is Traffic', () => {
  assert.equal(Views.of('/'), 'home');
  assert.equal(Views.of('/projects'), 'projects');
  // Home sits under the wordmark, in no place and on no row of pages.
  assert.equal(Views.place(Views.tab('home')), 'home');
  assert.deepEqual(Views.pages('home'), []);
  assert.equal(Views.of('/projects/[name]/work/[id]'), 'work');
  assert.equal(Views.of('/projects/[name]/sessions'), 'sessions');
  assert.equal(Views.of('/projects/[name]/release'), 'release');
  assert.equal(Views.of('/scratch'), 'scratch');
  assert.equal(Views.of('/monitor/overview'), 'overview');
  assert.equal(Views.of('/monitor/traffic'), 'traffic');
  assert.equal(Views.of('/monitor/sessions/[id]'), 'recording');
  assert.equal(Views.of('/monitor/sessions/[id]/report'), 'report');
  assert.equal(Views.of(null), 'traffic');
});

test("a session's calls, its report, and its turns sit under Monitor's Sessions tab", () => {
  for (const view of ['recordings', 'recording', 'report'] as const) {
    assert.equal(Views.tab(view), 'recordings');
  }
  // A work item sits under its project's Release, where work is listed.
  assert.equal(Views.tab('work'), 'release');
  assert.equal(Views.tab('traffic'), 'traffic');
  assert.equal(Views.title('recording'), 'Session calls');
  assert.equal(Views.title('recorder'), 'Recorder & storage');
});

test('three places hold every page, each page in exactly one', () => {
  assert.deepEqual(
    Views.places.map((place) => place.label),
    ['Projects', 'Monitor', 'System'],
  );
  const pages = Views.places.flatMap((place) => Views.pages(place.place, 'acme-web'));
  assert.equal(new Set(pages.map((page) => page.tab)).size, pages.length);
  for (const place of Views.places) {
    for (const page of Views.pages(place.place, 'acme-web')) {
      assert.equal(Views.place(page.tab), place.place, page.tab);
    }
    // Each place opens where its name leads: Projects on its list, the others
    // on their first page.
    assert.equal(
      Views.of(place.href),
      place.place === 'projects' ? 'projects' : Views.pages(place.place)[0]?.tab,
    );
  }
  assert.equal(Views.place(Views.tab('report')), 'monitor');
  assert.equal(Views.place(Views.tab('usage')), 'monitor');
  assert.equal(Views.place(Views.tab('work')), 'projects');
  assert.equal(Views.place('recorder'), 'system');
});

test("the Projects list has no pages; inside a project, the row holds the project's", () => {
  assert.deepEqual(Views.pages('projects'), []);
  assert.deepEqual(
    Views.pages('projects', 'acme web').map((page) => [page.tab, page.href]),
    [
      ['sessions', '/projects/acme%20web/sessions'],
      ['release', '/projects/acme%20web/release'],
    ],
  );
});

test('only the pages that read a project carry it between them', () => {
  const traffic = { tab: 'traffic', href: '/monitor/traffic', label: 'Traffic' } as const;
  const live = { tab: 'live', href: '/monitor/live', label: 'Live' } as const;
  const packages = { tab: 'packages', href: '/system/packages', label: 'Packages' } as const;
  assert.equal(Views.scopedHref(traffic, 'acme-web'), '/monitor/traffic?project=acme-web');
  assert.equal(Views.scopedHref(traffic, null), '/monitor/traffic');
  // Monitor's pages read a project; System's do not.
  assert.equal(Views.scopedHref(live, 'acme-web'), '/monitor/live?project=acme-web');
  assert.equal(Views.scopedHref(packages, 'acme-web'), '/system/packages');
  assert.ok(Views.scoped('recordings'));
  assert.ok(Views.scoped('search'));
  assert.equal(Views.scoped('usage'), false);
});

test('an address from before the places leads to the same page in its place', () => {
  assert.equal(Views.moved('/traffic'), '/monitor/traffic');
  assert.equal(Views.moved('/recordings/r1/report'), '/monitor/sessions/r1/report');
  assert.equal(Views.moved('/monitor/recordings'), '/monitor/sessions');
  assert.equal(Views.moved('/monitor/recordings/r1'), '/monitor/sessions/r1');
  assert.equal(Views.moved('/turns/t1'), '/monitor/turns/t1');
  assert.equal(Views.moved('/tools'), '/system/packages');
  assert.equal(Views.moved('/system/tools'), '/system/packages');
  assert.equal(Views.moved('/recorder'), '/system/recorder');
  for (const kept of [
    '/',
    '/projects',
    '/projects/acme-web/sessions',
    '/scratch',
    '/monitor/traffic',
    '/api/calls',
    '/trafficking',
  ]) {
    assert.equal(Views.moved(kept), null, kept);
  }
});
