import test from 'node:test';
import assert from 'node:assert/strict';
import { RecordingList, TurnContexts, type RecordingListInput } from '../src/lib/recordings.ts';
import { Board } from '../src/lib/board.ts';
import type { RecordingId, RecordingView, UnixMillis } from '../src/lib/api.ts';
import { callFixture, callId, uuid } from './fixtures.ts';

const DAY = 86_400_000;
const NOW = 100 * DAY + 12 * 3_600_000;
// Days start at UTC midnight here, so the test does not depend on the zone.
const dayStart = (ms: number) => Math.floor(ms / DAY) * DAY;
const sid = (n: number) => uuid(300 + n) as RecordingId;

function recording(
  n: number,
  lastSeen: number,
  overrides: Partial<RecordingView> = {},
): RecordingView {
  return {
    id: sid(n),
    provider: 'anthropic',
    source: 'claude_code_headers',
    native_id: `native-${n}` as RecordingView['native_id'],
    first_seen: (lastSeen - 60_000) as UnixMillis,
    last_seen: lastSeen as UnixMillis,
    call_count: 10 * n,
    turn_count: n,
    error_count: 0,
    tokens: { input: 1000 * n, output: 10 * n, cache_read: null },
    archived_at: null,
    ...overrides,
  };
}
const input = (overrides: Partial<RecordingListInput>): RecordingListInput => ({
  recordings: [],
  details: new Map(),
  open: [],
  now: NOW,
  query: '',
  dayStart,
  ...overrides,
});

test('recordings group by the day they were last seen, newest first, and checks fold away', () => {
  const list = new RecordingList(
    input({
      recordings: [
        recording(1, NOW - 2 * DAY),
        recording(2, NOW - DAY),
        recording(3, NOW - 60_000),
        recording(4, NOW - 2 * 60_000),
        // One call, no turn: a client's own check.
        recording(5, NOW - 30_000, { call_count: 1, turn_count: 0 }),
      ],
    }),
  );
  assert.deepEqual(
    list.groups.map((group) => [group.label, group.rows.map((row) => row.recording.id)]),
    [
      ['Today', [sid(3), sid(4)]],
      ['Yesterday', [sid(2)]],
      [RecordingList.dayLabel(dayStart(NOW - 2 * DAY), NOW, dayStart), [sid(1)]],
    ],
  );
  assert.deepEqual(
    list.folded.map((row) => row.recording.id),
    [sid(5)],
  );
  // Sizes are against the largest in the list.
  const largest = list.groups.flatMap((group) => group.rows).find((row) => row.requests === 1);
  assert.equal(largest?.recording.id, sid(4));
});

test('a row says who it is, what it is about, and what it does now', () => {
  const list = new RecordingList(
    input({
      recordings: [
        recording(1, NOW - 60_000),
        recording(2, NOW - 3_600_000),
        recording(3, NOW - 5_000),
      ],
      details: new Map([
        [
          sid(1),
          {
            prompt: { text: 'Fix the flaky test', omitted_chars: 0 },
            workdir: '/home/me/dev/doppel',
          },
        ],
      ]),
      open: [
        callFixture(callId(1), { recording: sid(3), started_at: (NOW - 4_000) as UnixMillis }),
      ],
    }),
  );
  const rows = new Map(
    list.groups.flatMap((group) => group.rows).map((row) => [row.recording.id, row]),
  );
  const first = rows.get(sid(1));
  assert.equal(first?.client, 'Claude Code');
  assert.equal(first?.detail?.prompt?.text, 'Fix the flaky test');
  assert.equal(Board.folder(first?.detail?.workdir ?? ''), 'doppel');
  assert.deepEqual(first?.now, { kind: 'recent', at: NOW - 60_000 });
  assert.deepEqual(rows.get(sid(2))?.now, { kind: 'quiet', at: NOW - 3_600_000 });
  const busy = rows.get(sid(3))?.now;
  assert.equal(busy?.kind === 'open' && busy.mark, 'waiting');
});

test('search reads the prompt and folder as well as the IDs', () => {
  const recordings = [recording(1, NOW - 60_000), recording(2, NOW - 60_000)];
  const details = new Map([[sid(2), { prompt: null, workdir: '/srv/ren' }]]);
  const ids = (query: string) =>
    new RecordingList(input({ recordings, details, query })).groups
      .flatMap((group) => group.rows)
      .map((row) => row.recording.id);
  assert.deepEqual(ids('ren'), [sid(2)]);
  assert.deepEqual(ids('NATIVE-1'), [sid(1)]);
  assert.deepEqual(ids('claude code'), [sid(1), sid(2)]);
  assert.deepEqual(ids('nothing'), []);
});

test('the list adds up its recordings, and each row knows its span within its day', () => {
  const list = new RecordingList(
    input({
      recordings: [
        recording(1, NOW - 60_000, { error_count: 2, first_seen: (NOW - 6 * 3_600_000) as never }),
        recording(2, NOW - 2 * 60_000, { source: 'codex_turn_metadata' as never }),
      ],
    }),
  );
  assert.deepEqual(
    { ...list.summary, clients: list.summary.clients.map((one) => [one.client, one.share]) },
    {
      recordings: 2,
      open: 0,
      requests: 30,
      errors: 2,
      tokensIn: 3000,
      clients: [
        [Board.clientName(Board.harness('codex_turn_metadata' as never)), 20 / 30],
        [Board.clientName(Board.harness('claude_code_headers' as never)), 10 / 30],
      ],
    },
  );
  const row = list.groups[0]?.rows.find((each) => each.recording.id === sid(1));
  // NOW is noon: six hours of it, from six until a minute before noon.
  assert.equal(row?.day.from, 0.25);
  assert.ok(row && Math.abs(row.day.to - (0.5 - 60_000 / DAY)) < 1e-9);
});

test('a report’s turns read as context ranges against its peak; a cold cache is a rebuild', () => {
  const turn = (start: number | null, end: number | null, hit: number | null, written: number) => ({
    turn: 't' as never,
    started_at: 0 as never,
    ended_at: null,
    call_count: 1,
    idle_before_ms: null,
    context_start_tokens: start,
    context_end_tokens: end,
    cache_hit_percent: hit,
    cache_written_tokens: written,
    output_tokens: 0,
  });
  const bars = TurnContexts.of([
    turn(10_000, 50_000, 90, 100),
    turn(50_000, 100_000, 10, 40_000),
    turn(null, null, null, 0),
  ]);
  // A turn that reported no context is left out; the rest stand against the
  // largest context any of them reached.
  assert.deepEqual(
    bars.map((bar) => [bar.start, bar.end, bar.hit, bar.rebuilt]),
    [
      [0.1, 0.5, 0.9, false],
      [0.5, 1, 0.1, true],
    ],
  );
});
