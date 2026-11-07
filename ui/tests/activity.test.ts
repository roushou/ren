import test from 'node:test';
import assert from 'node:assert/strict';
import { ManagementApi, SessionId, TurnId } from '../src/lib/api.ts';
import { ShapeError } from '../src/lib/decode.ts';
import {
  effectLabel,
  findingLine,
  resultBadge,
  snippetParts,
  turnRows,
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
  const id = SessionId.parse(session);
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
