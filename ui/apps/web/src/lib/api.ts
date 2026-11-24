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
  ShapeError,
  type Brand,
  type Decoder,
  type Stored,
  type AnyDecoder,
  tagValue,
  tagged,
  type Variant,
} from './decode.ts';
import { passages, type Passage } from './passages.ts';
import type { Build as WireBuild } from './wire/Build.ts';
import type { CanaryView as WireCanaryView } from './wire/CanaryView.ts';
import type { CheckView as WireCheckView } from './wire/CheckView.ts';
import type { ReleaseBody as WireReleaseBody } from './wire/ReleaseBody.ts';
import type { PullsBody as WirePullsBody } from './wire/PullsBody.ts';
import type { SightingBody as WireSightingBody } from './wire/SightingBody.ts';
import type { PullBody as WirePullBody } from './wire/PullBody.ts';
import type { CheckBody as WirePullCheck } from './wire/CheckBody.ts';
import type { BranchBody as WireBranchBody } from './wire/BranchBody.ts';
import type { PreviewsBody as WirePreviewsBody } from './wire/PreviewsBody.ts';
import type { PreviewBody as WirePreviewBody } from './wire/PreviewBody.ts';
import type { PreviewOfBody as WirePreviewOf } from './wire/PreviewOfBody.ts';
import type { PreviewStateBody as WirePreviewState } from './wire/PreviewStateBody.ts';
import type { ProjectBody as WireProjectBody } from './wire/ProjectBody.ts';
import type { ProjectsBody as WireProjectsBody } from './wire/ProjectsBody.ts';
import type { WorkCountsBody as WireWorkCounts } from './wire/WorkCountsBody.ts';
import type { ReleaseView as WireReleaseView } from './wire/ReleaseView.ts';
import type { EventCountsRecord as WireEventCountsRecord } from './wire/EventCountsRecord.ts';
import type { Excerpt as WireExcerpt } from './wire/Excerpt.ts';
import type { InspectionRecord as WireInspectionRecord } from './wire/InspectionRecord.ts';
import type { Observation as WireObservation } from './wire/Observation.ts';
import type { ToolCallRecord as WireToolCallRecord } from './wire/ToolCallRecord.ts';
import type { UnknownKindRecord as WireUnknownKindRecord } from './wire/UnknownKindRecord.ts';
import type { UsageRecord as WireUsageRecord } from './wire/UsageRecord.ts';
import type { AttemptView as WireAttemptView } from './wire/AttemptView.ts';
import type { LimitHeaderView as WireLimitHeaderView } from './wire/LimitHeaderView.ts';
import type { AttributionView as WireAttributionView } from './wire/AttributionView.ts';
import type { BodyView as WireBodyView } from './wire/BodyView.ts';
import type { CallActivity as WireCallActivity } from './wire/CallActivity.ts';
import type { CallDetail as WireCallDetail } from './wire/CallDetail.ts';
import type { CallMetrics as WireCallMetrics } from './wire/CallMetrics.ts';
import type { CallPage as WireCallPage } from './wire/CallPage.ts';
import type { CallView as WireCallView } from './wire/CallView.ts';
import type { CallerView as WireCallerView } from './wire/CallerView.ts';
import type { ChannelView as WireChannelView } from './wire/ChannelView.ts';
import type { EffectView as WireEffectView } from './wire/EffectView.ts';
import type { ExposureView as WireExposureView } from './wire/ExposureView.ts';
import type { Finding as WireFinding } from './wire/Finding.ts';
import type { MessageView as WireMessageView } from './wire/MessageView.ts';
import type { ModelUsage as WireModelUsage } from './wire/ModelUsage.ts';
import type { RecordingReport as WireRecordingReport } from './wire/RecordingReport.ts';
import type { RecordingView as WireRecordingView } from './wire/RecordingView.ts';
import type { ReportTotals as WireReportTotals } from './wire/ReportTotals.ts';
import type { ResultView as WireResultView } from './wire/ResultView.ts';
import type { SearchHit as WireSearchHit } from './wire/SearchHit.ts';
import type { SearchPage as WireSearchPage } from './wire/SearchPage.ts';
import type { SessionHintView as WireSessionHintView } from './wire/SessionHintView.ts';
import type { TokenTotals as WireTokenTotals } from './wire/TokenTotals.ts';
import type { ToolCallView as WireToolCallView } from './wire/ToolCallView.ts';
import type { TurnActivity as WireTurnActivity } from './wire/TurnActivity.ts';
import type { TurnMetrics as WireTurnMetrics } from './wire/TurnMetrics.ts';
import type { TurnSummary as WireTurnSummary } from './wire/TurnSummary.ts';
import type { UsageBucket as WireUsageBucket } from './wire/UsageBucket.ts';
import type { UsageCounts as WireUsageCounts } from './wire/UsageCounts.ts';
import type { UsageView as WireUsageView } from './wire/UsageView.ts';
import type { UserUsage as WireUserUsage } from './wire/UserUsage.ts';
import type { CaptureLimits } from './wire/CaptureLimits.ts';
import type { DedupMetrics } from './wire/DedupMetrics.ts';
import type { DiskUsage } from './wire/DiskUsage.ts';
import type { FileUsage } from './wire/FileUsage.ts';
import type { Health as WireHealth } from './wire/Health.ts';
import type { InstanceOptions } from './wire/InstanceOptions.ts';
import type { OmissionMetrics } from './wire/OmissionMetrics.ts';
import type { OperatorSummary as WireOperatorSummary } from './wire/OperatorSummary.ts';
import type { Pause as WirePause } from './wire/Pause.ts';
import type { QueueMetrics } from './wire/QueueMetrics.ts';
import type { RecorderMetrics } from './wire/RecorderMetrics.ts';
import type { RetentionPolicy } from './wire/RetentionPolicy.ts';
import type { Storage } from './wire/Storage.ts';
import type { StorageView } from './wire/StorageView.ts';
import type { BodyPreview as WireBodyPreview } from './wire/BodyPreview.ts';
import type { DecodingPreview as WireDecodingPreview } from './wire/DecodingPreview.ts';
import type { ErrorBody as WireErrorBody } from './wire/ErrorBody.ts';
import type { Metrics as WireMetrics } from './wire/Metrics.ts';
import type { RawPreview as WireRawPreview } from './wire/RawPreview.ts';
import { query, type Query } from './query.ts';
import type { SessionId } from './runs.ts';
import type { RawCallQuery } from './wire/RawCallQuery.ts';
import type { RawExposureQuery } from './wire/RawExposureQuery.ts';
import type { RawPageQuery } from './wire/RawPageQuery.ts';
import type { RawSearchQuery } from './wire/RawSearchQuery.ts';
import type { RawShelfQuery } from './wire/RawShelfQuery.ts';
import type { RawUsageQuery } from './wire/RawUsageQuery.ts';

// Generated from the gateway's wire types (`wire/`, written by `cargo test`).
// The decoders below are typed against them, so a changed field fails `check`.
// Where the UI decodes a value further (labels, revisions), its own type keeps
// the wire type's fields: `object<View, Wire>`.
export type { RawCallQuery, RawSearchQuery };
export type {
  CaptureLimits,
  DiskUsage,
  FileUsage,
  RecorderMetrics,
  RetentionPolicy,
  Storage,
  StorageView,
};

/** The management API version this UI understands. */
export const API_VERSION = 1;

// Identities. Ren's own ids are UUIDs; native client ids keep separate types.
export type CallId = Brand<string, 'CallId'>;
export type AttemptId = Brand<string, 'AttemptId'>;
export type ChannelId = Brand<string, 'ChannelId'>;
export type RecordingId = Brand<string, 'RecordingId'>;
export type TurnId = Brand<string, 'TurnId'>;
export type NativeSessionId = Brand<string, 'NativeSessionId'>;
/** A registered project's id: not its name, which pages and filters use. */
export type ProjectId = Brand<string, 'ProjectId'>;
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
export const RecordingId = new Uuid<'RecordingId'>();
export const TurnId = new Uuid<'TurnId'>();

// Vocabularies (the daemon's label enums). Unrecognised text is kept.
export const providers = ['anthropic', 'codex', 'openai', 'deepseek'] as const;
export type Provider = (typeof providers)[number];
export const endpoints = [
  'messages',
  'count_tokens',
  'models',
  'responses',
  'responses_compact',
  'chat_completions',
] as const;
export type Endpoint = (typeof endpoints)[number];
// `MESSAGE`: a `response.create` on a WebSocket channel.
export const methods = ['GET', 'POST', 'MESSAGE'] as const;
export type Method = (typeof methods)[number];
export const transports = ['http', 'websocket'] as const;
export type Transport = (typeof transports)[number];
export const callStates = ['open', 'finished', 'recovered'] as const;
export type CallState = (typeof callStates)[number];
export const attemptTerminals = [
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
export const terminalCauses = [
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
export const attemptPhases = ['upload', 'awaiting_head', 'response'] as const;
/** Why Ren answered a call without forwarding it. */
export const callPurposes = ['work', 'quota_probe'] as const;
/** Why a client sent a call: its session's work, or a check of its own. */
export type CallPurpose = (typeof callPurposes)[number];
export const callRejections = [
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
  'busy',
] as const;
export type CallRejection = (typeof callRejections)[number];
/** How a call's caller was identified (see the daemon's named users). */
export const callerKinds = ['user_tailscale', 'user_token', 'unlisted', 'unidentified'] as const;
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
export const bodyEndings = ['open', 'ended', 'unfinished'] as const;
export type BodyEnding = (typeof bodyEndings)[number];
export const modelOutcomes = [
  'completed',
  'incomplete',
  'refused',
  'cancelled',
  'failed',
  'unknown',
] as const;
export type ModelOutcome = (typeof modelOutcomes)[number];
export const hintSources = [
  'claude_code_headers',
  'codex_headers',
  'pi_extension_headers',
  'codex_message_metadata',
  'opencode_headers',
] as const;
export type HintSource = (typeof hintSources)[number];
export const hintIssues = [
  'invalid_or_duplicate_header',
  'invalid_turn_metadata',
  'conflicting_identifiers',
  'turn_without_session',
] as const;
export type HintIssue = (typeof hintIssues)[number];
export const inputAccountings = ['includes_cache', 'excludes_cache'] as const;
export type InputAccounting = (typeof inputAccountings)[number];
export const coverages = ['complete', 'partial', 'unavailable'] as const;
export type Coverage = (typeof coverages)[number];
export const limitations = [
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
export const formatSources = ['declared', 'inferred'] as const;
export type FormatSource = (typeof formatSources)[number];
export const terminalEvents = ['seen', 'missing'] as const;
export type TerminalEvent = (typeof terminalEvents)[number];
export const kindFamilies = [
  'event',
  'document',
  'content_block',
  'output_item',
  'stop_reason',
  'status',
  'incomplete_reason',
] as const;
export type KindFamily = (typeof kindFamilies)[number];
export const responseFormats = [
  'event_stream',
  'json',
  'other',
  'unspecified',
  'websocket_messages',
] as const;
export type ResponseFormat = (typeof responseFormats)[number];
export const responseEncodings = ['identity', 'gzip', 'zstd', 'brotli', 'unsupported'] as const;
export type ResponseEncoding = (typeof responseEncodings)[number];
export const observationKinds = [
  'call_opened',
  'attempt_started',
  'response_head',
  'attempt_finished',
  'call_finished',
  'call_recovered',
  'channel_closed',
  'operator_action',
  'run_started',
  'run_opened',
  'run_ended',
  'run_recovered',
  'history_edited',
  'run_configured',
  'session_created',
  'session_configured',
  'session_client_chosen',
  'session_folder_chosen',
  'session_archived',
  'session_unarchived',
  'session_attached',
  'session_detached',
  'session_attachments_sent',
  'run_detached',
  'run_rejoined',
  'work_queued',
  'work_sent',
  'work_landed',
  'work_shipped',
  'work_dismissed',
  'work_previewed',
  'project_registered',
  'project_archived',
  'project_unarchived',
  'session_assigned',
  'run_passed',
] as const;
export const channelEnds = [
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
export const messagePayloads = ['text', 'binary'] as const;
export type MessagePayload = (typeof messagePayloads)[number];
export type ObservationKind = (typeof observationKinds)[number];
export const healthStatuses = ['ok', 'degraded'] as const;
/** What the gateway says it is: the installed service, or a development one. */
export const environments = ['production', 'development', 'canary', 'preview'] as const;
export type Environment = (typeof environments)[number];
/** What the canary checks of a build. */
export const checkKinds = ['doctor', 'reads', 'pages', 'build', 'serve', 'declared'] as const;
export type CheckKind = (typeof checkKinds)[number];
/** What promotion decided for the canary's build. */
export const decisions = [
  'promote',
  'current',
  'failed',
  'unchecked',
  'production_ahead',
  'unconfirmed',
  'awaiting_operator',
  'outside_window',
  'promotion_failed',
] as const;
export type Decision = (typeof decisions)[number];
export type HealthStatus = (typeof healthStatuses)[number];
export const errorCodes = [
  'not_found',
  'invalid_query',
  'busy',
  'unavailable',
  'identity_refused',
] as const;
export type ErrorCode = (typeof errorCodes)[number];
/** Why the daemon's control API refused a request: its `error` code. The UI
 * server adds codes of its own when it refuses before the daemon is asked. */
export const controlRefusals = [
  'cross_site',
  'unauthorized',
  'forbidden',
  'invalid_request',
  'not_found',
  'unavailable',
  'not_running',
  'unrecorded',
  'unrecordable',
  'backup_running',
  'backup_incomplete',
  'backup_failed',
  'call_open',
  'run_running',
  'unreadable',
  'runs_disabled',
  'invalid_directory',
  'directory_not_found',
  'not_drivable',
  'no_approvals',
  'no_scratch',
  'no_worktree',
  'worktrees_full',
  'spawn_failed',
  'models_unavailable',
  'commands_unavailable',
  'starting',
  'busy',
  'not_working',
  'turn_not_started',
  'no_such_approval',
  'unanswered',
  'not_questions',
  'unfit_reply',
  'stopping',
  'ended',
  'nothing_queued',
  'not_offered',
  'no_images',
  'protocol',
  'client_fixed',
  'folder_fixed',
  'archived',
  'not_archived',
  'no_process',
  'no_attachments',
  'attachment_sent',
  'attachment_too_large',
  'attachment_storage',
  'blank_prompt',
  'too_many_attachments',
  'attachment_name',
  'attachment_empty',
  'attachment_unrecognized',
  'attachment_unreadable',
  'attachment_too_wide',
  'sign_in_not_offered',
  'not_installed',
  'no_sign_in',
  'not_awaiting_answer',
  'redirect_not_an_address',
  'redirect_elsewhere',
  'redirect_incomplete',
  'packages_disabled',
  'packages_busy',
  'packages_no_generation',
  'packages_collected',
  'packages_unsupported',
  'packages_refused',
  'packages_failed',
  'invalid_order',
  'work_state',
  'not_promotable',
  'invalid_project',
  'project_taken',
  'project_mismatch',
] as const;
export type ControlRefusal = (typeof controlRefusals)[number];
export const previewExtents = ['complete', 'partial', 'truncated', 'gap'] as const;
export type PreviewExtent = (typeof previewExtents)[number];
export const rawFormats = ['utf8', 'hex'] as const;
export type RawFormat = (typeof rawFormats)[number];
export const textEnds = ['intact', 'split_character'] as const;
export type TextEnd = (typeof textEnds)[number];
export const previewDecodings = ['none', 'gzip_magic', 'zstd_magic', 'brotli'] as const;
export type PreviewDecoding = (typeof previewDecodings)[number];
export const decodeIntegrities = ['not_decoded', 'verified', 'prefix', 'failed'] as const;
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
  recording: RecordingId | null;
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
  /** Why the client sent it, once derived; null before then */
  purpose: Stored<CallPurpose> | null;
  caller: CallerView;
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
  /** The provider's limit headers on its response head, as it sent them. */
  limit_headers: LimitHeader[];
}
/** A limit header: its name, and its value unless Ren withheld it (not
 * plain, or read as a credential). Allowance, not token usage. */
export interface LimitHeader {
  name: string;
  value: string | null;
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
export interface RecordingView {
  id: RecordingId;
  provider: Stored<Provider>;
  source: Stored<HintSource>;
  native_id: NativeSessionId;
  first_seen: UnixMillis;
  last_seen: UnixMillis;
  call_count: number;
  turn_count: number;
  error_count: number;
  /** Input includes cached context; null when no call reported usage. */
  tokens: TokenTotals;
  /** When the operator archived it, while it has no newer traffic. */
  archived_at: UnixMillis | null;
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
  users: UserUsage[];
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
export type Pause = WirePause | { state: 'unknown'; text: string };
export interface OperatorSummary {
  pause: Pause;
  /** Session blocks in force now. */
  blocked_sessions: number;
  /** User blocks in force now. */
  blocked_users: number;
}
export interface Health {
  status: Stored<HealthStatus>;
  build: Build;
  /** Production for daemons that predate the field. */
  instance: { environment: Stored<Environment> };
  operator: OperatorSummary;
  recorder: RecorderMetrics;
}
/** One check of the canary's build. */
export interface CheckView {
  check: Stored<CheckKind>;
  /** A project's declared check's name. */
  name: string | null;
  passed: boolean;
  detail: string | null;
}
/** The build on the canary and its checks. */
export interface CanaryView {
  commit: string;
  passed: boolean;
  checks: CheckView[];
}
/** `/api/projects/{name}/release`: a project's release, and for Ren itself
 * the pages its canary captured (each at `…/release/shots/{shot}`) and the
 * build the operator asked production to take, until the factory's next
 * pass takes it up. */
export interface ReleaseBody {
  release: ReleaseView | null;
  shots: string[];
  promotion_requested: string | null;
}
export const factories = ['ren', 'contract'] as const;
/** How a project is built: as Ren itself, or by its `.ren/project.toml`. */
export type Factory = (typeof factories)[number];
/** A project's work items, counted by state. */
export interface WorkCounts {
  queued: number;
  sent: number;
  landed: number;
  shipped: number;
  dismissed: number;
  /** In a state this UI does not know. */
  other: number;
}
/** GitHub's words for an open pull request, as the daemon passes them on. */
export const readinesses = ['draft', 'ready'] as const;
export const mergeStates = [
  'clean',
  'behind',
  'blocked',
  'conflicting',
  'draft',
  'unstable',
  'has_hooks',
  'computing',
] as const;
export const reviewDecisions = [
  'approved',
  'changes_requested',
  'review_required',
  'not_required',
] as const;
/** All of a pull request's checks, taken together. */
export const checksStates = [
  'passing',
  'failing',
  'errored',
  'pending',
  'expected',
  'none',
] as const;
/** One check on a pull request. */
export const pullCheckStates = [
  'queued',
  'running',
  'expected',
  'passed',
  'failed',
  'errored',
  'cancelled',
  'timed_out',
  'skipped',
  'neutral',
] as const;
export type ChecksState = (typeof checksStates)[number];
export type PullCheckState = (typeof pullCheckStates)[number];
export interface PullCheck {
  name: string;
  state: Stored<PullCheckState>;
}
/** One open pull request, as the daemon's poller last saw it. */
export interface Pull {
  number: number;
  title: string;
  url: string;
  author: string | null;
  readiness: Stored<(typeof readinesses)[number]>;
  head_branch: string;
  head: string;
  base_branch: string;
  merge: Stored<(typeof mergeStates)[number]>;
  review: Stored<(typeof reviewDecisions)[number]>;
  checks: Stored<ChecksState>;
  check_list: PullCheck[];
  /** The work items its description's `Work-Item:` trailers name. */
  work_items: string[];
}
/** One answer from GitHub. */
export interface Sighting {
  at: number;
  default_branch: { name: string; head: string } | null;
  pull_requests: Pull[];
  /** More open pull requests (or checks) exist than were listed. */
  truncated: boolean;
}
/** `/api/projects/{name}/pulls`: whether its repository is watched, and what was seen. */
export type Pulls =
  | { watch: 'off' }
  | { watch: 'waiting'; repository: string }
  | { watch: 'seen'; repository: string; seen: Sighting }
  | { watch: 'failed'; repository: string; error: string; at: number; last: Sighting | null }
  /** A watch this UI does not know. */
  | { watch: 'unknown'; text: string };
export const reaches = ['loopback', 'tailnet'] as const;
/** Where a preview stands. */
export type PreviewState =
  | { state: 'building' }
  | { state: 'running'; reach: Stored<(typeof reaches)[number]> }
  | { state: 'failed'; detail: string }
  | { state: 'unknown'; text: string };
/** What a preview is of: an open pull request at its head, or a commit asked for. */
export type PreviewOf = { kind: 'pull'; number: number } | { kind: 'commit' } | { kind: 'unknown' };
/** One preview in its slot. */
export interface Preview {
  slot: number;
  of: PreviewOf;
  commit: string;
  state: PreviewState;
  ui_port: number;
  /** The tailnet port its UI is published on. */
  published_ui_port: number;
}
/** `/api/projects/{name}/previews`: the factory's previews, and the commits asked for. */
export interface Previews {
  previews: Preview[];
  /** Newest ask first. */
  requested: string[];
}
/** One registered project: what it is, what is under way, and its release. */
export interface Project {
  id: ProjectId;
  name: string;
  repository: string;
  factory: Stored<Factory>;
  created_at: number;
  archived_at: number | null;
  work: WorkCounts;
  /** Its sessions that are not archived. */
  sessions: number;
  /** Its runs that have not ended. */
  running: number;
  /** Null until its canary has run. */
  release: ReleaseView | null;
}
/** Where a release stands, as its canary last wrote it. */
export interface ReleaseView {
  main: string;
  production: Revision;
  /** Commits main has that production does not; null when production is not a commit of main. */
  behind: number | null;
  canary: CanaryView | null;
  /** `manual`, `automatic`, or a daily window in UTC (`02:00-05:00`). */
  policy: string;
  decision: Stored<Decision>;
}
export interface Metrics {
  recorder: RecorderMetrics;
  storage: StorageView;
}
/** Which part of a listing: what is current, or what was archived. */
export type Shelf = 'current' | 'archived';
export interface Recordings {
  recordings: RecordingView[];
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
const bodyView = object<BodyView, WireBodyView>({
  observed_bytes: nullable(count),
  stored_bytes: count,
  ending: label(bodyEndings),
});
const usage = object<Usage, WireUsageRecord>({
  accounting: label(inputAccountings),
  input_tokens: nullable(count),
  output_tokens: nullable(count),
  cache_read_tokens: nullable(count),
  cache_creation_tokens: nullable(count),
  reasoning_tokens: nullable(count),
});
const callView = object<CallView, WireCallView>({
  id: CallId.decode,
  channel: ChannelId.decode,
  started_at: millis,
  ended_at: nullable(millis),
  provider: label(providers),
  endpoint: label(endpoints),
  method: label(methods),
  config_revision: string,
  recording: nullable(RecordingId.decode),
  turn: nullable(TurnId.decode),
  attribution: object<Attribution, WireAttributionView>({
    hint: nullable(
      object<SessionHint, WireSessionHintView>({
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
  purpose: nullable(label(callPurposes)),
  caller: object<CallerView, WireCallerView>({
    kind: label(callerKinds),
    user: nullable(string),
    login: nullable(string),
  }),
});
const inspection = object<InspectionRecord, WireInspectionRecord>({
  coverage: label(coverages),
  format: label(formatSources),
  limitations: array(label(limitations)),
  outcome: nullable(label(modelOutcomes)),
  terminal_event: label(terminalEvents),
  model: nullable(string),
  stop_reason: nullable(string),
  usage: nullable(usage),
  tool_calls: array(
    object<ToolCallRecord, WireToolCallRecord>({
      call_id: nullable(string),
      item_id: nullable(string),
      kind: string,
      name: nullable(string),
    }),
  ),
  unknown_kinds: array(
    object<InspectionRecord['unknown_kinds'][number], WireUnknownKindRecord>({
      family: label(kindFamilies),
      kind: string,
      count,
    }),
  ),
  events: object<InspectionRecord['events'], WireEventCountsRecord>({
    understood: count,
    unknown: count,
    malformed: count,
  }),
});
export const observationView = object<ObservationView, WireObservation>({
  seq: brandedCount<'ObservationSeq'>(),
  at: millis,
  kind: label(observationKinds),
  channel: ChannelId.decode,
  call: nullable(CallId.decode),
  attempt: nullable(AttemptId.decode),
});
const callDetail = object<CallDetail, WireCallDetail>({
  call: callView,
  transport: label(transports),
  channel: object<ChannelView, WireChannelView>({
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
    object<AttemptView, WireAttemptView>({
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
      limit_headers: array(
        object<LimitHeader, WireLimitHeaderView>({ name: string, value: nullable(string) }),
      ),
    }),
  ),
  observations: array(observationView),
  messages: array(
    object<MessageView, WireMessageView>({
      direction: label(['request', 'response'] as const),
      offset: count,
      len: count,
      payload: label(messagePayloads),
      at: millis,
    }),
  ),
});
const callPage = object<CallPage, WireCallPage>({
  calls: array(callView),
  next: nullable(branded<'PageCursor'>()),
});
function tokenTotals(value: unknown, path: string): TokenTotals {
  return object<TokenTotals, WireTokenTotals>({
    input: nullable(count),
    output: nullable(count),
    cache_read: nullable(count),
  })(value, path);
}
const recordings = object<Recordings, { recordings: WireRecordingView[] }>({
  recordings: array(
    object<RecordingView, WireRecordingView>({
      id: RecordingId.decode,
      provider: label(providers),
      source: label(hintSources),
      native_id: branded<'NativeSessionId'>(),
      first_seen: millis,
      last_seen: millis,
      call_count: count,
      turn_count: count,
      error_count: count,
      tokens: tokenTotals,
      archived_at: nullable(millis),
    }),
  ),
});
const usageCounts = {
  calls: count,
  errors: count,
  tokens: (value: unknown, path: string) => tokenTotals(value, path),
};
const usageView = object<UsageView, WireUsageView>({
  totals: object<UsageCounts, WireUsageCounts>(usageCounts),
  hours: array(object<UsageBucket, WireUsageBucket>({ start: millis, ...usageCounts })),
  models: array(
    object<ModelUsage, WireModelUsage>({
      provider: label(providers),
      model: nullable(string),
      ...usageCounts,
    }),
  ),
  users: array(object<UserUsage, WireUserUsage>({ user: nullable(string), ...usageCounts })),
});
const fileUsage = object<FileUsage>({ page_size: count, page_count: count, freelist_count: count });
const storageView = object<StorageView>({
  disk: object<DiskUsage>({
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
  queue: object<QueueMetrics>({
    items: count,
    capacity: count,
    bytes: count,
    byte_capacity: count,
  }),
  omissions: object<OmissionMetrics>({
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
  dedup: object<DedupMetrics>({
    referenced_bytes: count,
    new_bytes: count,
    compressed_bytes: count,
  }),
  recovered_calls: count,
});
const commitPattern = /^(?:[0-9a-f]{40}|[0-9a-f]{64})$/;
/** `<commit>`, `<commit>-dirty`, or `unknown`; anything else is kept as text. */
export const revision: Decoder<Revision, string> = (value, path) => {
  const text = string(value, path);
  if (text === 'unknown') return { kind: 'unknown' };
  const dirty = text.endsWith('-dirty');
  const commit = dirty ? text.slice(0, -'-dirty'.length) : text;
  return commitPattern.test(commit)
    ? { kind: 'commit', commit, tree: dirty ? 'modified' : 'clean' }
    : { kind: 'unrecognized', text };
};
const build = object<Build, WireBuild>({ version: string, revision });
const pause = tagged<Pause, WirePause, 'state'>(
  'state',
  {
    running: object<Variant<WirePause, 'state', 'running'>>({ state: tagValue('running') }),
    paused: object<Variant<WirePause, 'state', 'paused'>>({
      state: tagValue('paused'),
      retry_after: count,
    }),
  },
  (value, path) => {
    if (!isRecord(value)) throw new ShapeError(path, 'an object');
    return { state: 'unknown', text: string(value.state, `${path}.state`) };
  },
);
const operatorSummary = object<OperatorSummary, WireOperatorSummary>({
  pause,
  blocked_sessions: count,
  blocked_users: count,
});
const health = object<Health, WireHealth>({
  status: label(healthStatuses),
  build,
  instance: object<Health['instance'], InstanceOptions>({ environment: label(environments) }),
  operator: operatorSummary,
  recorder: recorderMetrics,
});
const metrics = object<Metrics, WireMetrics>({ recorder: recorderMetrics, storage: storageView });
const releaseView = object<ReleaseView, WireReleaseView>({
  main: string,
  production: revision,
  behind: nullable(count),
  canary: nullable(
    object<CanaryView, WireCanaryView>({
      commit: string,
      passed: boolean,
      checks: array(
        object<CheckView, WireCheckView>({
          check: label(checkKinds),
          name: nullable(string),
          passed: boolean,
          detail: nullable(string),
        }),
      ),
    }),
  ),
  policy: string,
  decision: label(decisions),
});
const releaseBody = object<ReleaseBody, WireReleaseBody>({
  release: nullable(releaseView),
  shots: array(string),
  promotion_requested: nullable(string),
});
const sighting = object<Sighting, WireSightingBody>({
  at: count,
  default_branch: nullable(
    object<{ name: string; head: string }, WireBranchBody>({ name: string, head: string }),
  ),
  pull_requests: array(
    object<Pull, WirePullBody>({
      number: count,
      title: string,
      url: string,
      author: nullable(string),
      readiness: label(readinesses),
      head_branch: string,
      head: string,
      base_branch: string,
      merge: label(mergeStates),
      review: label(reviewDecisions),
      checks: label(checksStates),
      check_list: array(
        object<PullCheck, WirePullCheck>({ name: string, state: label(pullCheckStates) }),
      ),
      work_items: array(string),
    }),
  ),
  truncated: boolean,
});
const pulls = tagged<Pulls, WirePullsBody, 'watch'>(
  'watch',
  {
    off: object<Variant<WirePullsBody, 'watch', 'off'>>({ watch: tagValue('off') }),
    waiting: object<Variant<WirePullsBody, 'watch', 'waiting'>>({
      watch: tagValue('waiting'),
      repository: string,
    }),
    seen: object<Extract<Pulls, { watch: 'seen' }>, Variant<WirePullsBody, 'watch', 'seen'>>({
      watch: tagValue('seen'),
      repository: string,
      seen: sighting,
    }),
    failed: object<Extract<Pulls, { watch: 'failed' }>, Variant<WirePullsBody, 'watch', 'failed'>>({
      watch: tagValue('failed'),
      repository: string,
      error: string,
      at: count,
      last: nullable(sighting),
    }),
  },
  (value, path) => {
    if (!isRecord(value)) throw new ShapeError(path, 'an object');
    return { watch: 'unknown', text: string(value.watch, `${path}.watch`) };
  },
);
const previewState = tagged<PreviewState, WirePreviewState, 'state'>(
  'state',
  {
    building: object<Variant<WirePreviewState, 'state', 'building'>>({
      state: tagValue('building'),
    }),
    running: object<
      Extract<PreviewState, { state: 'running' }>,
      Variant<WirePreviewState, 'state', 'running'>
    >({ state: tagValue('running'), reach: label(reaches) }),
    failed: object<Variant<WirePreviewState, 'state', 'failed'>>({
      state: tagValue('failed'),
      detail: string,
    }),
  },
  (value, path) => {
    if (!isRecord(value)) throw new ShapeError(path, 'an object');
    return { state: 'unknown', text: string(value.state, `${path}.state`) };
  },
);
const previewOf = tagged<PreviewOf, WirePreviewOf, 'kind'>(
  'kind',
  {
    pull: object<Variant<WirePreviewOf, 'kind', 'pull'>>({ kind: tagValue('pull'), number: count }),
    commit: object<Variant<WirePreviewOf, 'kind', 'commit'>>({ kind: tagValue('commit') }),
  },
  () => ({ kind: 'unknown' }),
);
const previews = object<Previews, WirePreviewsBody>({
  previews: array(
    object<Preview, WirePreviewBody>({
      slot: count,
      of: previewOf,
      commit: string,
      state: previewState,
      ui_port: count,
      published_ui_port: count,
    }),
  ),
  requested: array(string),
});
const project = object<Project, WireProjectBody>({
  id: branded<'ProjectId'>(),
  name: string,
  repository: string,
  factory: label(factories),
  created_at: count,
  archived_at: nullable(count),
  work: object<WorkCounts, WireWorkCounts>({
    queued: count,
    sent: count,
    landed: count,
    shipped: count,
    dismissed: count,
    other: count,
  }),
  sessions: count,
  running: count,
  release: nullable(releaseView),
});
const projectsBody = object<{ projects: Project[] }, WireProjectsBody>({
  projects: array(project),
});
const storage = object<Storage>({
  policy: object<RetentionPolicy>({
    enabled: boolean,
    max_age_seconds: count,
    max_calls: count,
    max_storage_bytes: count,
  }),
  usage: storageView,
  capture: object<CaptureLimits>({ body_limit_bytes: count }),
});
const bodyPreview = object<BodyPreview, WireBodyPreview>({
  observed_bytes: nullable(count),
  stored_bytes: count,
  ending: label(bodyEndings),
  captured_bytes: count,
  extent: label(previewExtents),
  raw: object<BodyPreview['raw'], WireRawPreview>({
    text: string,
    format: label(rawFormats),
    truncated: boolean,
  }),
  text: nullable(string),
  text_end: label(textEnds),
  decoding: object<BodyPreview['decoding'], WireDecodingPreview>({
    applied: label(previewDecodings),
    integrity: label(decodeIntegrities),
    truncated: boolean,
  }),
});
const errorBody = object<{ error: Stored<ErrorCode> }, WireErrorBody>({ error: label(errorCodes) });

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
// and where a recording's context and cache tokens went.
export const harnesses = ['claude_code', 'codex', 'pi', 'opencode'] as const;
export type Harness = (typeof harnesses)[number];
export const effectKinds = [
  'read_file',
  'write_file',
  'run_command',
  'search',
  'delegate',
  'fetch',
] as const;
export type EffectKind = (typeof effectKinds)[number];
export const resultStatuses = ['succeeded', 'failed', 'unreported'] as const;
export type ResultStatus = (typeof resultStatuses)[number];
export const activityLimitations = [
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
export const findingKinds = [
  'cache_rebuild',
  'context_growth',
  'large_tool_output',
  'repeated_failure',
  'secret_exposure',
] as const;
export const secretKinds = [
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
/** What a tool does, as Ren reads it: on the wire, or in a driven run. */
export const toolEffect = object<ToolEffect, WireEffectView>({
  kind: label(effectKinds),
  target: string,
});
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
  recording: RecordingId;
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
export interface RecordingReport {
  recording: RecordingId;
  totals: ReportTotals;
  turns: TurnMetrics[];
  findings: Finding[];
  omitted_calls: number;
}

const excerpt = object<Excerpt, WireExcerpt>({ text: string, omitted_chars: count });
const callMetrics = object<CallMetrics, WireCallMetrics>({
  context_tokens: nullable(count),
  cache_read_tokens: nullable(count),
  cache_written_tokens: nullable(count),
  output_tokens: nullable(count),
  request_bytes: nullable(count),
  duration_ms: nullable(count),
});
const callActivity = object<CallActivity, WireCallActivity>({
  call: CallId.decode,
  started_at: millis,
  ended_at: nullable(millis),
  model: nullable(string),
  status: nullable(count),
  outcome: nullable(label(modelOutcomes)),
  metrics: callMetrics,
  prompt: nullable(excerpt),
  tool_calls: array(
    object<ToolCallActivity, WireToolCallView>({
      id: nullable(string),
      name: string,
      effects: array(toolEffect),
      input: excerpt,
      result: nullable(
        object<ToolResultView, WireResultView>({ status: label(resultStatuses), output: excerpt }),
      ),
    }),
  ),
  earlier_results: count,
  reply: nullable(excerpt),
  limitations: array(label(activityLimitations)),
});
/** The parts of a call that search and secret findings name. */
export const searchFields = ['prompt', 'reply', 'tool_input', 'tool_output'] as const;
export type SearchField = (typeof searchFields)[number];
/** A finding by its `kind`; an unknown kind keeps only its label. */
const finding = tagged<Finding, WireFinding, 'kind'>(
  'kind',
  {
    cache_rebuild: object<
      Extract<Finding, { kind: 'cache_rebuild' }>,
      Extract<WireFinding, { kind: 'cache_rebuild' }>
    >({
      kind: tagValue('cache_rebuild'),
      call: CallId.decode,
      turn: nullable(TurnId.decode),
      cache_read_tokens: count,
      cache_written_tokens: count,
      earlier_peak_tokens: count,
      idle_before_ms: nullable(count),
    }),
    context_growth: object<
      Extract<Finding, { kind: 'context_growth' }>,
      Extract<WireFinding, { kind: 'context_growth' }>
    >({
      kind: tagValue('context_growth'),
      call: CallId.decode,
      turn: nullable(TurnId.decode),
      earlier_peak_tokens: count,
      context_tokens: count,
    }),
    large_tool_output: object<
      Extract<Finding, { kind: 'large_tool_output' }>,
      Extract<WireFinding, { kind: 'large_tool_output' }>
    >({
      kind: tagValue('large_tool_output'),
      call: CallId.decode,
      tool: string,
      tool_call_id: nullable(string),
      target: nullable(string),
      output_chars: count,
    }),
    repeated_failure: object<
      Extract<Finding, { kind: 'repeated_failure' }>,
      Extract<WireFinding, { kind: 'repeated_failure' }>
    >({
      kind: tagValue('repeated_failure'),
      tool: string,
      target: nullable(string),
      failures: count,
      calls: array(CallId.decode),
    }),
    secret_exposure: object<
      Extract<Finding, { kind: 'secret_exposure' }>,
      Extract<WireFinding, { kind: 'secret_exposure' }>
    >({
      kind: tagValue('secret_exposure'),
      call: CallId.decode,
      turn: nullable(TurnId.decode),
      field: label(searchFields),
      secret: label(secretKinds),
      hint: string,
      fingerprint: string,
      count: count,
    }),
  },
  (value, path) => {
    const kind = label(findingKinds)(isRecord(value) ? value.kind : undefined, `${path}.kind`);
    return { kind: 'unrecognized', text: kind };
  },
);
const turnActivity = object<TurnActivity, WireTurnActivity>({
  turn: TurnId.decode,
  recording: RecordingId.decode,
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
const turns = object<Turns, { turns: WireTurnSummary[] }>({
  turns: array(
    object<TurnSummary, WireTurnSummary>({
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
/** A `?field=` value: a known search field, or null for any other text or none. */
export function parseSearchField(text: string | null): SearchField | null {
  return searchFields.find((field) => field === text) ?? null;
}
export interface SearchHit {
  call: CallId;
  recording: RecordingId | null;
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
const searchPage = object<SearchPage, WireSearchPage>({
  hits: array(
    object<SearchHit, WireSearchHit>({
      call: CallId.decode,
      recording: nullable(RecordingId.decode),
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
  recording: RecordingId | null;
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
const exposures = object<Exposures, { exposures: WireExposureView[] }>({
  exposures: array(
    object<ExposureView, WireExposureView>({
      call: CallId.decode,
      recording: nullable(RecordingId.decode),
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
const recordingReport = object<RecordingReport, WireRecordingReport>({
  recording: RecordingId.decode,
  totals: object<ReportTotals, WireReportTotals>({
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
    object<TurnMetrics, WireTurnMetrics>({
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
  <T, R>(decode: Decoder<T, R>): Decoder<T, R> =>
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
  /** Every registered project, archived ones too, by name. */
  async projects(signal?: AbortSignal): Promise<Project[]> {
    return (await this.#read('/api/projects', projectsBody, signal)).projects;
  }
  /** One project, as `/api/projects` lists it. */
  project(name: string, signal?: AbortSignal): Promise<Project> {
    return this.#read(`/api/projects/${encodeURIComponent(name)}`, project, signal);
  }
  /** A project's release, with Ren's captured pages and any promotion asked for. */
  projectRelease(name: string, signal?: AbortSignal): Promise<ReleaseBody> {
    return this.#read(`/api/projects/${encodeURIComponent(name)}/release`, releaseBody, signal);
  }
  /** A project's open pull requests, as the daemon's poller last saw them. */
  projectPulls(name: string, signal?: AbortSignal): Promise<Pulls> {
    return this.#read(`/api/projects/${encodeURIComponent(name)}/pulls`, pulls, signal);
  }
  /** A project's previews and the commits asked for; Ren's alone has any. */
  projectPreviews(name: string, signal?: AbortSignal): Promise<Previews> {
    return this.#read(`/api/projects/${encodeURIComponent(name)}/previews`, previews, signal);
  }
  /** Ren's own release; null until its canary has run, or while no project
   * is Ren. Read as the project `install-vps` registers, `ren`, with one
   * file read; only a Ren registered under another name lists them all. */
  async release(signal?: AbortSignal): Promise<ReleaseView | null> {
    try {
      return (await this.projectRelease(ManagementApi.REN, signal)).release;
    } catch (failure) {
      if (!(failure instanceof ApiError && failure.status === 404)) throw failure;
    }
    const ren = ManagementApi.ren(await this.projects(signal));
    return ren ? (await this.projectRelease(ren.name, signal)).release : null;
  }
  /** The name `install-vps` registers Ren itself under. */
  static readonly REN = 'ren';
  /** Ren itself among `projects`: the one unarchived project registered as Ren. */
  static ren(projects: readonly Project[]): Project | null {
    const ren = projects.filter(
      (project) => project.archived_at === null && project.factory === 'ren',
    );
    return ren.length === 1 ? (ren[0] ?? null) : null;
  }
  /** Newest first. `query` holds the listing filters and `after`. */
  calls(params: Query<RawCallQuery>, signal?: AbortSignal): Promise<CallPage> {
    return this.#read(`/api/calls?${ManagementApi.#page(params)}`, callPage, signal);
  }
  call(id: CallId, signal?: AbortSignal): Promise<CallDetail> {
    return this.#read(`/api/calls/${id}`, callDetail, signal);
  }
  body(id: CallId, direction: Direction, signal?: AbortSignal): Promise<BodyPreview> {
    return this.#read(`/api/calls/${id}/body/${direction}`, bodyPreview, signal);
  }
  /** Totals, hours, models, and users for calls started within the range;
   * with `project`, only its sessions' runs' calls; with `session`, only its
   * runs' calls. */
  usage(
    range: { from_ms?: number; until_ms?: number } & (
      { project?: string | null; session?: never } | { session: SessionId; project?: never }
    ),
    signal?: AbortSignal,
  ): Promise<UsageView> {
    const params = query<RawUsageQuery>(range);
    const suffix = params.size ? `?${params}` : '';
    return this.#read(`/api/usage${suffix}`, usageView, signal);
  }
  /** The most recently active recordings; `archived` lists those archived
   * instead; with `project`, only its sessions' runs'. */
  recordings(
    limit = 100,
    signal?: AbortSignal,
    shelf: Shelf = 'current',
    project: string | null = null,
  ): Promise<Recordings> {
    const params = query<RawShelfQuery>({
      limit,
      shelf: shelf === 'archived' ? 'archived' : null,
      project,
    });
    return this.#read(`/api/recordings?${params}`, recordings, signal);
  }
  /** Oldest first. Only `after` is accepted besides the limit. */
  recordingCalls(id: RecordingId, after: string | null, signal?: AbortSignal): Promise<CallPage> {
    return this.#read(
      `/api/recordings/${id}/calls?${ManagementApi.#page(query<RawPageQuery>({ after }))}`,
      callPage,
      signal,
    );
  }

  /** A recording's turns, newest first, with the start of each prompt. */
  turns(id: RecordingId, limit = 100, signal?: AbortSignal): Promise<Turns> {
    const params = query<RawPageQuery>({ limit });
    return this.#read(`/api/recordings/${id}/turns?${params}`, turns, signal);
  }
  /** Where a recording's context and cache tokens went, from its call records. */
  report(id: RecordingId, signal?: AbortSignal): Promise<RecordingReport> {
    return this.#read(`/api/recordings/${id}/report`, recordingReport, signal);
  }
  /** Where a relayed run reached through the relay, grouped by what it
   * asked for and how that ended; none for a run that was not relayed. */
  async passages(session: string, signal?: AbortSignal): Promise<Passage[]> {
    return (await this.#read(`/api/sessions/${session}/passages`, passages, signal)).passages;
  }
  /** What one turn did, read from its calls' bodies. */
  turn(id: TurnId, signal?: AbortSignal): Promise<TurnActivity> {
    return this.#read(`/api/turns/${id}`, turnActivity, signal);
  }
  /** Full-text search: `q`, and optionally `field`, `recording`, `client`, `limit`. */
  search(params: Query<RawSearchQuery>, signal?: AbortSignal): Promise<SearchPage> {
    return this.#read(`/api/search?${params}`, searchPage, signal);
  }
  /** Credentials calls carried, newest first, from `from_ms` on; with
   * `project`, only its sessions' runs' calls'. */
  exposures(
    from_ms?: number,
    signal?: AbortSignal,
    project: string | null = null,
  ): Promise<Exposures> {
    const params = query<RawExposureQuery>({ limit: 100, from_ms, project });
    return this.#read(`/api/exposures?${params}`, exposures, signal);
  }

  /** `limit` first, the caller's or 100, then the rest of the query. */
  static #page(params: URLSearchParams): string {
    const page = new URLSearchParams([
      ['limit', params.get('limit') ?? '100'],
      ...[...params].filter(([key]) => key !== 'limit'),
    ]);
    return page.toString();
  }

  async #read<T>(
    path: string,
    decode: AnyDecoder<T>,
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
