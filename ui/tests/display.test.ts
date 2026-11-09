import test from 'node:test';
import assert from 'node:assert/strict';
import {
  attemptEnding,
  buildLabel,
  callStatus,
  channelState,
  messageCounts,
  statusBadge,
  duration,
  elapsed,
  captureState,
  modelState,
  transportState,
  streamChip,
  httpChip,
  modelChip,
  compact,
  compactOrDash,
  percentOrDash,
  tokenSummary,
  previewChips,
  span,
  sessionTone,
  captureChip,
  trimmedAtLimit,
} from '../src/lib/display.ts';
import { providerMark, providerName, revision, type CallView } from '../src/lib/api.ts';
import { callFixture, finishedFixture, callId } from './fixtures.ts';

const finished = (overrides: Partial<CallView>) => finishedFixture(callId(1), overrides);
const kind = (call: CallView) => callStatus(call).kind;
const unknownLabel = <T>(text: string) => text as T;

test('row states keep model completion separate from HTTP EOF and restart recovery', () => {
  assert.equal(kind(callFixture()), 'in_progress');
  assert.equal(kind(finished({})), 'http_complete');
  assert.equal(
    kind(finished({ outcome: 'completed', terminal: 'interrupted' })),
    'model_completed',
  );
  assert.equal(kind(finished({ outcome: 'incomplete' })), 'incomplete');
  assert.equal(kind(finished({ outcome: null, terminal: 'cancelled' })), 'interrupted');
  assert.deepEqual(callStatus(finished({ terminal: 'cancelled' })), {
    kind: 'interrupted',
    terminal: 'cancelled',
  });
  for (const overrides of [
    { status: 429 },
    { terminal: 'stream_failed' as const },
    { terminal: 'not_sent' as const },
    { terminal: 'acceptance_unknown' as const },
    { outcome: 'failed' as const },
  ])
    assert.equal(kind(finished(overrides)), 'error');
});

test('recovered and unrecognised outcomes are never rounded up to success', () => {
  const recovered = callFixture(callId(1), { state: 'recovered', ended_at: null });
  assert.equal(kind(recovered), 'recovered');
  assert.equal(statusBadge(callStatus(recovered)).tone, 'warning');
  // Even a recorded 200 head with a completed body is unknown after restart.
  assert.equal(kind({ ...recovered, status: 200, terminal: 'body_complete' }), 'recovered');
  assert.equal(kind({ ...recovered, status: 500 }), 'error');
  assert.equal(modelState(recovered), 'Unknown · restart');
  assert.equal(kind(finished({ outcome: 'unknown' })), 'unknown');
  assert.equal(kind(finished({ outcome: unknownLabel('future_outcome') })), 'unknown');
  assert.equal(kind(finished({ terminal: unknownLabel('future_terminal') })), 'unknown');
  assert.equal(kind(finished({ state: unknownLabel('future_state') })), 'unknown');
  assert.equal(kind(finished({ terminal: null, status: 200 })), 'unknown');
  for (const call of [recovered, finished({ outcome: 'unknown' })])
    assert.notEqual(statusBadge(callStatus(call)).tone, 'success');
});

test('badges and transport labels describe each state', () => {
  assert.deepEqual(statusBadge(callStatus(finished({ outcome: 'completed' }))), {
    label: 'Model completed',
    tone: 'success',
  });
  assert.equal(statusBadge(callStatus(finished({ outcome: 'refused' }))).label, 'Refused');
  assert.equal(transportState(finished({})), '200 · stream complete');
  assert.equal(transportState(callFixture()), '— · streaming');
  assert.equal(transportState(callFixture(callId(1), { status: 200 })), '200 · streaming');
  assert.equal(transportState(callFixture(callId(1), { state: 'recovered' })), '— · unknown');
  // Plain words, colored by what they mean.
  assert.deepEqual(
    [
      streamChip(finished({})),
      streamChip(finished({ terminal: 'interrupted' })),
      streamChip(finished({ terminal: 'stream_failed' })),
    ].map((chip) => [chip.label, chip.tone]),
    [
      ['Stream complete', 'success'],
      ['Cut off', 'warning'],
      ['Stream failed', 'error'],
    ],
  );
  assert.deepEqual(
    [200, 302, 429, 529, null].map((status) => httpChip(status).tone),
    ['success', 'neutral', 'warning', 'error', 'neutral'],
  );
  assert.equal(modelChip(finished({ outcome: 'completed' })).label, 'Model completed');
});

test('duration remains unknown for open calls and formats observed time', () => {
  assert.equal(duration(callFixture()), null);
  assert.equal(elapsed(null), '—');
  assert.equal(elapsed(duration(finished({ ended_at: 1250 as CallView['started_at'] }))), '250 ms');
  assert.equal(elapsed(duration(finished({ ended_at: 4600 as CallView['started_at'] }))), '3.60 s');
});

test('capture labels remain independent of model and transport outcomes', () => {
  const completed = finished({ outcome: 'completed', terminal: 'interrupted' });
  assert.equal(captureState({ ...completed, capture_complete: false }), 'Partial capture');
  assert.equal(captureState(completed), 'Complete capture');
  assert.equal(captureState(callFixture()), 'Recording');
  assert.equal(captureState(callFixture(callId(1), { state: 'recovered' })), 'Interrupted capture');
});

test('attempt endings name the recorded cause and phase, and nothing when absent', () => {
  assert.equal(
    attemptEnding({ terminal_cause: 'header_timeout', terminal_phase: 'awaiting_head' }),
    'header timeout · in awaiting head',
  );
  assert.equal(attemptEnding({ terminal_cause: null, terminal_phase: 'response' }), 'in response');
  assert.equal(attemptEnding({ terminal_cause: null, terminal_phase: null }), null);
});

test('websocket channels say how they ended and what was not stored', () => {
  const channel = {
    id: 'c' as never,
    transport: 'websocket',
    opened_at: 0 as never,
    closed_at: null,
    ending: null,
    close_code: null,
    unattributed_messages: 0,
    unattributed_bytes: 0,
  } as const;
  assert.equal(channelState({ ...channel, transport: 'http' }), null);
  assert.equal(channelState(channel), 'open');
  assert.equal(
    channelState({
      ...channel,
      ending: 'client_closed',
      close_code: 1000,
      unattributed_messages: 2,
    }),
    'client closed · code 1000 · 2 messages between calls (not stored)',
  );
  const mark = (direction: 'request' | 'response') =>
    ({ direction, offset: 0, len: 1, payload: 'text', at: 0 as never }) as const;
  assert.deepEqual(messageCounts([mark('request'), mark('response'), mark('response')]), {
    request: 1,
    response: 2,
  });
});

test('a call TGV refused says so, whatever else was recorded', () => {
  const paused = finishedFixture(callId(7), {
    rejection: 'paused',
    terminal: null,
    status: null,
    response: null,
  });
  assert.deepEqual(callStatus(paused), { kind: 'rejected', rejection: 'paused' });
  assert.deepEqual(statusBadge(callStatus(paused)), { label: 'TGV · paused', tone: 'warning' });
  // A label from a newer daemon is shown verbatim.
  const newer = finishedFixture(callId(8), { rejection: unknownLabel('over_quota') });
  assert.equal(statusBadge(callStatus(newer)).label, 'TGV · over quota');
});

test('builds read like tgv --version, and unknown revisions are kept', () => {
  const commit = '00a59029501566fad56007fa6efe866f97e99193';
  assert.equal(
    buildLabel({ version: '0.1.0', revision: revision(commit, '') }),
    '0.1.0 (00a590295015)',
  );
  assert.equal(
    buildLabel({ version: '0.1.0', revision: revision(`${commit}-dirty`, '') }),
    '0.1.0 (00a590295015, modified)',
  );
  assert.deepEqual(revision('unknown', ''), { kind: 'unknown' });
  assert.equal(
    buildLabel({ version: '0.1.0', revision: revision('unknown', '') }),
    '0.1.0 (unknown revision)',
  );
  assert.deepEqual(revision('v2+abc', ''), { kind: 'unrecognized', text: 'v2+abc' });
  assert.deepEqual(revision(commit.toUpperCase(), ''), {
    kind: 'unrecognized',
    text: commit.toUpperCase(),
  });
  assert.equal(buildLabel(null), '—');
});

test('a missing count or percentage reads as an em dash', () => {
  assert.equal(compactOrDash(null), '—');
  assert.equal(compactOrDash(0), '0');
  assert.equal(compactOrDash(101_658), '102k');
  assert.equal(percentOrDash(null), '—');
  assert.equal(percentOrDash(0), '0%');
  assert.equal(percentOrDash(87), '87%');
});

test('token counts are compact and include cached context', () => {
  assert.deepEqual([89, 1234, 101658, 3_400_000].map(compact), ['89', '1.2k', '102k', '3.4M']);
  const tokens = tokenSummary({
    accounting: 'excludes_cache',
    input_tokens: 96,
    output_tokens: 89,
    cache_read_tokens: 101_658,
    cache_creation_tokens: 777,
    reasoning_tokens: null,
  });
  assert.equal(tokens?.input, '103k');
  assert.equal(tokens?.output, '89');
  assert.match(tokens?.title ?? '', /101,658 cache read/);
  assert.equal(tokenSummary(null), null);
});

test('a body preview reads as chips, with gaps spelled out', () => {
  const preview = (over: object) => ({
    observed_bytes: 1200,
    stored_bytes: 1200,
    ending: 'ended',
    captured_bytes: 1200,
    extent: 'complete',
    raw: { text: '', format: 'utf8', truncated: false },
    text: '',
    text_end: 'intact',
    decoding: { applied: 'gzip_magic', integrity: 'verified', truncated: false },
    ...over,
  });
  const labels = (over: object) =>
    previewChips(preview(over) as Parameters<typeof previewChips>[0]).map((c) => [c.label, c.tone]);
  assert.deepEqual(labels({}), [
    ['1.2 KiB', 'neutral'],
    ['Complete', 'success'],
    ['gzip ✓', 'neutral'],
  ]);
  assert.deepEqual(
    labels({
      extent: 'partial',
      stored_bytes: 600,
      decoding: { applied: 'none', integrity: 'not_decoded', truncated: false },
    }),
    [
      ['1.2 KiB', 'neutral'],
      ['600 B of 1.2 KiB stored', 'warning'],
    ],
  );
  assert.deepEqual(
    labels({
      ending: 'unfinished',
      decoding: { applied: 'gzip_magic', integrity: 'failed', truncated: false },
    }).slice(2),
    [
      ['Never ended', 'warning'],
      ['gzip', 'error'],
    ],
  );
  assert.deepEqual(
    labels({ decoding: { applied: 'brotli', integrity: 'verified', truncated: false } }).at(-1),
    ['brotli ✓', 'neutral'],
  );
});

test('spans read at a glance, and session colors are stable', () => {
  assert.deepEqual([45_000, 12 * 60_000, 83 * 60_000, 120 * 60_000].map(span), [
    '45 s',
    '12 min',
    '1 h 23 min',
    '2 h',
  ]);
  const id = 'cecf7cbd-be7f-4563-9d85-3d5bea8fd082';
  assert.equal(sessionTone(id), sessionTone(id));
  assert.ok(sessionTone(id) >= 0 && sessionTone(id) < 8);
});

test('each provider has a name and a mark, and an unknown one is never shown as another', () => {
  assert.deepEqual(
    ['anthropic', 'codex', 'openai', 'deepseek'].map((p) => [providerName(p), providerMark(p)]),
    [
      ['Anthropic', 'A'],
      ['Codex', 'C'],
      ['OpenAI', 'O'],
      ['DeepSeek', 'D'],
    ],
  );
  assert.deepEqual([providerName('future'), providerMark('future')], ['future', '?']);
});

test('a body cut at the capture limit is trimmed by design; any other cut is partial', () => {
  const limit = 4096;
  const trimmed = finishedFixture(callId(1), {
    capture_complete: false,
    request: { observed_bytes: 9000, stored_bytes: limit, ending: 'ended' },
  });
  assert.equal(trimmedAtLimit(trimmed, limit), true);
  assert.deepEqual(
    [captureChip(trimmed, limit).label, captureChip(trimmed, limit).tone],
    ['Trimmed at limit', 'neutral'],
  );
  assert.equal(captureState(trimmed, limit), 'Trimmed at the capture limit');
  // Without the limit (an older daemon) it cannot be told apart, so it stays partial.
  assert.equal(captureChip(trimmed, null).tone, 'warning');
  // Bytes missing short of the limit are loss.
  const lost = finishedFixture(callId(2), {
    capture_complete: false,
    request: { observed_bytes: 9000, stored_bytes: 2000, ending: 'ended' },
  });
  assert.equal(captureChip(lost, limit).label, 'Partial capture');
  // A body that did not end is not trimmed, even at the limit.
  const cut = finishedFixture(callId(3), {
    capture_complete: false,
    request: { observed_bytes: 9000, stored_bytes: limit, ending: 'unfinished' },
  });
  assert.equal(trimmedAtLimit(cut, limit), false);
  assert.equal(trimmedAtLimit(finishedFixture(callId(4)), limit), false);
});

test('a body preview cut at the capture limit says so, without a warning', () => {
  const preview = {
    observed_bytes: 9000,
    stored_bytes: 4096,
    captured_bytes: 4096,
    extent: 'gap',
    ending: 'ended',
    raw: { text: '', format: 'utf8', truncated: false },
    text: '',
    text_end: 'intact',
    decoding: { applied: 'identity', integrity: 'verified', truncated: false },
  };
  const labels = (limit: number | null) =>
    previewChips(preview as Parameters<typeof previewChips>[0], limit).map((chip) => [
      chip.label,
      chip.tone,
    ]);
  assert.deepEqual(labels(4096)[1], ['Trimmed at 4.0 KiB', 'neutral']);
  assert.deepEqual(labels(null)[1], ['Stops at a gap', 'warning']);
});
