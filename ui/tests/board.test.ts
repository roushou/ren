import test from 'node:test';
import assert from 'node:assert/strict';
import { Band, Board, Feed, LivePlace, type BoardInput, type LiveView } from '../src/lib/board.ts';
import { LiveReader } from '../src/lib/loaders.ts';
import {
  type CallActivity,
  type CallView,
  type SessionView,
  type TurnActivity,
  type TurnId,
  type UnixMillis,
} from '../src/lib/api.ts';
import { callFixture, callId, finishedFixture, uuid, versioned } from './fixtures.ts';

const ms = (value: number) => value as UnixMillis;
const sid = (n: number) => uuid(100 + n) as SessionView['id'];
const tid = (n: number) => uuid(200 + n) as TurnId;
const NOW = 10_000_000;
const WINDOW = LivePlace.windows['15m'].ms;

function session(n: number, overrides: Partial<SessionView> = {}): SessionView {
  return {
    id: sid(n),
    provider: 'anthropic',
    source: 'claude_code_headers',
    native_id: `native-${n}` as SessionView['native_id'],
    first_seen: ms(NOW - 60_000 + n),
    last_seen: ms(NOW - 1000),
    call_count: 1,
    turn_count: 1,
    error_count: 0,
    tokens: { input: null, output: null, cache_read: null },
    ...overrides,
  };
}
const open = (n: number, s: number, overrides: Partial<CallView> = {}) =>
  callFixture(callId(n), { session: sid(s), started_at: ms(NOW - 5000), ...overrides });
const done = (n: number, s: number, overrides: Partial<CallView> = {}) =>
  finishedFixture(callId(n), {
    session: sid(s),
    started_at: ms(NOW - 20_000),
    ended_at: ms(NOW - 10_000),
    outcome: 'completed',
    ...overrides,
  });
function activity(n: number, overrides: Partial<CallActivity> = {}): CallActivity {
  return {
    call: callId(n),
    started_at: ms(NOW - 20_000),
    ended_at: ms(NOW - 10_000),
    model: 'fixture-model',
    status: 200,
    outcome: 'completed',
    metrics: {
      context_tokens: null,
      cache_read_tokens: null,
      cache_written_tokens: null,
      output_tokens: null,
      request_bytes: null,
      duration_ms: null,
    },
    prompt: null,
    tool_calls: [],
    earlier_results: 0,
    reply: null,
    limitations: [],
    ...overrides,
  };
}
function turn(n: number, s: number, calls: CallActivity[], overrides: Partial<TurnActivity> = {}) {
  return {
    turn: tid(n),
    session: sid(s),
    native_id: `turn-${n}` as TurnActivity['native_id'],
    client: 'claude_code',
    started_at: ms(NOW - 30_000),
    ended_at: null,
    prompt: { text: 'Fix the flaky test', omitted_chars: 0 },
    files_read: [],
    files_written: [],
    commands: [],
    tool_calls: 0,
    failed_tool_calls: 0,
    unanswered_tool_calls: 0,
    findings: [],
    calls,
    omitted_calls: 0,
    ...overrides,
  } satisfies TurnActivity;
}
const board = (input: Partial<BoardInput>) =>
  new Board({
    sessions: [],
    calls: [],
    turns: new Map(),
    now: NOW,
    windowMs: WINDOW,
    focus: null,
    pins: [],
    only: null,
    ...input,
  });

test('the phase separates waiting for a head, streaming, the client turn, and endings', () => {
  assert.equal(Board.phase([open(1, 1)], NOW).kind, 'awaiting');
  assert.equal(Board.phase([open(1, 1, { status: 200 })], NOW).kind, 'responding');
  // A completed call that asked for tools hands the turn to the client: inferred.
  const client = Board.phase([done(1, 1, { tool_names: ['Bash'] })], NOW);
  assert.equal(client.kind, 'client');
  assert.match(Board.phaseChip(client, NOW).title, /inferred/);
  // A turn finished without tools hands it to the user, until the lane's span goes quiet.
  assert.equal(Board.phase([done(1, 1)], NOW).kind, 'yours');
  assert.match(Board.phaseChip(Board.phase([done(1, 1)], NOW), NOW).title, /inferred/);
  const long = ms(NOW - Board.laneMs - 1);
  assert.equal(Board.phase([done(1, 1, { started_at: long, ended_at: long })], NOW).kind, 'idle');
  // An unknown ending is never rounded up to idle or success.
  const unknown = Board.phase([done(1, 1, { outcome: 'unknown', state: 'recovered' })], NOW);
  assert.equal(unknown.kind, 'stopped');
  const limited = Board.phase([done(1, 1, { status: 429, outcome: null })], NOW);
  assert.equal(limited.kind === 'stopped' && limited.label, 'Error 429');
  // The newest open call wins over an older finished one.
  assert.equal(Board.phase([done(1, 1), open(2, 1)], NOW).kind, 'awaiting');
});

test('a slow response head turns its chip to a warning and raises an alert', () => {
  const quick = Board.phaseChip({ kind: 'awaiting', call: open(1, 1), since: NOW - 5000 }, NOW);
  assert.equal(quick.tone, 'live');
  const slow = board({
    sessions: [session(1)],
    calls: [open(1, 1, { started_at: ms(NOW - Board.slowHeadMs - 1000) })],
  });
  assert.equal(Board.phaseChip(slow.tiles[0].phase, NOW).tone, 'warning');
  assert.deepEqual(
    slow.alerts.map((alert) => alert.label),
    ['No response head'],
  );
});

test('tiles keep the order sessions first appeared in, whatever is busiest', () => {
  const sessions = [session(2), session(1), session(3, { last_seen: ms(NOW - WINDOW - 1) })];
  const first = board({ sessions, calls: [open(1, 2)] });
  const second = board({ sessions, calls: [open(1, 1)] });
  assert.deepEqual(
    first.tiles.map((tile) => tile.session.id),
    [sid(1), sid(2)],
  );
  assert.deepEqual(
    second.tiles.map((tile) => tile.session.id),
    first.tiles.map((tile) => tile.session.id),
  );
  // A session outside the window stays while a call is still open.
  const longCall = board({ sessions, calls: [open(1, 3)] });
  assert.ok(longCall.tiles.some((tile) => tile.session.id === sid(3)));
  assert.equal(longCall.inFlight, 1);
});

test('sessions beyond the tile limit are counted, not dropped silently', () => {
  const sessions = Array.from({ length: Board.maxTiles + 3 }, (_, n) => session(n));
  const full = board({ sessions });
  assert.equal(full.tiles.length, Board.maxTiles);
  assert.equal(full.overflow, 3);
});

test('the wall shows the busiest and most recent sessions, in the order they appeared', () => {
  const sessions = Array.from({ length: Board.maxTiles + 2 }, (_, n) =>
    // Session 0 appeared first but was seen longest ago.
    session(n, { last_seen: ms(NOW - 60_000 + n * 1000) }),
  );
  const full = board({ sessions, calls: [open(1, 0)] });
  const shown = full.tiles.map((tile) => tile.session.id);
  // Session 0 has an open call, so it stays; session 1 is the oldest idle one and leaves.
  assert.equal(shown[0], sid(0));
  assert.ok(!shown.includes(sid(1)));
  assert.deepEqual(
    shown,
    [...shown].sort(
      (a, b) => sessions.findIndex((s) => s.id === a) - sessions.findIndex((s) => s.id === b),
    ),
  );
});

test('a tile reads its newest turn: prompt, lines, and what is not derived yet', () => {
  // Reads and greps alternate, so no two neighbours merge.
  const steps = Array.from({ length: Board.maxLines + 2 }, (_, n) => ({
    id: `tool-${n}`,
    name: n % 2 ? 'Grep' : 'Read',
    effects: [
      { kind: n % 2 ? ('search' as const) : ('read_file' as const), target: `src/${n}.ts` },
    ],
    input: { text: '{}', omitted_chars: 0 },
    result: { status: 'succeeded' as const, output: { text: 'ok', omitted_chars: 0 } },
  }));
  const activityTurn = turn(1, 1, [
    activity(1, { tool_calls: steps }),
    activity(2, { reply: { text: 'Done.', omitted_chars: 0 } }),
    activity(3, { limitations: ['not_yet_derived'] }),
  ]);
  const calls = [
    done(1, 1, { turn: tid(1), started_at: ms(NOW - 40_000) }),
    done(2, 1, {
      turn: tid(1),
      started_at: ms(NOW - 30_000),
      usage: {
        accounting: 'excludes_cache',
        input_tokens: 100,
        output_tokens: 50,
        cache_read_tokens: 800,
        cache_creation_tokens: 100,
        reasoning_tokens: null,
      },
    }),
    open(3, 1, { turn: tid(1) }),
  ];
  const [tile] = board({
    sessions: [session(1)],
    calls,
    turns: new Map([[tid(1), activityTurn]]),
  }).tiles;
  assert.equal(tile.harness, 'claude_code');
  assert.equal(tile.prompt?.text, 'Fix the flaky test');
  assert.equal(tile.lines.length, Board.maxLines);
  assert.deepEqual(tile.lines.at(-1), { kind: 'reply', key: `${callId(2)}:reply`, text: 'Done.' });
  // The expanded card keeps every read call, numbered as the header counts them.
  assert.deepEqual(
    tile.calls.map((call) => [call.index, call.tools.length]),
    [
      [1, Board.maxLines + 2],
      [2, 0],
    ],
  );
  assert.deepEqual(
    tile.unread.map((unread) => [unread.call.id, unread.index]),
    [[callId(3), 3]],
  );
  assert.deepEqual(tile.tally, { read: 0, wrote: 0, ran: 0, failed: 0 });
  // Context adds the cache back when the provider reports input without it.
  assert.equal(tile.context, 1000);
  assert.equal(tile.cached, 0.8);
});

test('the lane places calls in the window and runs open ones to now', () => {
  const lane = Board.lane(
    [
      done(1, 1, {
        started_at: ms(NOW - Board.laneMs - 60_000),
        ended_at: ms(NOW - Board.laneMs - 1),
      }),
      done(2, 1, {
        started_at: ms(NOW - Board.laneMs / 2),
        ended_at: ms(NOW - Board.laneMs / 4),
        status: 500,
      }),
      open(3, 1, { started_at: ms(NOW - Board.laneMs - 1000) }),
    ],
    NOW,
  );
  assert.deepEqual(
    lane.map(({ call, start, end, open, tone }) => [call.id, start, end, open, tone]),
    [
      [callId(2), 0.5, 0.75, false, 'error'],
      [callId(3), 0, 1, true, 'live'],
    ],
  );
});

test('alerts group rate limits, list bad endings, and surface exposures and repeated failures', () => {
  const exposure = {
    kind: 'secret_exposure' as const,
    call: callId(4),
    turn: tid(1),
    field: 'tool_output' as const,
    secret: 'github_token' as const,
    hint: 'ghp_…',
    fingerprint: 'abc',
    count: 1,
  };
  const growth = {
    kind: 'context_growth' as const,
    call: callId(4),
    turn: tid(1),
    earlier_peak_tokens: 1,
    context_tokens: 2,
  };
  const alerts = board({
    sessions: [session(1)],
    calls: [
      done(1, 1, { status: 429, outcome: null }),
      done(2, 1, { status: 429, outcome: null }),
      done(3, 1, { terminal: 'cancelled', outcome: null }),
      done(4, 1, { turn: tid(1) }),
    ],
    turns: new Map([[tid(1), turn(1, 1, [activity(4)], { findings: [exposure, growth] })]]),
  }).alerts;
  assert.deepEqual(alerts.map((alert) => alert.label).sort(), [
    '2 × 429',
    'Cancelled',
    'Secret exposed',
  ]);
  assert.ok(alerts.every((alert) => alert.href.startsWith('/')));
});

test('turns to read: each session newest turn, with a signature that moves with its calls', () => {
  const before = [done(1, 1, { turn: tid(1) }), open(2, 1, { turn: tid(2) }), done(3, 2)];
  const after = [done(1, 1, { turn: tid(1) }), done(2, 1, { turn: tid(2) }), done(3, 2)];
  const [first] = Board.turnsToRead([sid(1), sid(2)], before);
  assert.equal(first.turn, tid(2));
  assert.equal(Board.turnsToRead([sid(2)], before).length, 0);
  assert.notEqual(Board.turnsToRead([sid(1)], after)[0].signature, first.signature);
});

test('the URL place parses density, band, grouping, and window once', () => {
  const plain: LiveView = {
    density: 'comfortable',
    band: 'ribbon',
    window: '15m',
    focus: null,
    pins: [],
    by: 'client',
    series: null,
  };
  assert.deepEqual(LivePlace.parse(new URLSearchParams('')), plain);
  const view = LivePlace.parse(
    new URLSearchParams('density=compact&band=area&window=1h&focus=x&by=model&series=m-a'),
  );
  // A focus that is not a session ID is dropped.
  assert.deepEqual(view, {
    density: 'compact',
    band: 'area',
    window: '1h',
    focus: null,
    pins: [],
    by: 'model',
    series: 'm-a',
  });
  assert.equal(
    LivePlace.href(view),
    '/live?window=1h&density=compact&band=area&by=model&series=m-a',
  );
  // Ribbon and grouping by client are the defaults, kept off the URL.
  assert.equal(LivePlace.href(plain), '/live');
  assert.equal(LivePlace.href({ ...plain, band: 'area' }), '/live?band=area');
  const focused = LivePlace.parse(new URLSearchParams(`focus=${sid(1).toUpperCase()}`));
  assert.equal(focused.focus, sid(1));
  assert.equal(LivePlace.href(focused), `/live?focus=${sid(1)}`);
  assert.equal(LivePlace.parse(new URLSearchParams('band=seismo')).band, 'ribbon');
  assert.equal(LivePlace.parse(new URLSearchParams('by=harness')).by, 'client');
  assert.equal(LivePlace.parse(new URLSearchParams('series=')).series, null);
  // Unknown windows, and names inherited from Object, fall back to the default.
  assert.equal(LivePlace.parse(new URLSearchParams('window=2d')).window, '15m');
  assert.equal(LivePlace.parse(new URLSearchParams('window=toString')).window, '15m');
  assert.equal(LivePlace.longer('15m'), '1h');
  assert.equal(LivePlace.longer('4h'), null);
});

test('the window decides which sessions count as active', () => {
  const sessions = [
    session(1, { last_seen: ms(NOW - 10 * 60_000) }),
    session(2, { last_seen: ms(NOW - 2 * 60 * 60_000) }),
  ];
  const active = (window: '5m' | '15m' | '1h' | '4h') =>
    Board.activeSessions(sessions, new Map(), NOW, LivePlace.windows[window].ms).map((s) => s.id);
  assert.deepEqual(active('5m'), []);
  assert.deepEqual(active('15m'), [sid(1)]);
  assert.deepEqual(active('4h'), [sid(1), sid(2)]);
});

test('the reader reads a turn again only when its calls change or text is still pending', async () => {
  const reads: string[] = [];
  let calls = [done(1, 1, { turn: tid(1) })];
  let pending = true;
  const fetcher = (async (input: Parameters<typeof fetch>[0]) => {
    const path = String(input);
    reads.push(path.split('?')[0]);
    if (path.startsWith('/api/sessions'))
      return versioned({ sessions: [session(1, { last_seen: ms(Date.now()) })] });
    if (path.startsWith('/api/calls')) return versioned({ calls, next: null });
    const limitations = pending ? ['not_yet_derived'] : [];
    return versioned(turn(1, 1, [activity(1, { limitations: limitations as never })]));
  }) as typeof fetch;
  const reader = new LiveReader();
  const turnReads = () => reads.filter((path) => path.startsWith('/api/turns')).length;

  await reader.read(fetcher, '15m');
  assert.equal(turnReads(), 1);
  // Still being derived: read again.
  pending = false;
  await reader.read(fetcher, '15m');
  assert.equal(turnReads(), 2);
  // Settled and unchanged: served from the cache.
  const cached = await reader.read(fetcher, '15m');
  assert.equal(turnReads(), 2);
  assert.equal(cached.value?.turns.get(tid(1))?.prompt?.text, 'Fix the flaky test');
  // A new call in the turn changes it.
  calls = [...calls, open(2, 1, { turn: tid(1) })];
  await reader.read(fetcher, '15m');
  assert.equal(turnReads(), 3);
});

test('the reader reads as far back as the window and says when a listing came back full', async () => {
  const paths: string[] = [];
  let full = false;
  const fetcher = (async (input: Parameters<typeof fetch>[0]) => {
    const path = String(input);
    paths.push(path);
    if (path.startsWith('/api/sessions'))
      return versioned({ sessions: [session(1, { last_seen: ms(Date.now()) })] });
    if (path.startsWith('/api/calls')) {
      const limit = Number(new URL(path, 'http://ui').searchParams.get('limit'));
      const count = full && path.includes('from_ms') ? limit : 1;
      return versioned({
        calls: Array.from({ length: count }, (_, n) => done(n + 1, 1)),
        next: null,
      });
    }
    return versioned(turn(1, 1, []));
  }) as typeof fetch;

  const hour = await new LiveReader().read(fetcher, '1h');
  assert.equal(hour.key, 'live:1h');
  const from = paths
    .map((path) => new URL(path, 'http://ui').searchParams)
    .find((params) => params.has('from_ms'));
  assert.equal(from?.get('limit'), String(LivePlace.windows['1h'].calls));
  assert.ok(Date.now() - Number(from?.get('from_ms')) >= LivePlace.windows['1h'].ms);
  assert.deepEqual(hour.value?.truncated, { calls: false, sessions: false });

  full = true;
  const partial = await new LiveReader().read(fetcher, '1h');
  assert.equal(partial.value?.truncated.calls, true);
});

test('a turn that cannot be read leaves the board readable', async () => {
  const fetcher = (async (input: Parameters<typeof fetch>[0]) => {
    const path = String(input);
    if (path.startsWith('/api/sessions'))
      return versioned({ sessions: [session(1, { last_seen: ms(Date.now()) })] });
    if (path.startsWith('/api/calls'))
      return versioned({ calls: [done(1, 1, { turn: tid(1) })], next: null });
    return versioned({ error: { code: 'unavailable', message: 'busy' } }, { status: 503 });
  }) as typeof fetch;
  const snapshot = await new LiveReader().read(fetcher, '15m');
  assert.equal(snapshot.error, null);
  assert.equal(snapshot.value?.turns.size, 0);
  assert.equal(snapshot.value?.calls.length, 1);
});

test('a missing tool result is expected until a later call in the turn has been read', () => {
  const tool = {
    id: 'tool',
    name: 'Bash',
    effects: [],
    input: { text: '{}', omitted_chars: 0 },
    result: null,
  };
  const labels = (calls: CallActivity[]) =>
    Board.turnCalls(turn(1, 1, calls), new Map(), null)
      .flatMap((call) => call.tools)
      .map((tool) => tool.result.label);
  // A later call was read and carried no result for it: missing.
  assert.deepEqual(
    labels([activity(1, { tool_calls: [tool] }), activity(2, { tool_calls: [tool] })]),
    ['No result', 'No result yet'],
  );
  // The later call is not read yet: it may still carry the result.
  assert.deepEqual(
    labels([
      activity(1, { tool_calls: [tool] }),
      activity(2, { limitations: ['not_yet_derived'] }),
    ]),
    ['No result yet'],
  );
});

test('a session with nothing on its lane is quiet; an open call is not', () => {
  const board = new Board({
    sessions: [session(1), session(2)],
    calls: [done(1, 1, { started_at: ms(NOW - 400_000), ended_at: ms(NOW - 300_000) }), open(2, 2)],
    turns: new Map(),
    now: NOW,
    windowMs: WINDOW,
    focus: null,
    pins: [],
    only: null,
  });
  assert.deepEqual(
    board.tiles.map((tile) => tile.quiet),
    [true, false],
  );
});

test("a tile's attention is its worst alert", () => {
  const board = new Board({
    sessions: [session(1), session(2), session(3)],
    calls: [
      done(1, 1, { status: 429, outcome: null }),
      done(2, 2, { terminal: 'cancelled', outcome: null }),
      done(3, 3),
    ],
    turns: new Map(),
    now: NOW,
    windowMs: WINDOW,
    focus: null,
    pins: [],
    only: null,
  });
  assert.equal(board.attention(sid(1)), 'warning');
  assert.notEqual(board.attention(sid(2)), null);
  assert.equal(board.attention(sid(3)), null);
});

test('the band counts output tokens per client when each call ends', () => {
  const band = new Band(
    [
      session(1),
      session(2, { source: 'codex_headers' }),
      session(3, { source: 'pi_extension_headers' }),
    ],
    [
      done(1, 1, { usage: usage(600), ended_at: ms(NOW - 30_000) }),
      done(2, 1, { usage: usage(300), ended_at: ms(NOW - 120_000) }),
      done(3, 2, { usage: usage(150), ended_at: ms(NOW - 59_000), status: 429 }),
      // Outside the span: not drawn.
      done(4, 1, { usage: usage(9999), ended_at: ms(NOW - Band.spanMs - 1) }),
      done(5, 3, { usage: usage(9999), ended_at: ms(NOW - Band.spanMs - 1) }),
    ],
    NOW,
  );
  // Biggest first, and a client with nothing in the span is not drawn at all.
  assert.deepEqual(
    band.series.map((series) => [series.key, series.label, series.swatch]),
    [
      ['claude_code', 'Claude Code', 'harness-claude_code'],
      ['codex', 'Codex', 'harness-codex'],
    ],
  );
  const [claude, codex] = band.series;
  assert.equal(claude?.rate, 600 / 60);
  assert.equal(
    claude?.bins.reduce((a, b) => a + b, 0),
    (600 + 300) / (Band.binMs / 1000),
  );
  assert.equal(codex?.rate, 150 / 60);
  assert.equal(codex?.marks.length, 1);
  assert.ok(band.max >= 50);
  // The ribbon stacks every series, so its scale covers their sum.
  assert.ok(band.ribbonMax >= band.max);
  const top = band.layer(1, 100).line;
  const bottom = band.layer(0, 100).line;
  assert.equal(top.split(' L').length, (claude?.bins.length ?? 0) + 2);
  assert.notEqual(top, bottom);
  // The first layer sits on the baseline: its closing edge runs back along y = height.
  // (It starts with the first slot, at or just left of the box's edge.)
  assert.match(band.layer(0, 100).area, /L1000,100\.0 .* L-?\d+\.\d,100\.0 Z$/);
  // In Area each series gets an even slice of the height, first at the top.
  assert.equal(band.row(1, 100).top, 50);
  assert.match(band.row(0, 100).area, /L1000,50\.0 L0,50\.0 Z$/);
});

test('the band keeps past bins as time passes, and only slides', () => {
  const calls = [
    done(1, 1, { usage: usage(600), ended_at: ms(NOW - 200_000) }),
    done(2, 1, { usage: usage(300), ended_at: ms(NOW - 61_000) }),
  ];
  const bins = (now: number) => new Band([session(1)], calls, now).series[0]?.bins ?? [];
  // Within one slot, a redraw changes nothing about past calls.
  assert.deepEqual(bins(NOW + 1_000), bins(NOW));
  // A slot later, the same bins have moved one to the left.
  const later = bins(NOW + Band.binMs);
  assert.deepEqual(later.slice(0, -1), bins(NOW).slice(1));
  // And the band slides smoothly in between.
  const offset = (now: number) => new Band([session(1)], calls, now).offset;
  assert.ok(offset(NOW + 1_000) < offset(NOW));
  assert.ok(offset(NOW) <= 0 && offset(NOW) > -Band.binMs / Band.spanMs);
});

test('the band groups by provider or model, and merges past the biggest four', () => {
  const models = ['m-a', 'm-b', 'm-c', 'm-d', 'm-e', 'm-f'];
  const calls = models.map((model, n) =>
    done(n + 1, 1, {
      model,
      provider: n % 2 ? 'openai' : 'anthropic',
      usage: usage(100 * (models.length - n)),
      ended_at: ms(NOW - 30_000),
    }),
  );
  const byModel = new Band([session(1)], calls, NOW, 'model');
  assert.deepEqual(
    byModel.series.map((series) => series.key),
    ['m-a', 'm-b', 'm-c', 'm-d', Band.other],
  );
  const other = byModel.series.at(-1);
  assert.deepEqual(other?.members, ['m-e', 'm-f']);
  assert.equal(other?.label, '2 more');
  assert.equal(other?.swatch, 'series-other');
  assert.equal(byModel.seriesOf('m-f')?.key, Band.other);
  assert.equal(byModel.seriesOf('m-z'), null);
  // One past the limit is drawn on its own: merging a single series hides its name for nothing.
  const five = new Band([session(1)], calls.slice(0, 5), NOW, 'model');
  assert.equal(five.series.length, 5);
  assert.ok(!five.series.some((series) => series.key === Band.other));
  const byProvider = new Band([session(1)], calls, NOW, 'provider');
  assert.deepEqual(
    byProvider.series.map((series) => [series.label, series.swatch]),
    [
      ['Anthropic', 'series-0'],
      ['OpenAI', 'series-1'],
    ],
  );
});

test('picking a band series narrows the wall to its sessions', () => {
  const sessions = [session(1), session(2, { source: 'codex_headers', provider: 'openai' })];
  const calls = [done(1, 1, { model: 'm-a' }), done(2, 2, { model: 'm-b' })];
  const ids = (input: Partial<BoardInput>) =>
    board({ sessions, calls, ...input }).tiles.map((tile) => tile.session.id);
  assert.deepEqual(ids({}), [sid(1), sid(2)]);
  assert.deepEqual(ids({ only: { by: 'client', keys: ['codex'] } }), [sid(2)]);
  assert.deepEqual(ids({ only: { by: 'provider', keys: ['anthropic'] } }), [sid(1)]);
  assert.deepEqual(ids({ only: { by: 'model', keys: ['m-b', 'm-a'] } }), [sid(1), sid(2)]);
});

test('the feed tells what happened lately, ongoing waits first, newest next', () => {
  const failing = turn(
    1,
    1,
    [
      activity(1, {
        tool_calls: [
          {
            id: 'tool-1',
            name: 'Edit',
            effects: [{ kind: 'write_file' as const, target: 'src/relay.rs' }],
            input: { text: '{}', omitted_chars: 0 },
            result: { status: 'failed' as const, output: { text: 'no match', omitted_chars: 0 } },
          },
        ],
      }),
      activity(2, { started_at: ms(NOW - 8000) }),
    ],
    { started_at: ms(NOW - 40_000) },
  );
  const feed = board({
    sessions: [
      session(1, { first_seen: ms(NOW - 50_000) }),
      session(2, { first_seen: ms(NOW - 600_000) }),
    ],
    calls: [
      done(1, 1, { turn: tid(1), tool_names: ['Edit', 'Edit'], ended_at: ms(NOW - 9000) }),
      done(2, 1, { turn: tid(1), started_at: ms(NOW - 8000), ended_at: ms(NOW - 3000) }),
      // Waiting 6 s for its head: pinned. Waiting 2 s: not yet worth a line.
      open(3, 2, { started_at: ms(NOW - 6000) }),
      open(4, 2, { started_at: ms(NOW - 2000) }),
      done(5, 2, { status: 429, outcome: null, started_at: ms(NOW - 30_000) }),
      done(6, 2, { status: 429, outcome: null, started_at: ms(NOW - 20_000) }),
      // Finished long ago: gone from the feed.
      done(7, 2, { started_at: ms(NOW - 400_000), ended_at: ms(NOW - 300_000) }),
    ],
    turns: new Map([[tid(1), failing]]),
  }).feed;
  const lines = feed.map((item) => item.text);
  assert.equal(lines[0], 'no response head after 6 s');
  assert.equal(feed[0]?.pinned, true);
  assert.ok(lines.includes('new session'));
  assert.ok(lines.includes('you: “Fix the flaky test”'));
  assert.ok(lines.includes('asked for Edit'));
  assert.ok(lines.includes('turn finished'));
  assert.ok(lines.includes('Edit src/relay.rs failed'));
  assert.ok(lines.includes('Anthropic: 2 rate limits (429)'));
  assert.equal(lines.filter((line) => line === 'new session').length, 1);
  assert.equal(feed.filter((item) => item.text.startsWith('no response head')).length, 1);
  // After the pinned wait, newest first.
  const times = feed.slice(1).map((item) => item.at);
  assert.deepEqual(
    times,
    [...times].sort((a, b) => b - a),
  );
  assert.ok(new Set(feed.map((item) => item.key)).size === feed.length);
});

test('feed text is clipped to one line', () => {
  assert.equal(Feed.clip('  fix\n the   test  ', 20), 'fix the test');
  assert.equal(Feed.clip('abcdefghij', 5), 'abcd…');
});

function usage(output: number): CallView['usage'] {
  return {
    accounting: 'includes_cache',
    input_tokens: 1000,
    output_tokens: output,
    cache_read_tokens: 0,
    cache_creation_tokens: 0,
    reasoning_tokens: null,
  };
}

test('a session that reports no turns keeps its newest call, so its reply can stream', () => {
  const sessions = [session(1, { source: 'opencode_headers', turn_count: 0 })];
  const calls = [done(1, 1, { started_at: ms(NOW - 30_000) }), open(2, 1)];
  const [tile] = board({ sessions, calls, turns: new Map() }).tiles;
  assert.equal(tile.turnId, null);
  assert.deepEqual(
    tile.unread.map((unread) => [unread.call.id, unread.index]),
    [[callId(2), 2]],
  );
  const [quiet] = board({ sessions, calls: calls.slice(0, 1), turns: new Map() }).tiles;
  assert.deepEqual(
    quiet.unread.map((unread) => unread.call.id),
    [callId(1)],
  );
});

test('each hint source names its harness, opencode included', () => {
  assert.deepEqual(
    (
      [
        'claude_code_headers',
        'codex_headers',
        'codex_message_metadata',
        'pi_extension_headers',
        'opencode_headers',
      ] as const
    ).map(Board.harness),
    ['claude_code', 'codex', 'codex', 'pi', 'opencode'],
  );
});

const tool = (
  id: string,
  name: string,
  kind: 'read_file' | 'run_command' | 'write_file',
  target: string,
  status: 'succeeded' | 'failed' | null = 'succeeded',
) => ({
  id,
  name,
  effects: [{ kind, target }],
  input: { text: `{"target":"${target}"}`, omitted_chars: 0 },
  result:
    status === null ? null : { status, output: { text: `\n  ${id} out\nmore`, omitted_chars: 0 } },
});

test('a card merges like tool calls across calls, until the model says something', () => {
  const calls = Board.turnCalls(
    turn(1, 1, [
      activity(1, { tool_calls: [tool('a', 'Bash', 'run_command', 'cargo test')] }),
      activity(2, { tool_calls: [tool('b', 'Bash', 'run_command', 'cargo test', 'failed')] }),
      activity(3, { tool_calls: [tool('c', 'Bash', 'run_command', 'cargo clippy')] }),
      activity(4, {
        reply: { text: 'Fixing the reader.', omitted_chars: 0 },
        tool_calls: [tool('d', 'Edit', 'write_file', 'src/reader.rs')],
      }),
      activity(5, { tool_calls: [tool('e', 'Bash', 'run_command', 'cargo test')] }),
      activity(6, { tool_calls: [tool('f', 'Bash', 'run_command', 'cargo test')] }),
    ]),
    new Map(),
    null,
  );
  const lines = Board.lines(calls).map((line) =>
    line.kind === 'reply'
      ? `reply ${line.text}`
      : `${line.tool.name} ×${line.count} ${line.tool.target} ${line.result.label}`,
  );
  // The reply comes before the tools its call asked for, and breaks the run.
  assert.deepEqual(lines, [
    'Bash ×3 cargo clippy 1 failed',
    'reply Fixing the reader.',
    'Edit ×1 src/reader.rs Succeeded',
    'Bash ×2 cargo test 2 ok',
  ]);
  // The output line is the first one with text in it.
  assert.equal(calls[0]?.tools[0]?.output, 'a out');
});

test("a card drops the cd into the turn's directory and paths under it", () => {
  assert.deepEqual(Board.changeDirectory('cd "/a b" && ls'), { dir: '/a b', rest: 'ls' });
  assert.deepEqual(Board.changeDirectory('cd /repo; make'), { dir: '/repo', rest: 'make' });
  assert.equal(Board.changeDirectory('cargo test'), null);
  const activityTurn = turn(1, 1, [], {
    commands: [
      'cd /Users/me/dev/doppel && cargo test',
      'cd /Users/me/dev/doppel && cargo clippy',
      'cd /tmp && ls',
    ],
  });
  const workdir = Board.workdir(activityTurn);
  assert.equal(workdir, '/Users/me/dev/doppel');
  assert.equal(
    Board.shorten(
      'cd /Users/me/dev/doppel && cargo test --manifest-path /Users/me/dev/doppel/Cargo.toml',
      workdir,
    ),
    'cargo test --manifest-path Cargo.toml',
  );
  assert.equal(Board.shorten('/Users/me/dev/doppel/src/main.rs', workdir), 'src/main.rs');
  assert.equal(Board.shorten('ls /Users/me/dev/doppel', workdir), 'ls .');
  // A cd somewhere else is kept: it says where the command ran.
  assert.equal(Board.shorten('cd /tmp && ls', workdir), 'cd /tmp && ls');
  assert.equal(Board.shorten('cd /tmp && ls', null), 'cd /tmp && ls');
  assert.equal(Board.workdir(turn(1, 1, [], { commands: ['cargo test'] })), null);
});

test('an expanded card comes first, with four beside it and the rest counted', () => {
  const sessions = Array.from({ length: Board.maxTiles + 1 }, (_, n) => session(n));
  const focused = board({ sessions, focus: sid(6) });
  assert.equal(focused.focus, sid(6));
  assert.equal(focused.tiles[0]?.session.id, sid(6));
  assert.equal(focused.tiles.length, 1 + Board.besideFocus);
  assert.equal(focused.capacity, 1 + Board.besideFocus);
  assert.equal(focused.overflow, sessions.length - focused.tiles.length);
  // A focus on a session that is not active is ignored.
  const gone = board({ sessions, focus: sid(50) });
  assert.equal(gone.focus, null);
  assert.equal(gone.tiles.length, Board.maxTiles);
  assert.equal(gone.capacity, Board.maxTiles);
});

test('pins parse once, deduplicated, and toggle in the URL', () => {
  const view = LivePlace.parse(
    new URLSearchParams(`pin=${sid(2)}&pin=nope&pin=${sid(1).toUpperCase()}&pin=${sid(2)}`),
  );
  assert.deepEqual(view.pins, [sid(2), sid(1)]);
  assert.equal(LivePlace.href(view), `/live?pin=${sid(2)}&pin=${sid(1)}`);
  assert.deepEqual(LivePlace.togglePin(view, sid(2)).pins, [sid(1)]);
  assert.deepEqual(LivePlace.togglePin(view, sid(3)).pins, [sid(2), sid(1), sid(3)]);
});

test('a pinned session keeps its card when busier ones would take it', () => {
  // Session 0 is the quietest: seen longest ago, nothing open.
  const sessions = Array.from({ length: Board.maxTiles + 2 }, (_, n) =>
    session(n, { last_seen: ms(NOW - 60_000 + n * 1000) }),
  );
  const calls = Array.from({ length: Board.maxTiles + 1 }, (_, n) => open(n + 1, n + 1));
  const unpinned = board({ sessions, calls });
  assert.ok(!unpinned.tiles.some((tile) => tile.session.id === sid(0)));
  const pinned = board({ sessions, calls, pins: [sid(0)] });
  const tile = pinned.tiles.find((candidate) => candidate.session.id === sid(0));
  assert.equal(tile?.pinned, true);
  assert.equal(pinned.tiles.length, Board.maxTiles);
  assert.equal(pinned.tiles.filter((candidate) => candidate.pinned).length, 1);
  // A pin on a session that is not active adds nothing.
  assert.equal(board({ sessions, pins: [sid(50)] }).tiles.length, Board.maxTiles);
});

test('every state has its own mark, and an unknown ending is never a failure', () => {
  const mark = (calls: CallView[]) => Board.phaseChip(Board.phase(calls, NOW), NOW).mark;
  const long = ms(NOW - Board.laneMs - 1);
  assert.equal(mark([open(1, 1)]), 'waiting');
  assert.equal(mark([open(1, 1, { started_at: ms(NOW - Board.slowHeadMs - 1) })]), 'slow');
  assert.equal(mark([open(1, 1, { status: 200 })]), 'responding');
  assert.equal(mark([done(1, 1, { tool_names: ['Bash'] })]), 'client');
  assert.equal(mark([done(1, 1)]), 'yours');
  assert.equal(mark([done(1, 1, { started_at: long, ended_at: long })]), 'idle');
  assert.equal(mark([done(1, 1, { status: 500, outcome: null })]), 'failed');
  assert.equal(mark([done(1, 1, { status: 429, outcome: null })]), 'failed');
  assert.equal(mark([done(1, 1, { terminal: 'cancelled', outcome: null })]), 'warning');
  assert.equal(mark([done(1, 1, { outcome: 'unknown', state: 'recovered' })]), 'unknown');
  assert.equal(mark([]), 'idle');
});

test("a tool's result is unknown, not missing, when the request carrying it was unreadable", () => {
  const tool = {
    id: 'tool',
    name: 'Bash',
    effects: [],
    input: { text: '{}', omitted_chars: 0 },
    result: null,
  };
  const labels = (calls: CallActivity[]) =>
    Board.turnCalls(turn(1, 1, calls), new Map(), null)
      .flatMap((call) => call.tools)
      .map((line) => [line.result.label, line.result.tone]);
  assert.deepEqual(
    labels([
      activity(1, { tool_calls: [tool] }),
      activity(2, { limitations: ['capture_gap', 'not_json'] }),
    ]),
    [['Result not readable', 'neutral']],
  );
  // A readable next request with no result for it: missing.
  assert.deepEqual(labels([activity(1, { tool_calls: [tool] }), activity(2)]), [
    ['No result', 'warning'],
  ]);
});
