import type {
  AttemptView,
  CallDetail,
  CallId,
  CallView,
  ChannelId,
  Health,
  InspectionRecord,
  RecorderMetrics,
  Storage,
  StorageView,
  UnixMillis,
} from '../src/lib/api.ts';

/** A synthetic, valid UUID: `n` in the last group. */
export const uuid = (n: number) =>
  `00000000-0000-4000-8000-${n.toString(16).padStart(12, '0')}` as string;
export const callId = (n: number) => uuid(n) as CallId;
const ms = (value: number) => value as UnixMillis;

export function callFixture(id: CallId = callId(1), overrides: Partial<CallView> = {}): CallView {
  return {
    id,
    channel: uuid(900) as ChannelId,
    started_at: ms(1000),
    ended_at: null,
    provider: 'anthropic',
    endpoint: 'messages',
    method: 'POST',
    config_revision: 'fixture',
    session: null,
    turn: null,
    attribution: { hint: null, issues: [] },
    state: 'open',
    terminal: null,
    status: null,
    model: null,
    outcome: null,
    stop_reason: null,
    tool_names: [],
    usage: null,
    capture_complete: false,
    request: { observed_bytes: null, stored_bytes: 0, ending: 'open' },
    response: null,
    rejection: null,
    operator_revision: null,
    ...overrides,
  };
}

/** A call that finished with the given summary columns. */
export function finishedFixture(
  id: CallId = callId(1),
  overrides: Partial<CallView> = {},
): CallView {
  return callFixture(id, {
    state: 'finished',
    ended_at: ms(2000),
    terminal: 'body_complete',
    status: 200,
    capture_complete: true,
    request: { observed_bytes: 2, stored_bytes: 2, ending: 'ended' },
    response: { observed_bytes: 4, stored_bytes: 4, ending: 'ended' },
    ...overrides,
  });
}

export function inspectionFixture(overrides: Partial<InspectionRecord> = {}): InspectionRecord {
  return {
    coverage: 'complete',
    format: 'declared',
    limitations: [],
    outcome: 'completed',
    terminal_event: 'seen',
    model: 'fixture-model',
    stop_reason: 'end_turn',
    usage: null,
    tool_calls: [],
    unknown_kinds: [],
    events: { understood: 3, unknown: 0, malformed: 0 },
    ...overrides,
  };
}

export function detailFixture(call: CallView, attempts: Partial<AttemptView>[] = [{}]): CallDetail {
  return {
    call,
    transport: 'http',
    channel: {
      id: call.channel,
      transport: 'http',
      opened_at: call.started_at,
      closed_at: null,
      ending: null,
      close_code: null,
      unattributed_messages: 0,
      unattributed_bytes: 0,
    },
    attempts: attempts.map((attempt, index) => ({
      id: uuid(500 + index) as AttemptView['id'],
      ordinal: index,
      started_at: call.started_at,
      ended_at: call.ended_at,
      upstream: 'anthropic',
      status: call.status,
      response_format: 'event_stream',
      response_encoding: 'identity',
      terminal: call.terminal,
      terminal_cause: null,
      terminal_phase: call.terminal ? 'response' : null,
      response: call.response,
      inspection_version: 1,
      inspection: inspectionFixture(),
      ...attempt,
    })),
    observations: [],
    messages: [],
  };
}

export function recorderFixture(overrides: Partial<RecorderMetrics> = {}): RecorderMetrics {
  return {
    queue: { items: 0, capacity: 1024, bytes: 0, byte_capacity: 1 << 26 },
    omissions: {
      policy_limit_bytes: 0,
      queue_bytes_bytes: 0,
      queue_slots_bytes: 0,
      storage_failure_bytes: 0,
      unattributed_bytes: 0,
      unrecorded_calls: 0,
    },
    storage_errors: 0,
    retention_errors: 0,
    pruned_calls: 0,
    pruned_chunks: 0,
    commits: 0,
    last_commit_ms: 0,
    max_commit_ms: 0,
    lag_ms: 0,
    dedup: { referenced_bytes: 0, new_bytes: 0, compressed_bytes: 0 },
    recovered_calls: 0,
    ...overrides,
  };
}

export function storageViewFixture(): StorageView {
  const file = { page_size: 4096, page_count: 10, freelist_count: 2 };
  return {
    disk: { metadata: file, blobs: file },
    calls: 0,
    open_calls: 0,
    digested_calls: 0,
    chunks: 0,
    referenced_bytes: 0,
    unique_bytes: 0,
    compressed_bytes: 0,
  };
}

export const healthFixture = (overrides: Partial<RecorderMetrics> = {}): Health => ({
  status: overrides.storage_errors ? 'degraded' : 'ok',
  build: { version: '0.1.0', revision: { kind: 'unknown' } },
  operator: { pause: { state: 'running' }, blocked_sessions: 0 },
  recorder: recorderFixture(overrides),
});

/** `/health` as the daemon sends it: the build's revision is text. */
export const healthBody = (overrides: Partial<RecorderMetrics> = {}) => ({
  ...healthFixture(overrides),
  build: { version: '0.1.0', revision: 'unknown' },
});

export const storageFixture = (): Storage => ({
  policy: { enabled: true, max_age_seconds: 86400, max_calls: 100, max_storage_bytes: 1024 },
  usage: storageViewFixture(),
});

/** A JSON response in the daemon's versioned envelope. */
export const versioned = (body: object, init?: ResponseInit, version: unknown = 1) =>
  Response.json({ api_version: version, ...body }, init);
