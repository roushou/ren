import test from 'node:test';
import assert from 'node:assert/strict';
import { GitHubWatches } from '../src/lib/github.ts';
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
  assert.deepEqual(rows[0]?.groups, {
    ready: 1,
    review: 1,
    held: 0,
    failing: 1,
    running: 0,
    draft: 1,
    unknown: 0,
  });
  assert.equal(rows[1]?.error, 'timeout');
  const summary = GitHubWatches.summary(rows);
  assert.deepEqual([summary.watched, summary.failing, summary.open, summary.ready], [3, 2, 4, 1]);
  assert.equal(summary.checksFailing, 1);
  // Failing first, then waiting on review, then ready, then drafts.
  assert.deepEqual(
    GitHubWatches.pulls(rows).map((row) => row.pull.number),
    [2, 3, 1, 4],
  );
});

test('a pull request is grouped by where it stands, never by its glyph', () => {
  const group = (overrides: Partial<Pull>) => GitHubWatches.group(pull(1, overrides));
  assert.equal(group({ merge: 'conflicting' }), 'failing');
  assert.equal(group({ review: 'changes_requested' }), 'held');
  assert.equal(group({ merge: 'behind' }), 'held');
  assert.equal(group({ merge: 'blocked' }), 'held');
  assert.equal(group({ merge: 'unknown' as never }), 'unknown');
  // A draft stands as a draft whatever its checks say.
  assert.equal(group({ readiness: 'draft', checks: 'failing' }), 'draft');
  // A watch this UI does not know is unknown, not idle.
  assert.equal(GitHubWatches.mark('unknown'), 'unknown');
  assert.equal(GitHubWatches.mark('off'), 'idle');
});
