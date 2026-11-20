// A project's pull requests: the daemon's pinned answer decodes, and each
// state reads as a person would say it, GitHub's unknown text kept.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { PullRequests, PullsApi, type PullRequest, type Pulls } from '../src/lib/pulls.ts';
import type { UnknownLabel } from '../src/lib/decode.ts';

const pinned = readFileSync(new URL('./fixtures/api/project_pulls.json', import.meta.url), 'utf8');
const answering =
  (body: unknown): typeof fetch =>
  async () =>
    Response.json(body);

test('the pinned answer decodes', async () => {
  const pulls = await new PullsApi(answering(JSON.parse(pinned))).of('ren');
  assert.equal(pulls.watch, 'seen');
  const seen = PullRequests.sighting(pulls);
  assert.ok(seen?.current);
  const [pull] = seen.sighting.pull_requests;
  assert.ok(pull);
  assert.equal(pull.number, 12);
  assert.deepEqual(pull.work_items, ['00000000-0000-7000-8000-000000000001']);
  assert.deepEqual(PullRequests.merge(pull), ['behind main', 'warning']);
  assert.deepEqual(PullRequests.checks(pull.checks), ['checks running', 'warning']);
});

test('a failed poll keeps the last answer, not as current', async () => {
  const last = (JSON.parse(pinned) as { seen: unknown }).seen;
  const pulls = await new PullsApi(
    answering({
      api_version: 1,
      watch: 'failed',
      repository: 'acme/ren',
      error: 'GitHub answered 401 Unauthorized',
      at: 1_800_000_060_000,
      last,
    }),
  ).of('ren');
  const seen = PullRequests.sighting(pulls);
  assert.equal(seen?.current, false);
  assert.equal(seen?.sighting.pull_requests.length, 1);
});

test('states a newer daemon sends are kept', async () => {
  const pulls: Pulls = await new PullsApi(answering({ api_version: 1, watch: 'paused' })).of('ren');
  assert.deepEqual(pulls, { watch: 'unknown', text: 'paused' });
  assert.equal(PullRequests.sighting(pulls), null);
  const merge = 'SOMETHING_NEW' as UnknownLabel;
  const pull = { merge, base_branch: 'main' } as PullRequest;
  assert.deepEqual(PullRequests.merge(pull), ['merge: SOMETHING_NEW', 'warning']);
  assert.deepEqual(PullRequests.checkState('COMPLETED/NEW' as UnknownLabel), [
    'COMPLETED/NEW',
    'warning',
  ]);
});

test('nothing worth saying is said', () => {
  const pull = { merge: 'clean', base_branch: 'main' } as PullRequest;
  assert.equal(PullRequests.merge(pull), null);
  assert.equal(PullRequests.review('not_required'), null);
  assert.deepEqual(PullRequests.review('changes_requested'), ['changes requested', 'bad']);
});
