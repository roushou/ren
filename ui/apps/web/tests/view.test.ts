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
  assert.equal(Views.of('/overview'), 'overview');
  assert.equal(Views.of('/traffic'), 'traffic');
  assert.equal(Views.of('/recordings/[id]'), 'recording');
  assert.equal(Views.of('/recordings/[id]/report'), 'report');
  assert.equal(Views.of('/turns/[id]'), 'turn');
  assert.equal(Views.of(null), 'traffic');
});

test('a recording, its report, and its turns sit under the Recordings tab', () => {
  for (const view of ['recordings', 'recording', 'report', 'turn'] as const) {
    assert.equal(Views.tab(view), 'recordings');
  }
  // A work item sits under its project's Release, where work is listed.
  assert.equal(Views.tab('work'), 'release');
  assert.equal(Views.tab('traffic'), 'traffic');
  assert.equal(Views.title('recording'), 'Recording activity');
  assert.equal(Views.title('recorder'), 'Recorder & storage');
});

test('three places hold every page, each page in exactly one', () => {
  assert.deepEqual(
    Views.places.map((place) => place.label),
    ['Work', 'Monitor', 'System'],
  );
  const pages = Views.places.flatMap((place) => Views.pages(place.place, 'acme-web'));
  assert.equal(new Set(pages.map((page) => page.tab)).size, pages.length);
  for (const place of Views.places) {
    for (const page of Views.pages(place.place, 'acme-web')) {
      assert.equal(Views.place(page.tab), place.place, page.tab);
    }
    // Each place opens where its name leads: Work on Projects, the others
    // on their first page.
    assert.equal(
      Views.of(place.href),
      place.place === 'work' ? 'projects' : Views.pages(place.place)[0]?.tab,
    );
  }
  assert.equal(Views.place(Views.tab('turn')), 'monitor');
  assert.equal(Views.place(Views.tab('usage')), 'monitor');
  assert.equal(Views.place(Views.tab('work')), 'work');
  assert.equal(Views.place('recorder'), 'system');
});

test("Work's own home has no pages; inside a project, the row holds the project's", () => {
  assert.deepEqual(Views.pages('work'), []);
  assert.deepEqual(
    Views.pages('work', 'acme web').map((page) => [page.tab, page.href]),
    [
      ['sessions', '/projects/acme%20web/sessions'],
      ['release', '/projects/acme%20web/release'],
    ],
  );
});

test('only the pages that read a project carry it between them', () => {
  const traffic = { tab: 'traffic', href: '/traffic', label: 'Traffic' } as const;
  const live = { tab: 'live', href: '/live', label: 'Live' } as const;
  const tools = { tab: 'tools', href: '/tools', label: 'Tools' } as const;
  assert.equal(Views.scopedHref(traffic, 'acme-web'), '/traffic?project=acme-web');
  assert.equal(Views.scopedHref(traffic, null), '/traffic');
  // Monitor's pages read a project; System's do not.
  assert.equal(Views.scopedHref(live, 'acme-web'), '/live?project=acme-web');
  assert.equal(Views.scopedHref(tools, 'acme-web'), '/tools');
  assert.ok(Views.scoped('recordings'));
  assert.ok(Views.scoped('search'));
  assert.equal(Views.scoped('usage'), false);
});
