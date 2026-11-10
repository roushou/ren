import test from 'node:test';
import assert from 'node:assert/strict';
import { SessionList, type SessionListInput } from '../src/lib/sessions.ts';
import { Board } from '../src/lib/board.ts';
import type { SessionId, SessionView, UnixMillis } from '../src/lib/api.ts';
import { callFixture, callId, uuid } from './fixtures.ts';

const DAY = 86_400_000;
const NOW = 100 * DAY + 12 * 3_600_000;
// Days start at UTC midnight here, so the test does not depend on the zone.
const dayStart = (ms: number) => Math.floor(ms / DAY) * DAY;
const sid = (n: number) => uuid(300 + n) as SessionId;

function session(n: number, lastSeen: number, overrides: Partial<SessionView> = {}): SessionView {
  return {
    id: sid(n),
    provider: 'anthropic',
    source: 'claude_code_headers',
    native_id: `native-${n}` as SessionView['native_id'],
    first_seen: (lastSeen - 60_000) as UnixMillis,
    last_seen: lastSeen as UnixMillis,
    call_count: 10 * n,
    turn_count: n,
    error_count: 0,
    tokens: { input: 1000 * n, output: 10 * n, cache_read: null },
    ...overrides,
  };
}
const input = (overrides: Partial<SessionListInput>): SessionListInput => ({
  sessions: [],
  details: new Map(),
  open: [],
  now: NOW,
  query: '',
  dayStart,
  ...overrides,
});

test('sessions group by the day they were last seen, newest first, and checks fold away', () => {
  const list = new SessionList(
    input({
      sessions: [
        session(1, NOW - 2 * DAY),
        session(2, NOW - DAY),
        session(3, NOW - 60_000),
        session(4, NOW - 2 * 60_000),
        // One call, no turn: a client's own check.
        session(5, NOW - 30_000, { call_count: 1, turn_count: 0 }),
      ],
    }),
  );
  assert.deepEqual(
    list.groups.map((group) => [group.label, group.rows.map((row) => row.session.id)]),
    [
      ['Today', [sid(3), sid(4)]],
      ['Yesterday', [sid(2)]],
      [SessionList.dayLabel(dayStart(NOW - 2 * DAY), NOW, dayStart), [sid(1)]],
    ],
  );
  assert.deepEqual(
    list.folded.map((row) => row.session.id),
    [sid(5)],
  );
  // Sizes are against the largest in the list.
  const largest = list.groups.flatMap((group) => group.rows).find((row) => row.requests === 1);
  assert.equal(largest?.session.id, sid(4));
});

test('a row says who it is, what it is about, and what it does now', () => {
  const list = new SessionList(
    input({
      sessions: [session(1, NOW - 60_000), session(2, NOW - 3_600_000), session(3, NOW - 5_000)],
      details: new Map([
        [
          sid(1),
          {
            prompt: { text: 'Fix the flaky test', omitted_chars: 0 },
            workdir: '/home/me/dev/doppel',
          },
        ],
      ]),
      open: [callFixture(callId(1), { session: sid(3), started_at: (NOW - 4_000) as UnixMillis })],
    }),
  );
  const rows = new Map(
    list.groups.flatMap((group) => group.rows).map((row) => [row.session.id, row]),
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
  const sessions = [session(1, NOW - 60_000), session(2, NOW - 60_000)];
  const details = new Map([[sid(2), { prompt: null, workdir: '/srv/ren' }]]);
  const ids = (query: string) =>
    new SessionList(input({ sessions, details, query })).groups
      .flatMap((group) => group.rows)
      .map((row) => row.session.id);
  assert.deepEqual(ids('ren'), [sid(2)]);
  assert.deepEqual(ids('NATIVE-1'), [sid(1)]);
  assert.deepEqual(ids('claude code'), [sid(1), sid(2)]);
  assert.deepEqual(ids('nothing'), []);
});
