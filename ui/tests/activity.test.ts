import test from 'node:test';
import assert from 'node:assert/strict';
import {
  CallId,
  ManagementApi,
  RecordingId,
  TurnId,
  parseSearchField,
  type SearchHit,
} from '../src/lib/api.ts';
import { ShapeError } from '../src/lib/decode.ts';
import {
  callHref,
  effectLabel,
  findingLine,
  findingLink,
  harnessLabel,
  resultBadge,
  searchHitHref,
  turnHref,
  rowPrompt,
  snippetParts,
  turnRows,
  ContextChart,
  type TurnRow,
  type FindingLine,
} from '../src/lib/activity.ts';
import { uuid, versioned } from './fixtures.ts';

const session = uuid(1);
const turn = uuid(2);
const call = uuid(3);

/** A `fetch` answering one path with one body. */
const answering =
  (expected: string, body: object): typeof fetch =>
  async (input) => {
    assert.equal(input, expected);
    return versioned(body);
  };

const reportBody = {
  session,
  totals: {
    calls: 3,
    calls_with_usage: 2,
    peak_context_tokens: 175_115,
    context_tokens: 350_000,
    cache_read_tokens: 173_873,
    cache_written_tokens: 176_484,
    output_tokens: 1_791,
    cache_hit_percent: 49,
  },
  turns: [
    {
      turn,
      started_at: 1_000,
      ended_at: 2_000,
      call_count: 2,
      idle_before_ms: 3_708_151,
      context_start_tokens: 175_115,
      context_end_tokens: 176_486,
      cache_hit_percent: 49,
      cache_written_tokens: 176_484,
      output_tokens: 1_791,
    },
  ],
  findings: [
    {
      kind: 'cache_rebuild',
      call,
      turn,
      cache_read_tokens: 0,
      cache_written_tokens: 175_113,
      earlier_peak_tokens: 173_873,
      idle_before_ms: 3_708_151,
    },
    {
      kind: 'context_growth',
      call,
      turn: null,
      earlier_peak_tokens: 52_004,
      context_tokens: 71_230,
    },
    { kind: 'compaction', call },
  ],
  omitted_calls: 0,
};

test('a session report decodes, keeps unknown findings, and reads as measurements', async () => {
  const id = RecordingId.parse(session);
  assert.ok(id);
  const report = await new ManagementApi(
    answering(`/api/sessions/${session}/report`, reportBody),
  ).report(id);
  assert.equal(report.totals.cache_hit_percent, 49);
  assert.deepEqual(
    report.findings.map((finding) => finding.kind),
    ['cache_rebuild', 'context_growth', 'unrecognized'],
  );
  const rebuild = findingLine(report.findings[0]);
  assert.equal(rebuild.title, 'Cache rebuilt');
  assert.equal(rebuild.tone, 'warning');
  assert.equal(
    rebuild.detail,
    'Wrote 175k tokens to the cache and read 0 of an earlier 174k tokens, after 1 h 2 min idle.',
  );
  assert.doesNotMatch(rebuild.detail, /expired|because/);
  assert.equal(findingLine(report.findings[2]).title, 'compaction');

  const rows = turnRows(report, [
    {
      turn: turn as never,
      native_id: 'prompt' as never,
      started_at: 1_000 as never,
      ended_at: 2_000 as never,
      call_count: 2,
      error_count: 0,
      tokens: { input: 1, output: 2, cache_read: 0 },
      prompt: { text: 'Fix the tests', omitted_chars: 0 },
    },
  ]);
  assert.equal(rows.length, 1);
  assert.equal(rows[0].summary?.prompt?.text, 'Fix the tests');
  assert.equal(rowPrompt(rows[0]), 'Fix the tests');
  // A turn the listing did not reach had its prompt not loaded, not missing.
  assert.equal(rowPrompt({ ...rows[0], summary: null }), 'Prompt not loaded');
  assert.equal(
    rowPrompt({ ...rows[0], summary: { ...rows[0].summary!, prompt: null } }),
    'No new prompt',
  );
  // Only findings that point into the turn are attached to it.
  assert.deepEqual(
    rows[0].findings.map((line) => line.title),
    ['Cache rebuilt'],
  );
});

test('a turn decodes its calls, tool calls, effects, and results', async () => {
  const id = TurnId.parse(turn);
  assert.ok(id);
  const body = {
    turn,
    session,
    native_id: 'prompt-1',
    client: 'claude_code',
    started_at: 1_000,
    ended_at: 3_500,
    prompt: { text: 'Why do the tests fail?', omitted_chars: 0 },
    files_read: [],
    files_written: [],
    commands: ['cargo test'],
    tool_calls: 1,
    failed_tool_calls: 1,
    unanswered_tool_calls: 0,
    findings: [
      { kind: 'repeated_failure', tool: 'Bash', target: 'cargo test', failures: 3, calls: [call] },
    ],
    calls: [
      {
        call,
        started_at: 1_000,
        ended_at: 2_000,
        model: 'fixture',
        status: 200,
        outcome: 'completed',
        metrics: {
          context_tokens: 100,
          cache_read_tokens: null,
          cache_written_tokens: null,
          output_tokens: 5,
          request_bytes: 300,
          duration_ms: 1_000,
        },
        prompt: null,
        tool_calls: [
          {
            id: 'toolu_1',
            name: 'Bash',
            effects: [{ kind: 'run_command', target: 'cargo test' }],
            input: { text: '{"command":"cargo test"}', omitted_chars: 0 },
            result: { status: 'failed', output: { text: 'FAILED', omitted_chars: 12 } },
          },
        ],
        earlier_results: 0,
        reply: null,
        limitations: ['unfinished_tool_call', 'some_new_limitation'],
      },
    ],
    omitted_calls: 0,
  };
  const activity = await new ManagementApi(answering(`/api/turns/${turn}`, body)).turn(id);
  const tool = activity.calls[0].tool_calls[0];
  assert.equal(effectLabel(tool.effects[0].kind), 'Ran');
  assert.deepEqual(resultBadge(tool.result), { tone: 'error', label: 'Failed' });
  assert.deepEqual(resultBadge(null), { tone: 'warning', label: 'No result' });
  assert.deepEqual(activity.calls[0].limitations, ['unfinished_tool_call', 'some_new_limitation']);
  assert.equal(findingLine(activity.findings[0]).detail, 'Bash cargo test failed 3 times.');

  // A required field missing is a shape error, not a silent default.
  const broken = { ...body, calls: [{ ...body.calls[0], metrics: undefined }] };
  await assert.rejects(
    new ManagementApi(answering(`/api/turns/${turn}`, broken)).turn(id),
    ShapeError,
  );
});

test('snippets split into marked matches without trusting any markup', () => {
  assert.deepEqual(snippetParts('test «parser» ... «FAILED»!'), [
    { text: 'test ', match: false },
    { text: 'parser', match: true },
    { text: ' ... ', match: false },
    { text: 'FAILED', match: true },
    { text: '!', match: false },
  ]);
  // An unclosed marker is plain text; HTML stays text for the view to escape.
  assert.deepEqual(snippetParts('a «b <script>'), [{ text: 'a «b <script>', match: false }]);
  assert.deepEqual(snippetParts(''), []);
});

test('an exposed secret reads as what happened, never the secret', () => {
  const line = findingLine({
    kind: 'secret_exposure',
    call: uuid(3) as never,
    turn: null,
    field: 'tool_output',
    secret: 'github_token',
    hint: 'ghp_…wxyz',
    fingerprint: '0123456789abcdef',
    count: 2,
  });
  assert.equal(line.tone, 'error');
  assert.equal(line.title, 'Secret exposed');
  assert.equal(
    line.detail,
    'GitHub token ghp_…wxyz in the tool output, sent to the provider (2 times). Rotate it: Ren cannot recall it.',
  );
  const reply = findingLine({
    kind: 'secret_exposure',
    call: uuid(3) as never,
    turn: null,
    field: 'reply',
    secret: 'jwt',
    hint: 'eyJ…abcd',
    fingerprint: 'f',
    count: 1,
  });
  assert.match(reply.detail, /^JSON Web Token eyJ…abcd in the model's reply\./);
});

test('harnesses read by name, and an unknown one keeps its text', () => {
  assert.deepEqual((['claude_code', 'codex', 'pi', 'opencode'] as const).map(harnessLabel), [
    'Claude Code',
    'Codex',
    'Pi',
    'OpenCode',
  ]);
  assert.equal(harnessLabel('new_agent' as never), 'new agent');
});

test('a search field parses from its label and nothing else', () => {
  assert.equal(parseSearchField('tool_output'), 'tool_output');
  assert.equal(parseSearchField('prompt'), 'prompt');
  for (const text of [null, '', 'Prompt', 'tool output', 'everything']) {
    assert.equal(parseSearchField(text), null);
  }
});

test('links to calls, turns, hits, and findings encode their IDs', () => {
  const sessionId = RecordingId.parse(session);
  const turnId = TurnId.parse(turn);
  const callId = CallId.parse(call);
  assert.ok(sessionId && turnId && callId);
  assert.equal(callHref(sessionId, callId), `/recordings/${session}?call=${call}`);
  assert.equal(turnHref(turnId), `/turns/${turn}`);

  const hit = (overrides: Partial<SearchHit>): SearchHit => ({
    call: callId,
    session: sessionId,
    turn: turnId,
    client: 'codex',
    started_at: 0 as never,
    field: 'prompt',
    snippet: '',
    ...overrides,
  });
  assert.equal(searchHitHref(hit({})), `/turns/${turn}`);
  assert.equal(searchHitHref(hit({ turn: null })), `/recordings/${session}?call=${call}`);
  assert.equal(searchHitHref(hit({ turn: null, session: null })), `/traffic?call=${call}`);

  const line = (overrides: Partial<FindingLine>): FindingLine => ({
    tone: 'warning',
    title: 'Cache rebuilt',
    detail: '',
    call: callId,
    turn: turnId,
    ...overrides,
  });
  // A session report opens the turn first; a turn's page opens only the call.
  assert.deepEqual(findingLink(line({}), sessionId, 'session'), {
    href: `/turns/${turn}`,
    label: 'Open turn',
  });
  assert.deepEqual(findingLink(line({ turn: null }), sessionId, 'session'), {
    href: `/recordings/${session}?call=${call}`,
    label: 'Open call',
  });
  assert.deepEqual(findingLink(line({}), sessionId, 'turn'), {
    href: `/recordings/${session}?call=${call}`,
    label: 'Open call',
  });
  assert.equal(findingLink(line({ call: null, turn: null }), sessionId, 'session'), null);
  assert.equal(findingLink(line({ call: null }), sessionId, 'turn'), null);
});

test('the context chart reads oldest first, against the peak, and marks flagged turns', () => {
  const row = (n: number, start: number | null, end: number | null, flagged = false): TurnRow => ({
    metrics: {
      turn: uuid(500 + n) as never,
      started_at: (n * 1000) as never,
      ended_at: null,
      call_count: 1,
      idle_before_ms: null,
      context_start_tokens: start,
      context_end_tokens: end,
      cache_hit_percent: null,
      cache_written_tokens: 0,
      output_tokens: 0,
    },
    summary: null,
    findings: flagged ? [{ title: 'Context grew' } as never] : [],
  });
  // Report rows come newest first.
  const chart = new ContextChart([
    row(3, 200_000, 100_000),
    row(2, 100_000, 200_000, true),
    row(1, 50_000, 100_000),
    row(0, null, null),
  ]);
  assert.equal(chart.max, 200_000);
  assert.deepEqual(
    chart.bars.map((bar) => [bar.turn, bar.start, bar.end, bar.flagged]),
    [
      [uuid(501), 0.25, 0.5, false],
      [uuid(502), 0.5, 1, true],
      // A compaction: it ended below where it started.
      [uuid(503), 1, 0.5, false],
    ],
  );
  assert.ok(chart.bars.every((bar, index) => index === 0 || bar.x > chart.bars[index - 1]!.x));
  assert.match(chart.bars[1]!.title, /100k → 200k/);
});
