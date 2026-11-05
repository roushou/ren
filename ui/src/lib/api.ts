// Types and boundary parsing for the daemon's read-only management API
// (`crates/tgv-daemon/src/api/mod.rs`). Field names match the daemon's JSON.
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
  ShapeError,
  type Brand,
  type Decoder,
  type Stored,
} from './decode.ts';

/** The management API version this UI understands. */
export const API_VERSION = 1;

// Identities. TGV's own ids are UUIDs; native client ids keep separate types.
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
const providers = ['anthropic', 'codex', 'openai'] as const;
export type Provider = (typeof providers)[number];
const endpoints = ['messages', 'count_tokens', 'models', 'responses', 'responses_compact'] as const;
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
  // The TGV operator cancelled the call.
  'operator',
] as const;
export type TerminalCause = (typeof terminalCauses)[number];
const attemptPhases = ['upload', 'awaiting_head', 'response'] as const;
/** Why TGV answered a call without forwarding it. */
const callRejections = [
  'too_large',
  'buffer_full',
  'upload_timeout',
  'upload_failed',
  'abandoned',
  'paused',
  'session_blocked',
  'blocked',
  'shutdown',
] as const;
export type CallRejection = (typeof callRejections)[number];
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
const responseEncodings = ['identity', 'gzip', 'zstd', 'unsupported'] as const;
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
export type HealthStatus = (typeof healthStatuses)[number];
const errorCodes = ['not_found', 'invalid_query', 'busy', 'unavailable'] as const;
export type ErrorCode = (typeof errorCodes)[number];
const previewExtents = ['complete', 'partial', 'truncated', 'gap'] as const;
export type PreviewExtent = (typeof previewExtents)[number];
const rawFormats = ['utf8', 'hex'] as const;
export type RawFormat = (typeof rawFormats)[number];
const textEnds = ['intact', 'split_character'] as const;
export type TextEnd = (typeof textEnds)[number];
const previewDecodings = ['none', 'gzip_magic', 'zstd_magic'] as const;
export type PreviewDecoding = (typeof previewDecodings)[number];
const decodeIntegrities = ['not_decoded', 'verified', 'prefix', 'failed'] as const;
export type DecodeIntegrity = (typeof decodeIntegrities)[number];

export type Direction = 'request' | 'response';

// Views (`crates/tgv-store/src/view.rs`).
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
  /** Set when TGV answered without forwarding the call. */
  rejection: Stored<CallRejection> | null;
  /** The operator state revision the call was planned against. */
  operator_revision: number | null;
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
}
export interface FileUsage {
  page_size: number;
  page_count: number;
  freelist_count: number;
}
export interface StorageView {
  disk: { metadata: FileUsage; blobs: FileUsage };
  calls: number;
  open_calls: number;
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
export interface Health {
  status: Stored<HealthStatus>;
  recorder: RecorderMetrics;
}
export interface Metrics {
  recorder: RecorderMetrics;
  storage: StorageView;
}
export interface Storage {
  policy: RetentionPolicy;
  usage: StorageView;
}
export interface Sessions {
  sessions: SessionView[];
}
/** A bounded preview of a stored body (`crates/tgv-daemon/src/api/preview.rs`). */
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
    }),
  ),
});
const fileUsage = object<FileUsage>({ page_size: count, page_count: count, freelist_count: count });
const storageView = object<StorageView>({
  disk: object<StorageView['disk']>({ metadata: fileUsage, blobs: fileUsage }),
  calls: count,
  open_calls: count,
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
const health = object<Health>({ status: label(healthStatuses), recorder: recorderMetrics });
const metrics = object<Metrics>({ recorder: recorderMetrics, storage: storageView });
const storage = object<Storage>({
  policy: object<RetentionPolicy>({
    enabled: boolean,
    max_age_seconds: count,
    max_calls: count,
    max_storage_bytes: count,
  }),
  usage: storageView,
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
      `Incompatible TGV daemon: it reported management API version ${
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
        ? 'Cannot reach TGV. Check that the daemon is running and the UI gateway URL is correct.'
        : status === 503
          ? code === 'unavailable'
            ? 'TGV storage is unavailable. Check the daemon logs.'
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

/** Check the version envelope, then decode the fields beside it. */
const versioned =
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
  sessions(limit = 100, signal?: AbortSignal): Promise<Sessions> {
    return this.#read(`/api/sessions?limit=${limit}`, sessions, signal);
  }
  /** Oldest first. Only `after` is accepted besides the limit. */
  sessionCalls(id: SessionId, after: string | null, signal?: AbortSignal): Promise<CallPage> {
    const query = new URLSearchParams();
    if (after) query.set('after', after);
    return this.#read(`/api/sessions/${id}/calls?${ManagementApi.#page(query)}`, callPage, signal);
  }

  static #page(query: URLSearchParams): string {
    const params = new URLSearchParams([['limit', '100'], ...query]);
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
};
export const providerName = (provider: string) =>
  (providerNames as Record<string, string>)[provider] ?? provider;
/** One-letter mark used beside provider names. */
export const providerMark = (provider: string) =>
  provider === 'anthropic' ? 'A' : provider === 'codex' ? 'C' : 'O';
