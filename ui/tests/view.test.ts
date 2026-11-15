import test from 'node:test';
import assert from 'node:assert/strict';
import { Views } from '../src/lib/view.ts';

test('each route is its view, and an unknown one is Traffic', () => {
  assert.equal(Views.of('/'), 'overview');
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
  assert.equal(Views.tab('traffic'), 'traffic');
  assert.equal(Views.title('recording'), 'Recording activity');
  assert.equal(Views.title('recorder'), 'Recorder & storage');
});
