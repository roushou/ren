import test from 'node:test';
import assert from 'node:assert/strict';
import {
  ago,
  Attention,
  Day,
  healthLine,
  Verdict,
  type OverviewInput,
} from '../src/lib/overview.ts';
import type { Health, Storage } from '../src/lib/api.ts';
import {
  callId,
  finishedFixture,
  healthFixture,
  recorderFixture,
  storageFixture,
  uuid,
} from './fixtures.ts';

const labels = (items: { label: string; tone: string }[]) =>
  items.map((item) => [item.label, item.tone]);
const storage = (used: number, budget: number, enabled = true): Storage => {
  const fixture = storageFixture();
  return {
    ...fixture,
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
  assert.deepEqual(labels(healthLine(null, null, 'Cannot reach TGV.')), [
    ["Can't reach TGV", 'error'],
  ]);
  assert.deepEqual(labels(healthLine(healthFixture(), storage(5, 0, false), null)).at(-1), [
    '5 B stored',
    'neutral',
  ]);
});

test('relative times', () => {
  const now = 10_000_000;
  assert.deepEqual(
    [now, now - 30_000, now - 12 * 60_000, now - 3 * 3_600_000].map((t) => ago(t, now)),
    ['just now', '30 s ago', '12 min ago', '3 h ago'],
  );
});

const NOW = 100 * 3_600_000;
const at = (hoursAgo: number) => (NOW - hoursAgo * 3_600_000) as never;
const exposure = (n: number, session: number, kind: 'openai_key' | 'github_token') => ({
  call: callId(n),
  session: uuid(session) as never,
  turn: null,
  client: null,
  started_at: at(n),
  field: 'prompt' as const,
  kind,
  hint: 'sk-…abcd',
  fingerprint: '0123456789abcdef',
  count: 1,
});
const input = (overrides: Partial<OverviewInput>): OverviewInput => ({
  now: NOW,
  health: healthFixture(),
  failed: [],
  stopped: [],
  bookkeeping: [],
  exposures: [],
  ...overrides,
});

test('attention ranks secrets, then failed work, then cut-off work, then bookkeeping', () => {
  const probe = (n: number, hoursAgo: number) =>
    finishedFixture(callId(10 + n), { status: 429, started_at: at(hoursAgo) });
  const attention = new Attention(
    input({
      bookkeeping: [probe(1, 1), probe(2, 3), probe(3, 9)],
      stopped: [finishedFixture(callId(20), { terminal: 'interrupted', started_at: at(1) })],
      failed: [finishedFixture(callId(30), { status: 500, started_at: at(2), model: 'm-a' })],
      exposures: [exposure(1, 9, 'openai_key'), exposure(2, 9, 'openai_key')],
    }),
  );
  assert.deepEqual(
    attention.groups.map((group) => [group.severity, group.title]),
    [
      ['error', '2 secrets exposed'],
      ['error', 'Anthropic Error 500 ×1'],
      ['warning', 'Anthropic Interrupted ×1'],
      ['neutral', 'Anthropic Error 429 ×3'],
    ],
  );
  assert.equal(attention.worst, 'error');
  const [secrets, , , probes] = attention.groups;
  // One session: the secrets link to its report, and say what and where.
  assert.equal(secrets?.href, `/sessions/${uuid(9)}/report`);
  assert.match(secrets?.detail ?? '', /OpenAI API key in prompt · 1 session · rotate them/);
  // Repeats fold into one line, with when each happened along the day.
  assert.deepEqual(
    probes?.marks.map((mark) => Math.round(mark * 24)),
    [23, 21, 15],
  );
  assert.equal(probes?.href, '/traffic?status=errors&kind=other&since=24h');
  assert.match(probes?.detail ?? '', /not your agents' work/);
});

test('bytes past the capture limit are noted, never raised; lost capture is', () => {
  const omissions = recorderFixture().omissions;
  const policy = new Attention(
    input({ health: healthFixture({ omissions: { ...omissions, policy_limit_bytes: 2048 } }) }),
  );
  assert.deepEqual(
    policy.groups.map((group) => [group.severity, group.title]),
    [['neutral', '2.0 KiB over the capture limit']],
  );
  const lost = new Attention(
    input({ health: healthFixture({ omissions: { ...omissions, queue_bytes_bytes: 1024 } }) }),
  );
  assert.deepEqual(
    lost.groups.map((group) => group.severity),
    ['warning'],
  );
  assert.equal(new Attention(input({})).worst, null);
});

test('the day counts calls against the day before, and splits input into cached and fresh', () => {
  const hour = 3_600_000;
  const usage = (calls: number) => ({
    totals: {
      calls,
      errors: 0,
      tokens: { input: 1000, output: 50, cache_read: 900 },
    },
    hours: [
      {
        start: (Math.floor(NOW / hour) * hour - 2 * hour) as never,
        calls,
        errors: 0,
        tokens: { input: 1000, output: 50, cache_read: 900 },
      },
    ],
    models: [],
    users: [],
  });
  const day = new Day(usage(150), usage(100), NOW, { bookkeeping: 9, stopped: 1 });
  assert.deepEqual(
    day.tiles.map((tile) => [tile.label, tile.value, tile.detail]),
    [
      ['Calls', '150', '+50% vs the day before'],
      ['Failed work calls', '0', '9 failed bookkeeping · 1 cut off'],
      ['Tokens in', '1.0k', '900 cached · 100 fresh'],
      ['Tokens out', '50', `peak 50 at ${Day.clock(Math.floor(NOW / hour) * hour - 2 * hour)}`],
    ],
  );
  assert.deepEqual(day.input, { cached: 900, fresh: 100 });
  assert.equal(Day.change(5, null), 'no earlier day read');
  assert.equal(Day.change(0, 0), 'none the day before either');
  assert.equal(Day.change(544, 7), '7 the day before');
  // Bars stack cached under fresh, on the same time scale as the ticks.
  const bar = day.bars(100).find((candidate) => candidate.cached > 0);
  assert.equal(bar?.cached, 90);
  assert.equal(bar?.fresh, 10);
  assert.equal(day.x(day.span.from), 0);
  assert.equal(day.x(NOW + 10 * hour), 1000);
  assert.equal(Day.spark([0, 0], 22), '0.0,21.0 100.0,21.0');
});

test('the verdict takes its tone from the worst real problem, never from bookkeeping', () => {
  const healthy = healthFixture();
  const items = healthLine(healthy, null, null);
  const probes = new Attention(
    input({ bookkeeping: [finishedFixture(callId(40), { status: 429, started_at: at(1) })] }),
  );
  const calm = new Verdict({
    health: healthy,
    items,
    attention: probes,
    active: 2,
    inFlight: 0,
    lastCall: NOW - 30_000,
    now: NOW,
  });
  assert.equal(calm.tone, 'success');
  assert.equal(calm.headline, 'Recording normally');
  assert.deepEqual(calm.facts, ['2 sessions active', 'nothing in flight, last call 30 s ago']);
  assert.deepEqual(calm.problems, []);
  const secrets = new Verdict({
    health: healthy,
    items,
    attention: new Attention(input({ exposures: [exposure(1, 9, 'openai_key')] })),
    active: 0,
    inFlight: 3,
    lastCall: 0,
    now: NOW,
  });
  assert.equal(secrets.tone, 'error');
  assert.deepEqual(secrets.facts, ['no session active', '3 in flight']);
  assert.deepEqual(
    secrets.problems.map((problem) => problem.title),
    ['1 secret exposed'],
  );
  assert.equal(Verdict.headline(null), "Can't reach TGV");
  assert.equal(Verdict.headline(healthFixture({ storage_errors: 1 })), 'Recording failing');
});
