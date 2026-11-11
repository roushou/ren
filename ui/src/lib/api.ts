// Types and boundary parsing for the daemon's read-only management API
// (`crates/ren-daemon/src/api/mod.rs`). Field names match the daemon's JSON.
import {
  Uuid,
  array,
  boolean,
  branded,
  brandedCount,
  count,
  isRecord,
  label,
  nullable,
  object,
  string,
  unrecognised,
  ShapeError,
  type Brand,
  type Decoder,
  type Stored,
} from './decode.ts';

/** The management API version this UI understands. */
export const API_VERSION = 1;

// Identities. Ren's own ids are UUIDs; native client ids keep separate types.
export type CallId = Brand<string, 'CallId'>;
export type AttemptId = Brand<string, 'AttemptId'>;
export type ChannelId = Brand<string, 'ChannelId'>;
export type SessionId = Brand<string, 'SessionId'>;
export type TurnId = Brand<string, 'TurnId'>;
export type NativeSessionId = Brand<string, 'NativeSessionId'>;
export type NativeTurnId = Brand<string, 'NativeTurnId'>;
export type NativeThreadId = Brand<string, 'NativeThreadId'>;
/** An opaque call-listing position; pass back unchanged as `after`. */
export type PageCursor = Brand<string, 'PageCursor'>;
/** Position in the observation log. */
export type ObservationSeq = Brand<number, 'ObservationSeq'>;
/** Wall-clock milliseconds since the Unix epoch. */
export type UnixMillis = Brand<number, 'UnixMillis'>;

export const CallId = new Uuid<'CallId'>();
export const AttemptId = new Uuid<'AttemptId'>();
export const ChannelId = new Uuid<'ChannelId'>();
export const SessionId = new Uuid<'SessionId'>();
export const TurnId = new Uuid<'TurnId'>();

// Vocabularies (the daemon's label enums). Unrecognised text is kept.
const providers = ['anthropic', 'codex', 'openai', 'deepseek'] as const;
export type Provider = (typeof providers)[number];
const endpoints = [
  'messages',
  'count_tokens',
  'models',
  'responses',
  'responses_compact',
  'chat_completions',
] as const;
export type Endpoint = (typeof endpoints)[number];
// `MESSAGE`: a `response.create` on a WebSocket channel.
const methods = ['GET', 'POST', 'MESSAGE'] as const;
export type Method = (typeof methods)[number];
const transports = ['http', 'websocket'] as const;
export type Transport = (typeof transports)[number];
const callStates = ['open', 'finished', 'recovered'] as const;
export type CallState = (typeof callStates)[number];
const attemptTerminals = [
  'body_complete',
  // A WebSocket call's terminal event arrived.
  'terminal_event',
  'not_sent',
  // An HTTP/2 stream refused before processing; safe to send elsewhere.
  'refused',
  'acceptance_unknown',
  'stream_failed',
  'cancelled',
  'interrupted',
] as const;
export type AttemptTerminal = (typeof attemptTerminals)[number];
const terminalCauses = [
  'connect_failed',
  'stream_refused',
  'header_timeout',
  'send_failed',
  'idle_timeout',
  'body_error',
  'upstream_closed',
  'client',
  'shutdown',
  // The Ren operator cancelled the call.
  'operator',
  // With body_complete: the declared content-length was relayed whole before
  // the stream's own end was observed.
  'declared_length',
] as const;
export type TerminalCause = (typeof terminalCauses)[number];
const attemptPhases = ['upload', 'awaiting_head', 'response'] as const;
/** Why Ren answered a call without forwarding it. */
const callPurposes = ['work', 'quota_probe'] as const;
/** Why a client sent a call: its session's work, or a check of its own. */
export type CallPurpose = (typeof callPurposes)[number];
const callRejections = [
  'too_large',
  'buffer_full',
  'upload_timeout',
  'upload_failed',
  'abandoned',
  'paused',
  'session_blocked',
  'user_blocked',
  'blocked',
  'shutdown',
  'identity_refused',
  'user_not_allowed',
  'user_busy',
] as const;
export type CallRejection = (typeof callRejections)[number];
/** How a call's caller was identified (see the daemon's named users). */
const callerKinds = ['user_tailscale', 'user_token', 'unlisted', 'unidentified'] as const;
export type CallerKind = (typeof callerKinds)[number];
/** Who made a call: a named user, a Tailscale login no user lists, or nobody Ren can identify. */
export interface CallerView {
  kind: Stored<CallerKind>;
  /** The named user it proved to be. */
  user: string | null;
  /** The Tailscale login the request carried. */
  login: string | null;
}
export type AttemptPhase = (typeof attemptPhases)[number];
const bodyEndings = ['open', 'ended', 'unfinished'] as const;
export type BodyEnding = (typeof bodyEndings)[number];
const modelOutcomes = [
  'completed',
  'incomplete',
  'refused',
  'cancelled',
  'failed',
  'unknown',
] as const;
export type ModelOutcome = (typeof modelOutcomes)[number];
const hintSources = [
  'claude_code_headers',
  'codex_headers',
  'pi_extension_headers',
  'codex_message_metadata',
  'opencode_headers',
] as const;
export type HintSource = (typeof hintSources)[number];
const hintIssues = [
  'invalid_or_duplicate_header',
  'invalid_turn_metadata',
  'conflicting_identifiers',
  'turn_without_session',
] as const;
export type HintIssue = (typeof hintIssues)[number];
const inputAccountings = ['includes_cache', 'excludes_cache'] as const;
export type InputAccounting = (typeof inputAccountings)[number];
const coverages = ['complete', 'partial', 'unavailable'] as const;
export type Coverage = (typeof coverages)[number];
const limitations = [
  'unsupported_format',
  'unsupported_encoding',
  'capture_gap',
  'body_unfinished',
  'incomplete_event',
  'malformed_event',
  'malformed_field',
  'event_too_large',
  'document_too_large',
  'decoded_too_large',
  'decoding_error',
  'too_many_tool_calls',
  'too_many_unknown_kinds',
  'no_terminal_event',
] as const;
export type Limitation = (typeof limitations)[number];
const formatSources = ['declared', 'inferred'] as const;
export type FormatSource = (typeof formatSources)[number];
const terminalEvents = ['seen', 'missing'] as const;
export type TerminalEvent = (typeof terminalEvents)[number];
const kindFamilies = [
  'event',
  'document',
  'content_block',
  'output_item',
  'stop_reason',
  'status',
  'incomplete_reason',
] as const;
export type KindFamily = (typeof kindFamilies)[number];
const responseFormats = [
  'event_stream',
  'json',
  'other',
  'unspecified',
  'websocket_messages',
] as const;
export type ResponseFormat = (typeof responseFormats)[number];
const responseEncodings = ['identity', 'gzip', 'zstd', 'brotli', 'unsupported'] as const;
export type ResponseEncoding = (typeof responseEncodings)[number];
const observationKinds = [
  'call_opened',
  'attempt_started',
  'response_head',
  'attempt_finished',
  'call_finished',
  'call_recovered',
  'channel_closed',
  'operator_action',
] as const;
const channelEnds = [
  'client_closed',
  'upstream_closed',
  'client_failed',
  'upstream_failed',
  'client_unresponsive',
  'upstream_unresponsive',
  'upstream_stalled',
  'drained',
  'aborted',
  'dropped',
  'recovered',
] as const;
export type ChannelEnd = (typeof channelEnds)[number];
const messagePayloads = ['text', 'binary'] as const;
export type MessagePayload = (typeof messagePayloads)[number];
export type ObservationKind = (typeof observationKinds)[number];
const healthStatuses = ['ok', 'degraded'] as const;
/** What the gateway says it is: the installed service, or a development one. */
const environments = ['production', 'development'] as const;
export type Environment = (typeof environments)[number];
export type HealthStatus = (typeof healthStatuses)[number];
const errorCodes = ['not_found', 'invalid_query', 'busy', 'unavailable'] as const;
export type ErrorCode = (typeof errorCodes)[number];
const previewExtents = ['complete', 'partial', 'truncated', 'gap'] as const;
export type PreviewExtent = (typeof previewExtents)[number];
const rawFormats = ['utf8', 'hex'] as const;
export type RawFormat = (typeof rawFormats)[number];
const textEnds = ['intact', 'split_character'] as const;
export type TextEnd = (typeof textEnds)[number];
const previewDecodings = ['none', 'gzip_magic', 'zstd_magic', 'brotli'] as const;
export type PreviewDecoding = (typeof previewDecodings)[number];
const decodeIntegrities = ['not_decoded', 'verified', 'prefix', 'failed'] as const;
export type DecodeIntegrity = (typeof decodeIntegrities)[number];

export type Direction = 'request' | 'response';

// Views (`crates/ren-store/src/view.rs`).
export interface BodyView {
  /** Unknown while the body is open, or after a restart. */
  observed_bytes: number | null;
  stored_bytes: number;
  ending: Stored<BodyEnding>;
}
export interface SessionHint {
  source: Stored<HintSource>;
  session: NativeSessionId;
  turn: NativeTurnId | null;
  thread: NativeThreadId | null;
}
export interface Attribution {
  hint: SessionHint | null;
  issues: Stored<HintIssue>[];
}
export interface Usage {
  accounting: Stored<InputAccounting>;
  input_tokens: number | null;
  output_tokens: number | null;
  cache_read_tokens: number | null;
  cache_creation_tokens: number | null;
  reasoning_tokens: number | null;
}
/** One client request, with summary columns copied from its final attempt. */
export interface CallView {
  id: CallId;
  channel: ChannelId;
  started_at: UnixMillis;
  ended_at: UnixMillis | null;
  provider: Stored<Provider>;
  endpoint: Stored<Endpoint>;
  method: Stored<Method>;
  config_revision: string;
  session: SessionId | null;
  turn: TurnId | null;
  attribution: Attribution;
  state: Stored<CallState>;
  terminal: Stored<AttemptTerminal> | null;
  status: number | null;
  model: string | null;
  outcome: Stored<ModelOutcome> | null;
  stop_reason: string | null;
  tool_names: string[];
  usage: Usage | null;
  /** Both bodies ended and were stored without omissions. */
  capture_complete: boolean;
  request: BodyView | null;
  response: BodyView | null;
  /** Set when Ren answered without forwarding the call. */
  rejection: Stored<CallRejection> | null;
  /** The operator state revision the call was planned against. */
  operator_revision: number | null;
  /** Why the client sent it, once derived; null before then, or from an older daemon. */
  purpose: Stored<CallPurpose> | null;
  /** Null for calls recorded before callers were, or from an older daemon. */
  caller: CallerView | null;
}
export interface ToolCallRecord {
  call_id: string | null;
  item_id: string | null;
  kind: string;
  name: string | null;
}
export interface InspectionRecord {
  coverage: Stored<Coverage>;
  format: Stored<FormatSource>;
  limitations: Stored<Limitation>[];
  outcome: Stored<ModelOutcome> | null;
  terminal_event: Stored<TerminalEvent>;
  model: string | null;
  stop_reason: string | null;
  usage: Usage | null;
  tool_calls: ToolCallRecord[];
  unknown_kinds: { family: Stored<KindFamily>; kind: string; count: number }[];
  events: { understood: number; unknown: number; malformed: number };
}
/** One upstream try of a call. */
export interface AttemptView {
  id: AttemptId;
  ordinal: number;
  started_at: UnixMillis;
  ended_at: UnixMillis | null;
  /** The upstream entry's id: a provider's own entry is named after it. */
  upstream: string;
  status: number | null;
  response_format: Stored<ResponseFormat> | null;
  response_encoding: Stored<ResponseEncoding> | null;
  terminal: Stored<AttemptTerminal> | null;
  terminal_cause: Stored<TerminalCause> | null;
  terminal_phase: Stored<AttemptPhase> | null;
  response: BodyView | null;
  inspection_version: number | null;
  inspection: InspectionRecord | null;
}
export interface ObservationView {
  seq: ObservationSeq;
  at: UnixMillis;
  kind: Stored<ObservationKind>;
  channel: ChannelId;
  call: CallId | null;
  attempt: AttemptId | null;
}
/** The connection that carried a call; for WebSocket, shared by its calls. */
export interface ChannelView {
  id: ChannelId;
  transport: Stored<Transport>;
  opened_at: UnixMillis;
  /** WebSocket only; absent while the channel is open. */
  closed_at: UnixMillis | null;
  ending: Stored<ChannelEnd> | null;
  close_code: number | null;
  /** Messages forwarded while no call was open: counted, not stored. */
  unattributed_messages: number;
  unattributed_bytes: number;
}
/** Where one WebSocket message sits inside a body. */
export interface MessageView {
  direction: Stored<Direction>;
  offset: number;
  len: number;
  payload: Stored<MessagePayload>;
  at: UnixMillis;
}
export interface CallDetail {
  call: CallView;
  transport: Stored<Transport>;
  channel: ChannelView;
  attempts: AttemptView[];
  observations: ObservationView[];
  /** WebSocket message boundaries, request then response; empty for HTTP. */
  messages: MessageView[];
}
export interface CallPage {
  calls: CallView[];
  next: PageCursor | null;
}
export interface SessionView {
  id: SessionId;
  provider: Stored<Provider>;
  source: Stored<HintSource>;
  native_id: NativeSessionId;
  first_seen: UnixMillis;
  last_seen: UnixMillis;
  call_count: number;
  turn_count: number;
  /** Null from a daemon older than session totals. */
  error_count: number | null;
  /** Input includes cached context; null when no call reported usage. */
  tokens: TokenTotals;
}
/** Provider-reported tokens summed over calls; null when none reported them. */
export interface TokenTotals {
  /** Includes cached context, whichever way the provider accounts it. */
  input: number | null;
  output: number | null;
  /** The part of input read from the provider's cache. */
  cache_read: number | null;
}
export interface UsageCounts {
  calls: number;
  errors: number;
  tokens: TokenTotals;
}
export interface UsageBucket extends UsageCounts {
  /** The hour's start, UTC. */
  start: UnixMillis;
}
export interface ModelUsage extends UsageCounts {
  provider: Stored<Provider>;
  /** Null for calls whose response reported no model. */
  model: string | null;
}
export interface UserUsage extends UsageCounts {
  /** Null for calls no named user made. */
  user: string | null;
}
export interface UsageView {
  totals: UsageCounts;
  /** Hours with calls, oldest first. */
  hours: UsageBucket[];
  models: ModelUsage[];
  /** Empty from daemons older than named users. */
  users: UserUsage[];
}
export interface FileUsage {
  page_size: number;
  page_count: number;
  freelist_count: number;
}
export interface StorageView {
  /** The search index is part of the metadata file. */
  disk: { metadata: FileUsage; blobs: FileUsage };
  calls: number;
  open_calls: number;
  /** Calls whose activity is derived: every finished call, once derived. */
  digested_calls: number;
  chunks: number;
  /** Raw bytes all stored segments reference. */
  referenced_bytes: number;
  /** Raw bytes of the distinct chunks. */
  unique_bytes: number;
  /** Bytes the chunks occupy after compression. */
  compressed_bytes: number;
}
export interface RetentionPolicy {
  enabled: boolean;
  max_age_seconds: number;
  max_calls: number;
  max_storage_bytes: number;
}
export interface RecorderMetrics {
  queue: { items: number; capacity: number; bytes: number; byte_capacity: number };
  omissions: {
    policy_limit_bytes: number;
    queue_bytes_bytes: number;
    queue_slots_bytes: number;
    storage_failure_bytes: number;
    unattributed_bytes: number;
    unrecorded_calls: number;
  };
  storage_errors: number;
  retention_errors: number;
  pruned_calls: number;
  pruned_chunks: number;
  commits: number;
  last_commit_ms: number;
  max_commit_ms: number;
  lag_ms: number;
  dedup: { referenced_bytes: number; new_bytes: number; compressed_bytes: number };
  recovered_calls: number;
}
/** What a daemon binary was compiled from (`/health` `build.revision`). */
export type Revision =
  | { kind: 'commit'; commit: string; tree: 'clean' | 'modified' }
  | { kind: 'unknown' }
  /** Text this UI does not recognize, kept for display. */
  | { kind: 'unrecognized'; text: string };
export interface Build {
  version: string;
  revision: Revision;
}
/** Whether new calls are admitted; reported read-only by `/health`. */
export type Pause =
  | { state: 'running' }
  | { state: 'paused'; retry_after: number }
  | { state: 'unknown'; text: string };
export interface OperatorSummary {
  pause: Pause;
  /** Session blocks in force now. */
  blocked_sessions: number;
}
export interface Health {
  status: Stored<HealthStatus>;
  build: Build;
  /** Production for daemons that predate the field. */
  instance: { environment: Stored<Environment> };
  operator: OperatorSummary;
  recorder: RecorderMetrics;
}
export interface Metrics {
  recorder: RecorderMetrics;
  storage: StorageView;
}
export interface Storage {
  policy: RetentionPolicy;
  usage: StorageView;
  /** What capture keeps of each body; null from a daemon that does not say. */
  capture: CaptureLimits | null;
}
export interface CaptureLimits {
  /** Bytes stored per body; past this, the rest is left out by design. */
  body_limit_bytes: number;
}
export interface Sessions {
  sessions: SessionView[];
}
/** A bounded preview of a stored body (`crates/ren-daemon/src/api/preview.rs`). */
export interface BodyPreview {
  observed_bytes: number | null;
  stored_bytes: number;
  ending: Stored<BodyEnding>;
  captured_bytes: number;
  extent: Stored<PreviewExtent>;
  raw: { text: string; format: Stored<RawFormat>; truncated: boolean };
  text: string | null;
  text_end: Stored<TextEnd>;
  decoding: {
    applied: Stored<PreviewDecoding>;
    integrity: Stored<DecodeIntegrity>;
    truncated: boolean;
  };
}

// Decoders, one per view.
const millis = brandedCount<'UnixMillis'>();
const bodyView = object<BodyView>({
  observed_bytes: nullable(count),
  stored_bytes: count,
  ending: label(bodyEndings),
});
const usage = object<Usage>({
  accounting: label(inputAccountings),
  input_tokens: nullable(count),
  output_tokens: nullable(count),
  cache_read_tokens: nullable(count),
  cache_creation_tokens: nullable(count),
  reasoning_tokens: nullable(count),
});
const callView = object<CallView>({
  id: CallId.decode,
  channel: ChannelId.decode,
  started_at: millis,
  ended_at: nullable(millis),
  provider: label(providers),
  endpoint: label(endpoints),
  method: label(methods),
  config_revision: string,
  session: nullable(SessionId.decode),
  turn: nullable(TurnId.decode),
  attribution: object<Attribution>({
    hint: nullable(
      object<SessionHint>({
        source: label(hintSources),
        session: branded<'NativeSessionId'>(),
        turn: nullable(branded<'NativeTurnId'>()),
        thread: nullable(branded<'NativeThreadId'>()),
      }),
    ),
    issues: array(label(hintIssues)),
  }),
  state: label(callStates),
  terminal: nullable(label(attemptTerminals)),
  status: nullable(count),
  model: nullable(string),
  outcome: nullable(label(modelOutcomes)),
  stop_reason: nullable(string),
  tool_names: array(string),
  usage: nullable(usage),
  capture_complete: boolean,
  request: nullable(bodyView),
  response: nullable(bodyView),
  rejection: nullable(label(callRejections)),
  operator_revision: nullable(count),
  // Absent from daemons older than the field: read as not derived.
  purpose: (value, path) =>
    value === undefined || value === null ? null : label(callPurposes)(value, path),
  // Absent from daemons older than the field: an unknown caller.
  caller: (value, path) =>
    value === undefined || value === null
      ? null
      : object<CallerView>({
          kind: label(callerKinds),
          user: nullable(string),
          login: nullable(string),
        })(value, path),
});
const inspection = object<InspectionRecord>({
  coverage: label(coverages),
  format: label(formatSources),
  limitations: array(label(limitations)),
  outcome: nullable(label(modelOutcomes)),
  terminal_event: label(terminalEvents),
  model: nullable(string),
  stop_reason: nullable(string),
  usage: nullable(usage),
  tool_calls: array(
    object<ToolCallRecord>({
      call_id: nullable(string),
      item_id: nullable(string),
      kind: string,
      name: nullable(string),
    }),
  ),
  unknown_kinds: array(
    object<InspectionRecord['unknown_kinds'][number]>({
      family: label(kindFamilies),
      kind: string,
      count,
    }),
  ),
  events: object<InspectionRecord['events']>({
    understood: count,
    unknown: count,
    malformed: count,
  }),
});
export const observationView = object<ObservationView>({
  seq: brandedCount<'ObservationSeq'>(),
  at: millis,
  kind: label(observationKinds),
  channel: ChannelId.decode,
  call: nullable(CallId.decode),
  attempt: nullable(AttemptId.decode),
});
const callDetail = object<CallDetail>({
  call: callView,
  transport: label(transports),
  channel: object<ChannelView>({
    id: ChannelId.decode,
    transport: label(transports),
    opened_at: millis,
    closed_at: nullable(millis),
    ending: nullable(label(channelEnds)),
    close_code: nullable(count),
    unattributed_messages: count,
    unattributed_bytes: count,
  }),
  attempts: array(
    object<AttemptView>({
      id: AttemptId.decode,
      ordinal: count,
      started_at: millis,
      ended_at: nullable(millis),
      upstream: string,
      status: nullable(count),
      response_format: nullable(label(responseFormats)),
      response_encoding: nullable(label(responseEncodings)),
      terminal: nullable(label(attemptTerminals)),
      terminal_cause: nullable(label(terminalCauses)),
      terminal_phase: nullable(label(attemptPhases)),
      response: nullable(bodyView),
      inspection_version: nullable(count),
      inspection: nullable(inspection),
    }),
  ),
  observations: array(observationView),
  messages: array(
    object<MessageView>({
      direction: label(['request', 'response'] as const),
      offset: count,
      len: count,
      payload: label(messagePayloads),
      at: millis,
    }),
  ),
});
const callPage = object<CallPage>({
  calls: array(callView),
  next: nullable(branded<'PageCursor'>()),
});
function tokenTotals(value: unknown, path: string): TokenTotals {
  return object<TokenTotals>({
    input: nullable(count),
    output: nullable(count),
    cache_read: nullable(count),
  })(value, path);
}
const sessions = object<Sessions>({
  sessions: array(
    object<SessionView>({
      id: SessionId.decode,
      provider: label(providers),
      source: label(hintSources),
      native_id: branded<'NativeSessionId'>(),
      first_seen: millis,
      last_seen: millis,
      call_count: count,
      turn_count: count,
      error_count: count,
      tokens: tokenTotals,
    }),
  ),
});
const usageCounts = {
  calls: count,
  errors: count,
  tokens: (value: unknown, path: string) => tokenTotals(value, path),
};
const usageView = object<UsageView>({
  totals: object<UsageCounts>(usageCounts),
  hours: array(object<UsageBucket>({ start: millis, ...usageCounts })),
  models: array(
    object<ModelUsage>({
      provider: label(providers),
      model: nullable(string),
      ...usageCounts,
    }),
  ),
  users: (value, path) =>
    value === undefined
      ? []
      : array(object<UserUsage>({ user: nullable(string), ...usageCounts }))(value, path),
});
const fileUsage = object<FileUsage>({ page_size: count, page_count: count, freelist_count: count });
const storageView = object<StorageView>({
  disk: object<StorageView['disk']>({
    metadata: fileUsage,
    blobs: fileUsage,
  }),
  calls: count,
  open_calls: count,
  digested_calls: count,
  chunks: count,
  referenced_bytes: count,
  unique_bytes: count,
  compressed_bytes: count,
});
const recorderMetrics = object<RecorderMetrics>({
  queue: object<RecorderMetrics['queue']>({
    items: count,
    capacity: count,
    bytes: count,
    byte_capacity: count,
  }),
  omissions: object<RecorderMetrics['omissions']>({
    policy_limit_bytes: count,
    queue_bytes_bytes: count,
    queue_slots_bytes: count,
    storage_failure_bytes: count,
    unattributed_bytes: count,
    unrecorded_calls: count,
  }),
  storage_errors: count,
  retention_errors: count,
  pruned_calls: count,
  pruned_chunks: count,
  commits: count,
  last_commit_ms: count,
  max_commit_ms: count,
  lag_ms: count,
  dedup: object<RecorderMetrics['dedup']>({
    referenced_bytes: count,
    new_bytes: count,
    compressed_bytes: count,
  }),
  recovered_calls: count,
});
const commitPattern = /^(?:[0-9a-f]{40}|[0-9a-f]{64})$/;
/** `<commit>`, `<commit>-dirty`, or `unknown`; anything else is kept as text. */
export const revision: Decoder<Revision> = (value, path) => {
  const text = string(value, path);
  if (text === 'unknown') return { kind: 'unknown' };
  const dirty = text.endsWith('-dirty');
  const commit = dirty ? text.slice(0, -'-dirty'.length) : text;
  return commitPattern.test(commit)
    ? { kind: 'commit', commit, tree: dirty ? 'modified' : 'clean' }
    : { kind: 'unrecognized', text };
};
const build = object<Build>({ version: string, revision });
const pause: Decoder<Pause> = (value, path) => {
  if (!isRecord(value)) throw new ShapeError(path, 'an object');
  const state = string(value.state, `${path}.state`);
  if (state === 'running') return { state };
  if (state === 'paused')
    return { state, retry_after: count(value.retry_after, `${path}.retry_after`) };
  return { state: 'unknown', text: state };
};
const operatorSummary = object<OperatorSummary>({ pause, blocked_sessions: count });
const health = object<Health>({
  status: label(healthStatuses),
  build,
  instance: (value, path) =>
    value === undefined
      ? { environment: 'production' }
      : object<Health['instance']>({ environment: label(environments) })(value, path),
  operator: operatorSummary,
  recorder: recorderMetrics,
});
const metrics = object<Metrics>({ recorder: recorderMetrics, storage: storageView });
const storage = object<Storage>({
  policy: object<RetentionPolicy>({
    enabled: boolean,
    max_age_seconds: count,
    max_calls: count,
    max_storage_bytes: count,
  }),
  usage: storageView,
  // Absent from daemons older than the field: read as not reported.
  capture: (value, path) =>
    value === undefined || value === null
      ? null
      : object<CaptureLimits>({ body_limit_bytes: count })(value, path),
});
const bodyPreview = object<BodyPreview>({
  observed_bytes: nullable(count),
  stored_bytes: count,
  ending: label(bodyEndings),
  captured_bytes: count,
  extent: label(previewExtents),
  raw: object<BodyPreview['raw']>({
    text: string,
    format: label(rawFormats),
    truncated: boolean,
  }),
  text: nullable(string),
  text_end: label(textEnds),
  decoding: object<BodyPreview['decoding']>({
    applied: label(previewDecodings),
    integrity: label(decodeIntegrities),
    truncated: boolean,
  }),
});
const errorBody = object<{ error: Stored<ErrorCode> }>({ error: label(errorCodes) });

/** The daemon speaks a management API version this UI does not understand. */
export class ApiVersionError extends Error {
  readonly found: unknown;
  constructor(found: unknown) {
    super(
      `Incompatible Ren daemon: it reported management API version ${
        typeof found === 'number' || typeof found === 'string' ? found : 'none'
      }, but this UI requires version ${API_VERSION}. Update the UI or the daemon so they match.`,
    );
    this.found = found;
  }
}

export class ApiError extends Error {
  status: number;
  code: Stored<ErrorCode> | null;
  constructor(status: number, code: Stored<ErrorCode> | null = null) {
    super(
      status === 502 || status === 504
        ? 'Cannot reach Ren. Check that the daemon is running and the UI gateway URL is correct.'
        : status === 503
          ? code === 'unavailable'
            ? 'Ren storage is unavailable. Check the daemon logs.'
            : 'The history reader is busy. Try again in a moment.'
          : status === 404
            ? 'This record is no longer available. It may have expired.'
            : status === 400
              ? 'Invalid filters or page cursor. Check the filters or return to the first page.'
              : `Request failed (${status}).`,
    );
    this.status = status;
    this.code = code;
  }
}

// Activity and reports: what a turn or call did, read from recorded bodies,
// and where a session's context and cache tokens went.
const harnesses = ['claude_code', 'codex', 'pi', 'opencode'] as const;
export type Harness = (typeof harnesses)[number];
export const HARNESSES = harnesses;
const effectKinds = [
  'read_file',
  'write_file',
  'run_command',
  'search',
  'delegate',
  'fetch',
] as const;
export type EffectKind = (typeof effectKinds)[number];
const resultStatuses = ['succeeded', 'failed', 'unreported'] as const;
export type ResultStatus = (typeof resultStatuses)[number];
const activityLimitations = [
  'unsupported_endpoint',
  'capture_gap',
  'truncated',
  'unsupported_encoding',
  'decoding_error',
  'decoded_too_large',
  'not_json',
  'unexpected_shape',
  'unfinished_tool_call',
  'too_many_tool_calls',
  'not_yet_derived',
] as const;
export type ActivityLimitation = (typeof activityLimitations)[number];
const findingKinds = [
  'cache_rebuild',
  'context_growth',
  'large_tool_output',
  'repeated_failure',
  'secret_exposure',
] as const;
const secretKinds = [
  'anthropic_key',
  'openai_key',
  'github_token',
  'aws_access_key',
  'google_api_key',
  'slack_token',
  'stripe_live_key',
  'private_key',
  'jwt',
] as const;
export type SecretKind = (typeof secretKinds)[number];
export type FindingKind = (typeof findingKinds)[number];

/** Text cut to a budget; `omitted_chars` says how much was left out. */
export interface Excerpt {
  text: string;
  omitted_chars: number;
}
export interface ToolEffect {
  kind: Stored<EffectKind>;
  /** The path, command, pattern, task, or URL the tool's input names. */
  target: string;
}
/** A tool result as the client reported it. */
export interface ToolResultView {
  status: Stored<ResultStatus>;
  output: Excerpt;
}
/** A tool call the model requested: requested, not proof that it ran. */
export interface ToolCallActivity {
  id: string | null;
  name: string;
  effects: ToolEffect[];
  input: Excerpt;
  /** Null when no later request in the turn answered it. */
  result: ToolResultView | null;
}
/** Provider-reported tokens and recorded sizes and times; not cost. */
export interface CallMetrics {
  context_tokens: number | null;
  cache_read_tokens: number | null;
  cache_written_tokens: number | null;
  output_tokens: number | null;
  request_bytes: number | null;
  duration_ms: number | null;
}
export interface CallActivity {
  call: CallId;
  started_at: UnixMillis;
  ended_at: UnixMillis | null;
  model: string | null;
  status: number | null;
  outcome: Stored<ModelOutcome> | null;
  metrics: CallMetrics;
  prompt: Excerpt | null;
  tool_calls: ToolCallActivity[];
  /** Results this request carried for tool calls outside the turn. */
  earlier_results: number;
  reply: Excerpt | null;
  limitations: Stored<ActivityLimitation>[];
}
export type Finding =
  | {
      kind: 'cache_rebuild';
      call: CallId;
      turn: TurnId | null;
      cache_read_tokens: number;
      cache_written_tokens: number;
      earlier_peak_tokens: number;
      idle_before_ms: number | null;
    }
  | {
      kind: 'context_growth';
      call: CallId;
      turn: TurnId | null;
      earlier_peak_tokens: number;
      context_tokens: number;
    }
  | {
      kind: 'large_tool_output';
      call: CallId;
      tool: string;
      tool_call_id: string | null;
      target: string | null;
      output_chars: number;
    }
  | {
      kind: 'repeated_failure';
      tool: string;
      target: string | null;
      failures: number;
      calls: CallId[];
    }
  | {
      kind: 'secret_exposure';
      call: CallId;
      turn: TurnId | null;
      field: Stored<SearchField>;
      /** The credential's format; the secret itself is never sent. */
      secret: Stored<SecretKind>;
      hint: string;
      fingerprint: string;
      count: number;
    }
  /** A finding a newer daemon reports, kept by its label for display. */
  | { kind: 'unrecognized'; text: string };
export interface TurnActivity {
  turn: TurnId;
  session: SessionId;
  native_id: NativeTurnId;
  client: Stored<Harness> | null;
  started_at: UnixMillis | null;
  ended_at: UnixMillis | null;
  prompt: Excerpt | null;
  files_read: string[];
  files_written: string[];
  commands: string[];
  tool_calls: number;
  failed_tool_calls: number;
  unanswered_tool_calls: number;
  findings: Finding[];
  calls: CallActivity[];
  omitted_calls: number;
}
export interface TurnSummary {
  turn: TurnId;
  native_id: NativeTurnId;
  started_at: UnixMillis;
  ended_at: UnixMillis | null;
  call_count: number;
  error_count: number;
  tokens: TokenTotals;
  prompt: Excerpt | null;
}
export interface Turns {
  turns: TurnSummary[];
}
export interface TurnMetrics {
  turn: TurnId;
  started_at: UnixMillis;
  ended_at: UnixMillis | null;
  call_count: number;
  idle_before_ms: number | null;
  context_start_tokens: number | null;
  context_end_tokens: number | null;
  cache_hit_percent: number | null;
  cache_written_tokens: number;
  output_tokens: number;
}
export interface ReportTotals {
  calls: number;
  calls_with_usage: number;
  peak_context_tokens: number;
  context_tokens: number;
  cache_read_tokens: number;
  cache_written_tokens: number;
  output_tokens: number;
  cache_hit_percent: number | null;
}
export interface SessionReport {
  session: SessionId;
  totals: ReportTotals;
  turns: TurnMetrics[];
  findings: Finding[];
  omitted_calls: number;
}

const excerpt = object<Excerpt>({ text: string, omitted_chars: count });
const callMetrics = object<CallMetrics>({
  context_tokens: nullable(count),
  cache_read_tokens: nullable(count),
  cache_written_tokens: nullable(count),
  output_tokens: nullable(count),
  request_bytes: nullable(count),
  duration_ms: nullable(count),
});
const callActivity = object<CallActivity>({
  call: CallId.decode,
  started_at: millis,
  ended_at: nullable(millis),
  model: nullable(string),
  status: nullable(count),
  outcome: nullable(label(modelOutcomes)),
  metrics: callMetrics,
  prompt: nullable(excerpt),
  tool_calls: array(
    object<ToolCallActivity>({
      id: nullable(string),
      name: string,
      effects: array(object<ToolEffect>({ kind: label(effectKinds), target: string })),
      input: excerpt,
      result: nullable(object<ToolResultView>({ status: label(resultStatuses), output: excerpt })),
    }),
  ),
  earlier_results: count,
  reply: nullable(excerpt),
  limitations: array(label(activityLimitations)),
});
/** A finding by its `kind`; an unknown kind keeps only its label. */
const finding: Decoder<Finding> = (value, path) => {
  const kind = label(findingKinds)(isRecord(value) ? value.kind : undefined, `${path}.kind`);
  switch (kind) {
    case 'cache_rebuild':
      return object<Extract<Finding, { kind: 'cache_rebuild' }>>({
        kind: () => 'cache_rebuild',
        call: CallId.decode,
        turn: nullable(TurnId.decode),
        cache_read_tokens: count,
        cache_written_tokens: count,
        earlier_peak_tokens: count,
        idle_before_ms: nullable(count),
      })(value, path);
    case 'context_growth':
      return object<Extract<Finding, { kind: 'context_growth' }>>({
        kind: () => 'context_growth',
        call: CallId.decode,
        turn: nullable(TurnId.decode),
        earlier_peak_tokens: count,
        context_tokens: count,
      })(value, path);
    case 'large_tool_output':
      return object<Extract<Finding, { kind: 'large_tool_output' }>>({
        kind: () => 'large_tool_output',
        call: CallId.decode,
        tool: string,
        tool_call_id: nullable(string),
        target: nullable(string),
        output_chars: count,
      })(value, path);
    case 'repeated_failure':
      return object<Extract<Finding, { kind: 'repeated_failure' }>>({
        kind: () => 'repeated_failure',
        tool: string,
        target: nullable(string),
        failures: count,
        calls: array(CallId.decode),
      })(value, path);
    case 'secret_exposure':
      return object<Extract<Finding, { kind: 'secret_exposure' }>>({
        kind: () => 'secret_exposure',
        call: CallId.decode,
        turn: nullable(TurnId.decode),
        field: label(searchFields),
        secret: label(secretKinds),
        hint: string,
        fingerprint: string,
        count: count,
      })(value, path);
    default:
      return { kind: 'unrecognized', text: unrecognised(kind) };
  }
};
const turnActivity = object<TurnActivity>({
  turn: TurnId.decode,
  session: SessionId.decode,
  native_id: branded<'NativeTurnId'>(),
  client: nullable(label(harnesses)),
  started_at: nullable(millis),
  ended_at: nullable(millis),
  prompt: nullable(excerpt),
  files_read: array(string),
  files_written: array(string),
  commands: array(string),
  tool_calls: count,
  failed_tool_calls: count,
  unanswered_tool_calls: count,
  findings: array(finding),
  calls: array(callActivity),
  omitted_calls: count,
});
const turns = object<Turns>({
  turns: array(
    object<TurnSummary>({
      turn: TurnId.decode,
      native_id: branded<'NativeTurnId'>(),
      started_at: millis,
      ended_at: nullable(millis),
      call_count: count,
      error_count: count,
      tokens: (value, path) => tokenTotals(value, path),
      prompt: nullable(excerpt),
    }),
  ),
});
const searchFields = ['prompt', 'reply', 'tool_input', 'tool_output'] as const;
export type SearchField = (typeof searchFields)[number];
export const SEARCH_FIELDS = searchFields;
/** A `?field=` value: a known search field, or null for any other text or none. */
export function parseSearchField(text: string | null): SearchField | null {
  return searchFields.find((field) => field === text) ?? null;
}
export interface SearchHit {
  call: CallId;
  session: SessionId | null;
  turn: TurnId | null;
  client: Stored<Harness> | null;
  started_at: UnixMillis;
  field: Stored<SearchField>;
  /** The text around the match, matched terms marked «like this». */
  snippet: string;
}
export interface SearchPage {
  hits: SearchHit[];
  /** Calls indexed so far; fewer than retained while the daemon backfills. */
  indexed_calls: number;
}
const searchPage = object<SearchPage>({
  hits: array(
    object<SearchHit>({
      call: CallId.decode,
      session: nullable(SessionId.decode),
      turn: nullable(TurnId.decode),
      client: nullable(label(harnesses)),
      started_at: millis,
      field: label(searchFields),
      snippet: string,
    }),
  ),
  indexed_calls: count,
});
/** A credential one call carried: never the secret itself. */
export interface ExposureView {
  call: CallId;
  session: SessionId | null;
  turn: TurnId | null;
  client: Stored<Harness> | null;
  started_at: UnixMillis;
  field: Stored<SearchField>;
  kind: Stored<SecretKind>;
  hint: string;
  fingerprint: string;
  count: number;
}
export interface Exposures {
  exposures: ExposureView[];
}
const exposures = object<Exposures>({
  exposures: array(
    object<ExposureView>({
      call: CallId.decode,
      session: nullable(SessionId.decode),
      turn: nullable(TurnId.decode),
      client: nullable(label(harnesses)),
      started_at: millis,
      field: label(searchFields),
      kind: label(secretKinds),
      hint: string,
      fingerprint: string,
      count: count,
    }),
  ),
});
const sessionReport = object<SessionReport>({
  session: SessionId.decode,
  totals: object<ReportTotals>({
    calls: count,
    calls_with_usage: count,
    peak_context_tokens: count,
    context_tokens: count,
    cache_read_tokens: count,
    cache_written_tokens: count,
    output_tokens: count,
    cache_hit_percent: nullable(count),
  }),
  turns: array(
    object<TurnMetrics>({
      turn: TurnId.decode,
      started_at: millis,
      ended_at: nullable(millis),
      call_count: count,
      idle_before_ms: nullable(count),
      context_start_tokens: nullable(count),
      context_end_tokens: nullable(count),
      cache_hit_percent: nullable(count),
      cache_written_tokens: count,
      output_tokens: count,
    }),
  ),
  findings: array(finding),
  omitted_calls: count,
});

/** Check the version envelope, then decode the fields beside it. */
export const versioned =
  <T>(decode: Decoder<T>): Decoder<T> =>
  (value, path) => {
    const found = isRecord(value) ? value.api_version : undefined;
    if (found !== API_VERSION) throw new ApiVersionError(found);
    return decode(value, path);
  };

type Acceptance = 'success_only' | 'success_or_degraded';

/** Typed reads of the management API through a capability-supplied `fetch`. */
export class ManagementApi {
  readonly #fetch: typeof fetch;
  constructor(fetcher: typeof fetch = fetch) {
    this.#fetch = fetcher;
  }

  /** A degraded recorder answers 503 with valid health. */
  health(signal?: AbortSignal): Promise<Health> {
    return this.#read('/health', health, signal, 'success_or_degraded');
  }
  metrics(signal?: AbortSignal): Promise<Metrics> {
    return this.#read('/api/metrics', metrics, signal);
  }
  storage(signal?: AbortSignal): Promise<Storage> {
    return this.#read('/api/storage', storage, signal);
  }
  /** Newest first. `query` holds the listing filters and `after`. */
  calls(query: URLSearchParams, signal?: AbortSignal): Promise<CallPage> {
    return this.#read(`/api/calls?${ManagementApi.#page(query)}`, callPage, signal);
  }
  call(id: CallId, signal?: AbortSignal): Promise<CallDetail> {
    return this.#read(`/api/calls/${id}`, callDetail, signal);
  }
  body(id: CallId, direction: Direction, signal?: AbortSignal): Promise<BodyPreview> {
    return this.#read(`/api/calls/${id}/body/${direction}`, bodyPreview, signal);
  }
  /** Totals, hours, and models for calls started within the range. */
  usage(range: { from_ms?: number; until_ms?: number }, signal?: AbortSignal): Promise<UsageView> {
    const query = new URLSearchParams();
    if (range.from_ms !== undefined) query.set('from_ms', String(Math.floor(range.from_ms)));
    if (range.until_ms !== undefined) query.set('until_ms', String(Math.floor(range.until_ms)));
    const suffix = query.size ? `?${query}` : '';
    return this.#read(`/api/usage${suffix}`, usageView, signal);
  }
  sessions(limit = 100, signal?: AbortSignal): Promise<Sessions> {
    return this.#read(`/api/sessions?limit=${limit}`, sessions, signal);
  }
  /** Oldest first. Only `after` is accepted besides the limit. */
  sessionCalls(id: SessionId, after: string | null, signal?: AbortSignal): Promise<CallPage> {
    const query = new URLSearchParams();
    if (after) query.set('after', after);
    return this.#read(`/api/sessions/${id}/calls?${ManagementApi.#page(query)}`, callPage, signal);
  }

  /** A session's turns, newest first, with the start of each prompt. */
  turns(id: SessionId, limit = 100, signal?: AbortSignal): Promise<Turns> {
    return this.#read(`/api/sessions/${id}/turns?limit=${limit}`, turns, signal);
  }
  /** Where a session's context and cache tokens went, from its call records. */
  report(id: SessionId, signal?: AbortSignal): Promise<SessionReport> {
    return this.#read(`/api/sessions/${id}/report`, sessionReport, signal);
  }
  /** What one turn did, read from its calls' bodies. */
  turn(id: TurnId, signal?: AbortSignal): Promise<TurnActivity> {
    return this.#read(`/api/turns/${id}`, turnActivity, signal);
  }
  /** Full-text search: `q`, and optionally `field`, `session`, `client`, `limit`. */
  search(query: URLSearchParams, signal?: AbortSignal): Promise<SearchPage> {
    return this.#read(`/api/search?${query}`, searchPage, signal);
  }
  /** Credentials calls carried, newest first, from `from_ms` on. */
  exposures(from_ms?: number, signal?: AbortSignal): Promise<Exposures> {
    const query = new URLSearchParams({ limit: '100' });
    if (from_ms !== undefined) query.set('from_ms', String(Math.floor(from_ms)));
    return this.#read(`/api/exposures?${query}`, exposures, signal);
  }

  /** `limit` first, the caller's or 100, then the rest of the query. */
  static #page(query: URLSearchParams): string {
    const params = new URLSearchParams([
      ['limit', query.get('limit') ?? '100'],
      ...[...query].filter(([key]) => key !== 'limit'),
    ]);
    return params.toString();
  }

  async #read<T>(
    path: string,
    decode: Decoder<T>,
    signal: AbortSignal | undefined,
    acceptance: Acceptance = 'success_only',
  ): Promise<T> {
    const timeout = AbortSignal.timeout(10_000);
    const response = await this.#fetch(path, {
      signal: signal ? AbortSignal.any([signal, timeout]) : timeout,
      cache: 'no-store',
      headers: { Accept: 'application/json' },
    });
    const accepted =
      response.ok || (acceptance === 'success_or_degraded' && response.status === 503);
    if (!accepted) throw new ApiError(response.status, await ManagementApi.#errorCode(response));
    let body: unknown;
    try {
      body = await response.json();
    } catch {
      throw new ShapeError('', 'JSON');
    }
    return versioned(decode)(body, '');
  }

  static async #errorCode(response: Response): Promise<Stored<ErrorCode> | null> {
    try {
      return versioned(errorBody)(await response.json(), '').error;
    } catch {
      return null;
    }
  }
}

export const num = (value: number | null | undefined) =>
  value == null ? '—' : value.toLocaleString();
export const short = (id: string | null | undefined) => (id ? id.slice(0, 8) : 'Unassigned');
export const title = (value: string | null | undefined) =>
  value ? value.replaceAll('_', ' ') : 'Unknown';
export function bytes(value: number) {
  return value < 1024
    ? `${value} B`
    : value < 1048576
      ? `${(value / 1024).toFixed(1)} KiB`
      : value < 1073741824
        ? `${(value / 1048576).toFixed(1)} MiB`
        : `${(value / 1073741824).toFixed(2)} GiB`;
}
export const clock = (time: number) =>
  new Date(time).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', second: '2-digit' });

const providerNames: Record<Provider, string> = {
  anthropic: 'Anthropic',
  codex: 'Codex',
  openai: 'OpenAI',
  deepseek: 'DeepSeek',
};
export const providerName = (provider: string) =>
  (providerNames as Record<string, string>)[provider] ?? provider;
const providerMarks: Record<Provider, string> = {
  anthropic: 'A',
  codex: 'C',
  openai: 'O',
  deepseek: 'D',
};
/** One-letter mark used beside provider names; `?` for one this UI does not know. */
export const providerMark = (provider: string) =>
  (providerMarks as Record<string, string>)[provider] ?? '?';
