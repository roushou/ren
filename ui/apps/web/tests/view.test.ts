import test from 'node:test';
import assert from 'node:assert/strict';
import { Views } from '../src/lib/view.ts';

test('each route is its view, and an unknown one is Traffic', () => {
  assert.equal(Views.of('/'), 'projects');
  assert.equal(Views.of('/projects/[name]'), 'project');
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
  assert.equal(Views.tab('project'), 'projects');
  assert.equal(Views.tab('traffic'), 'traffic');
  assert.equal(Views.title('recording'), 'Recording activity');
  assert.equal(Views.title('recorder'), 'Recorder & storage');
});

test('three destinations hold every page, each page in exactly one', () => {
  assert.deepEqual(
    Views.destinations.map((place) => place.label),
    ['Projects', 'Inspect', 'Host'],
  );
  const pages = Views.destinations.flatMap((place) => Views.pages(place.destination));
  assert.equal(new Set(pages.map((page) => page.tab)).size, pages.length);
  for (const place of Views.destinations) {
    for (const page of Views.pages(place.destination)) {
      assert.equal(Views.destination(page.tab), place.destination, page.tab);
    }
    // Each destination opens on its first page.
    assert.equal(Views.pages(place.destination)[0]?.href, place.href);
  }
  assert.equal(Views.destination(Views.tab('turn')), 'inspect');
  assert.equal(Views.destination(Views.tab('project')), 'projects');
});
