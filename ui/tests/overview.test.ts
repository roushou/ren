import test from 'node:test';
import assert from 'node:assert/strict';
import { activeSessions, ago, healthLine, problems } from '../src/lib/overview.ts';
import type { Health, SessionView, Storage } from '../src/lib/api.ts';
import {
  callId,
  finishedFixture,
  healthFixture,
  recorderFixture,
  storageFixture,
} from './fixtures.ts';

const labels = (items: { label: string; tone: string }[]) =>
  items.map((item) => [item.label, item.tone]);
const storage = (used: number, budget: number, enabled = true): Storage => {
  const fixture = storageFixture();
  return {
    policy: { ...fixture.policy, enabled, max_storage_bytes: budget },
    usage: {
      ...fixture.usage,
      disk: {
        metadata: { page_size: 1, page_count: used, freelist_count: 0 },
        blobs: { page_size: 1, page_count: 0, freelist_count: 0 },
      },
    },
  };
};

test('a healthy daemon is one green line', () => {
  const health: Health = {
    ...healthFixture(),
    operator: { pause: { state: 'running' }, blocked_sessions: 0 },
  };
  assert.deepEqual(labels(healthLine(health, storage(100, 1000), null)), [
    ['Daemon reachable', 'success'],
    ['Recording', 'success'],
    ['Admitting calls', 'success'],
    ['100 B of 1000 B', 'success'],
  ]);
});

test('each problem says what is wrong, and where to look', () => {
  const health: Health = {
    ...healthFixture({
      omissions: { ...recorderFixture().omissions, unrecorded_calls: 2 },
    }),
    operator: { pause: { state: 'paused', retry_after: 30 }, blocked_sessions: 2 },
  };
  const line = healthLine(health, storage(950, 1000), null);
  assert.deepEqual(labels(line), [
    ['Daemon reachable', 'success'],
    ['Capture loss', 'warning'],
    ['Paused · retry in 30 s', 'warning'],
    ['2 sessions blocked', 'warning'],
    ['950 B of 1000 B', 'error'],
  ]);
  const failing = healthLine(healthFixture({ storage_errors: 1 }), null, null);
  assert.deepEqual(failing[1], {
    label: 'Recording failing',
    tone: 'error',
    title: 'Storage or retention errors since the daemon started',
    href: '/recorder',
  });
  // An older daemon reports no operator state: nothing is claimed about it.
  assert.equal(healthLine(healthFixture(), null, null).length, 2);
  assert.deepEqual(labels(healthLine(null, null, 'Cannot reach TGV.')), [
    ["Can't reach TGV", 'error'],
  ]);
  assert.deepEqual(labels(healthLine(healthFixture(), storage(5, 0, false), null)).at(-1), [
    '5 B stored',
    'neutral',
  ]);
});

test('problems are newest first, each call once', () => {
  const old = finishedFixture(callId(1), { status: 429, started_at: 1_000 as never });
  const cut = finishedFixture(callId(2), { terminal: 'interrupted', started_at: 3_000 as never });
  const both = finishedFixture(callId(3), {
    status: 500,
    terminal: 'interrupted',
    started_at: 2_000 as never,
  });
  const listed = problems([old, both], [cut, both]);
  assert.deepEqual(
    listed.map((p) => [p.call.id, p.reason.label]),
    [
      [cut.id, 'Interrupted'],
      [both.id, 'Error 500'],
      [old.id, 'Error 429'],
    ],
  );
  assert.equal(problems([old, cut, both], [], 2).length, 2);
});

test('active sessions and relative times', () => {
  const now = 10_000_000;
  const session = (id: number, last: number) =>
    ({ id: `s${id}`, last_seen: last }) as unknown as SessionView;
  assert.deepEqual(
    activeSessions(
      [session(1, now - 20 * 60_000), session(2, now - 60_000), session(3, now)],
      now,
    ).map((s) => s.id),
    ['s3', 's2'],
  );
  assert.deepEqual(
    [now, now - 30_000, now - 12 * 60_000, now - 3 * 3_600_000].map((t) => ago(t, now)),
    ['just now', '30 s ago', '12 min ago', '3 h ago'],
  );
});
