import test from 'node:test';
import assert from 'node:assert/strict';
import { GitHubWatches } from '../src/lib/integrations.ts';
import type { Pull, Pulls } from '../src/lib/api.ts';

const pull = (number: number, overrides: Partial<Pull> = {}): Pull => ({
  number,
  title: `pull ${number}`,
  url: `https://github.example/${number}`,
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
const seen = (repository: string, pulls: Pull[]): Pulls => ({
  watch: 'seen',
  repository,
  seen: { at: 1000, default_branch: null, pull_requests: pulls, truncated: false },
});

test('each watch counts its pull requests by where they stand', () => {
  const rows = GitHubWatches.rows([
    {
      project: 'acme-api',
      watch: seen('acme/api', [
        pull(1),
        pull(2, { checks: 'failing' }),
        pull(3, { review: 'review_required', merge: 'blocked' }),
        pull(4, { readiness: 'draft' }),
      ]),
    },
    {
      project: 'acme-docs',
      watch: { watch: 'failed', repository: 'acme/docs', error: 'timeout', at: 5, last: null },
    },
    { project: 'acme-infra', watch: { watch: 'off' } },
    { project: 'acme-web', watch: null },
  ]);
  assert.deepEqual(
    rows.map((row) => [row.project, row.mark, row.repository, row.pulls.length]),
    [
      ['acme-api', 'passed', 'acme/api', 4],
      ['acme-docs', 'failed', 'acme/docs', 0],
      ['acme-infra', 'idle', null, 0],
      ['acme-web', 'failed', null, 0],
    ],
  );
  assert.deepEqual(rows[0]?.standings, {
    ready: 1,
    review: 1,
    failing: 1,
    running: 0,
    draft: 1,
    other: 0,
  });
  assert.equal(rows[1]?.error, 'timeout');
  const summary = GitHubWatches.summary(rows);
  assert.deepEqual([summary.watched, summary.failing, summary.open, summary.ready], [3, 2, 4, 1]);
  assert.deepEqual(
    summary.checksFailing.map((row) => row.pull.number),
    [2],
  );
  // Failing first, then waiting on review, then ready, then drafts.
  assert.deepEqual(
    GitHubWatches.pulls(rows).map((row) => row.pull.number),
    [2, 3, 1, 4],
  );
});
