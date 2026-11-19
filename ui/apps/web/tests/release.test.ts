import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { ManagementApi, type ReleaseView } from '../src/lib/api.ts';
import { ReleaseLine } from '../src/lib/release.ts';

const pinned: typeof fetch = async () =>
  Response.json(JSON.parse(readFileSync(new URL('./fixtures/api/release.json', import.meta.url), 'utf8')));

test("the daemon's release decodes and reads as one status line", async () => {
  const release = await new ManagementApi(pinned).release();
  assert.ok(release);
  const line = ReleaseLine.of(release);
  assert.equal(line.text, 'Production 1 behind main · canary passed');
  assert.equal(line.tone, 'warning');
  assert.match(line.detail, /^main 62e9e9e00000\nproduction 07f4bb7c1e2a\ncanary 62e9e9e00000\n  doctor passed/);
  assert.match(line.detail, /promotion \(manual\): passed; waiting for the operator to promote it$/);
});

test('a failed canary and an unknown decision read as such', () => {
  const release: ReleaseView = {
    main: '62e9e9e0000000000000000000000000000000ff',
    production: { kind: 'commit', commit: '62e9e9e0000000000000000000000000000000ff', tree: 'clean' },
    behind: 0,
    canary: {
      commit: '62e9e9e0000000000000000000000000000000ff',
      passed: false,
      checks: [{ check: 'pages', name: null, passed: false, detail: 'the sessions page did not load' }],
    },
    policy: 'automatic',
    decision: 'failed',
  };
  const line = ReleaseLine.of(release);
  assert.equal(line.text, 'Production on main · canary failed');
  assert.equal(line.tone, 'bad');
  assert.match(line.detail, /pages failed: the sessions page did not load/);
  const later = ReleaseLine.of({ ...release, decision: 'held_for_review' as ReleaseView['decision'] });
  assert.equal(later.tone, 'warning');
  assert.match(later.detail, /held_for_review$/);
});
