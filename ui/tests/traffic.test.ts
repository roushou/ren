import test from 'node:test';
import assert from 'node:assert/strict';
import {
  emptyFilters,
  trafficParams,
  selectedCall,
  turnGroups,
  neighbour,
  listingQuery,
  sinceStart,
  parseSince,
  filtersFromParams,
} from '../src/lib/traffic.ts';
import { callFixture, finishedFixture, callId } from './fixtures.ts';

test('empty traffic, failed initial loads, and dismissed selections have no inspector', () => {
  assert.equal(selectedCall(undefined, []), undefined);
  assert.equal(selectedCall(null, []), undefined);
  assert.equal(selectedCall(callId(404), []), undefined);
  const previous = finishedFixture(callId(1));
  assert.equal(selectedCall(null, [previous], previous, previous), undefined);
  assert.equal(selectedCall(undefined, [], previous, previous), undefined);
});

test('selection retains off-page calls and finished live observations for the matching ID', () => {
  const open = callFixture(callId(1));
  const finished = finishedFixture(callId(1));
  const other = callFixture(callId(2));
  assert.equal(selectedCall(callId(1), [open], other), open);
  assert.equal(selectedCall(callId(1), [open], finished), finished);
  // An open observation never replaces the listed row.
  assert.equal(selectedCall(callId(1), [finished], open), finished);
  assert.equal(selectedCall(callId(1), [], other, open), open);
  assert.equal(selectedCall(callId(1), [], other, other), undefined);
});

test('traffic filters encode literals and local date bounds, and reject invalid ranges', () => {
  const filters = {
    ...emptyFilters(),
    q: ' a&b%_ ',
    provider: 'openai',
    has_tools: true,
    from: '2026-11-03T10:00',
    until: '2026-11-03T11:00',
  };
  const params = new URLSearchParams(trafficParams(filters));
  assert.equal(params.get('q'), 'a&b%_');
  assert.equal(params.get('provider'), 'openai');
  assert.equal(params.get('has_tools'), 'true');
  assert.equal(params.get('model'), null);
  assert.equal(Number(params.get('from_ms')), new Date(filters.from).getTime());
  assert.throws(() => trafficParams({ ...filters, until: '2026-11-03T09:00' }));
  assert.throws(() => trafficParams({ ...filters, from: 'invalid' }));
  assert.equal(trafficParams(emptyFilters()), '');
});

test('URL filters restore datetime-local values without changing their absolute timestamps', async () => {
  const { filtersFromParams } = await import('../src/lib/traffic.ts');
  const original = {
    ...emptyFilters(),
    q: 'a & b',
    has_tools: true,
    from: '2026-11-03T10:00',
    until: '2026-11-03T11:00',
  };
  assert.equal(
    trafficParams(filtersFromParams(new URLSearchParams(trafficParams(original)))),
    trafficParams(original),
  );
  assert.equal(filtersFromParams(new URLSearchParams('from_ms=invalid')).from, '');
});

test('runs of calls outside a turn share one group; turns keep theirs', () => {
  const turn = (n: number) => `00000000-0000-4000-8000-${String(n).padStart(12, '0')}` as never;
  const calls = [
    finishedFixture(callId(1)),
    finishedFixture(callId(2)),
    finishedFixture(callId(3), { turn: turn(7) }),
    finishedFixture(callId(4), { turn: turn(7) }),
    finishedFixture(callId(5)),
  ];
  assert.deepEqual(
    turnGroups(calls).map((g) => [g.label, g.rows.length]),
    [
      ['Outside a turn', 2],
      ['Turn 00000000', 2],
      ['Outside a turn', 1],
    ],
  );
  // A turn that resumes after an outside call is a new group with its own key.
  const resumed = turnGroups([...calls, finishedFixture(callId(6), { turn: turn(7) })]);
  assert.equal(new Set(resumed.map((g) => g.id)).size, resumed.length);
});

test('j and k step through calls and stop at the ends', () => {
  const calls = [1, 2, 3].map((n) => finishedFixture(callId(n)));
  assert.equal(neighbour(calls, calls[0].id, 1)?.id, calls[1].id);
  assert.equal(neighbour(calls, calls[2].id, 1)?.id, calls[2].id);
  assert.equal(neighbour(calls, calls[0].id, -1)?.id, calls[0].id);
  assert.equal(neighbour(calls, null, 1)?.id, calls[0].id);
  assert.equal(neighbour([], null, 1), undefined);
});

test('the listing defaults to model calls and resolves since against now', () => {
  const now = Date.UTC(2026, 10, 6, 18, 0);
  const keys = ['q', 'session', 'client', 'status', 'from_ms', 'after'] as const;
  const query = (search: string) =>
    Object.fromEntries(listingQuery(new URLSearchParams(search), keys, now));
  assert.deepEqual(query(''), { kind: 'model' });
  assert.deepEqual(query('kind=all&client=codex'), { client: 'codex' });
  assert.deepEqual(query('kind=other'), { kind: 'other' });
  assert.deepEqual(query('kind=nonsense'), { kind: 'model' });
  assert.deepEqual(query('since=1h&session=abc'), {
    session: 'abc',
    kind: 'model',
    from_ms: String(now - 3_600_000),
  });
  // An unknown `since` is ignored rather than guessed at.
  assert.deepEqual(query('since=year'), { kind: 'model' });
  assert.equal(sinceStart('24h', now), now - 86_400_000);
  assert.equal(parseSince('today'), 'today');
});

test('narrowing filters round-trip through the URL', () => {
  const filters = {
    ...emptyFilters(),
    session: '33333333-3333-4333-8333-333333333333',
    client: 'claude_code',
    kind: 'all' as const,
    since: 'today',
  };
  const params = trafficParams(filters);
  assert.equal(
    params,
    'session=33333333-3333-4333-8333-333333333333&client=claude_code&since=today&kind=all',
  );
  const back = filtersFromParams(new URLSearchParams(params));
  assert.deepEqual(
    [back.session, back.client, back.kind, back.since],
    [filters.session, filters.client, 'all', 'today'],
  );
  // The default kind stays out of the URL.
  assert.equal(trafficParams(emptyFilters()), '');
});
