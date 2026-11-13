import type {
  AttemptTerminal,
  BodyPreview,
  Build,
  CallState,
  CallView,
  CallerView,
  ChannelView,
  MessageView,
  ModelOutcome,
  Usage,
} from './api.ts';
import type { Stored } from './decode.ts';
import { bytes, title } from './api.ts';
import { assertNever, unrecognised } from './decode.ts';

/**
 * What a row can truthfully claim about a call. Derived from the call's
 * lifecycle state, transport terminal, HTTP status, and model outcome; an
 * unknown or recovered outcome is never rounded up to success.
 */
export type CallStatus =
  | { kind: 'in_progress' }
  /** Ren answered without forwarding: a limit, a pause, a block. */
  | { kind: 'rejected'; rejection: string }
  | { kind: 'error'; cause: 'http_status' | 'transport' | 'model' }
  | { kind: 'interrupted'; terminal: 'cancelled' | 'interrupted' }
  /** The daemon restarted before the call finished: its outcome is unknown. */
  | { kind: 'recovered' }
  | { kind: 'model_completed' }
  | { kind: 'incomplete'; outcome: 'incomplete' | 'refused' | 'cancelled' }
  /** The response body ended, but no model terminal evidence was understood. */
  | { kind: 'http_complete' }
  | { kind: 'unknown' };

/** How a call's caller reads in a row and in the inspector. */
export interface CallerShown {
  label: string;
  title: string;
  /** The named user, for filtering by it. */
  user: string | null;
}

/**
 * Who made a call. A call recorded before callers were has no caller: that is
 * unknown, not unidentified.
 */
export function callerShown(caller: CallerView | null): CallerShown {
  if (caller === null)
    return { label: 'Unknown', title: 'Recorded before Ren recorded callers', user: null };
  const user = caller.user ?? '?';
  const login = caller.login ?? '?';
  switch (caller.kind) {
    case 'user_tailscale':
      return { label: user, title: `User ${user}, by Tailscale login ${login}`, user: caller.user };
    case 'user_token':
      return {
        label: user,
        title: `User ${user}, by user token${caller.login ? ` (Tailscale login ${login})` : ''}`,
        user: caller.user,
      };
    case 'unlisted':
      return {
        label: login,
        title: `Tailscale login ${login}, which no named user lists`,
        user: null,
      };
    case 'unidentified':
      return {
        label: 'Unidentified',
        title:
          'No user token or Tailscale identity: a local process, a tagged device, or Funnel traffic',
        user: null,
      };
    default:
      return {
        label: title(unrecognised(caller.kind)),
        title: 'A caller kind this UI does not know',
        user: null,
      };
  }
}

type Lifecycle = 'open' | 'finished' | 'recovered' | 'unknown';
type TransportEnd = 'complete' | 'failed' | 'stopped' | 'none' | 'unknown';
type ModelEnd = 'completed' | 'stopped' | 'failed' | 'none' | 'unknown';

function lifecycle(state: Stored<CallState>): Lifecycle {
  switch (state) {
    case 'open':
      return 'open';
    case 'finished':
      return 'finished';
    case 'recovered':
      return 'recovered';
    default:
      unrecognised(state);
      return 'unknown';
  }
}

function transportEnd(terminal: Stored<AttemptTerminal> | null): TransportEnd {
  switch (terminal) {
    case null:
      return 'none';
    case 'body_complete':
    case 'terminal_event':
      return 'complete';
    case 'not_sent':
    case 'refused':
    case 'acceptance_unknown':
    case 'stream_failed':
      return 'failed';
    case 'cancelled':
    case 'interrupted':
      return 'stopped';
    default:
      unrecognised(terminal);
      return 'unknown';
  }
}

function modelEnd(outcome: Stored<ModelOutcome> | null): ModelEnd {
  switch (outcome) {
    case null:
      return 'none';
    case 'completed':
      return 'completed';
    case 'incomplete':
    case 'refused':
    case 'cancelled':
      return 'stopped';
    case 'failed':
      return 'failed';
    case 'unknown':
      return 'unknown';
    default:
      unrecognised(outcome);
      return 'unknown';
  }
}

export function callStatus(call: CallView): CallStatus {
  const life = lifecycle(call.state);
  if (life === 'open') return { kind: 'in_progress' };
  if (call.rejection !== null) return { kind: 'rejected', rejection: call.rejection };
  const transport = transportEnd(call.terminal);
  const model = modelEnd(call.outcome);
  // Error evidence recorded before a restart is still true.
  if ((call.status ?? 0) >= 400) return { kind: 'error', cause: 'http_status' };
  if (transport === 'failed') return { kind: 'error', cause: 'transport' };
  if (model === 'failed') return { kind: 'error', cause: 'model' };
  if (life === 'recovered') return { kind: 'recovered' };
  if (model === 'completed') return { kind: 'model_completed' };
  if (transport === 'stopped')
    return {
      kind: 'interrupted',
      terminal: call.terminal === 'cancelled' ? 'cancelled' : 'interrupted',
    };
  if (model === 'stopped')
    return {
      kind: 'incomplete',
      outcome:
        call.outcome === 'refused'
          ? 'refused'
          : call.outcome === 'cancelled'
            ? 'cancelled'
            : 'incomplete',
    };
  if (life === 'unknown' || model === 'unknown') return { kind: 'unknown' };
  if (transport === 'complete') return { kind: 'http_complete' };
  return { kind: 'unknown' };
}

export type Tone = 'live' | 'error' | 'warning' | 'success' | 'neutral';

export function statusBadge(status: CallStatus): { label: string; tone: Tone } {
  switch (status.kind) {
    case 'in_progress':
      return { label: 'In progress', tone: 'live' };
    case 'rejected':
      return { label: `Ren · ${title(status.rejection)}`, tone: 'warning' };
    case 'error':
      return { label: 'Error', tone: 'error' };
    case 'interrupted':
      return {
        label: status.terminal === 'cancelled' ? 'Cancelled' : 'Interrupted',
        tone: 'warning',
      };
    case 'recovered':
      return { label: 'Unknown · restart', tone: 'warning' };
    case 'model_completed':
      return { label: 'Model completed', tone: 'success' };
    case 'incomplete':
      return {
        label:
          status.outcome === 'refused'
            ? 'Refused'
            : status.outcome === 'cancelled'
              ? 'Model cancelled'
              : 'Incomplete',
        tone: 'warning',
      };
    case 'http_complete':
      return { label: 'HTTP complete', tone: 'neutral' };
    case 'unknown':
      return { label: 'Unknown', tone: 'neutral' };
    default:
      return assertNever(status);
  }
}

export const isActive = (call: CallView) => lifecycle(call.state) === 'open';

/** Model terminal evidence, separate from transport. */
export function modelState(call: CallView): string {
  if (call.outcome) return title(call.outcome);
  const life = lifecycle(call.state);
  switch (life) {
    case 'open':
      return 'Awaiting outcome';
    case 'recovered':
      return 'Unknown · restart';
    case 'finished':
    case 'unknown':
      return 'Unknown';
    default:
      return assertNever(life);
  }
}

/** A short colored label, with an explanation for its tooltip. */
export interface Chip {
  label: string;
  tone: Tone;
  title: string;
}

/** The HTTP status: 2xx success, 4xx warning, 5xx error, none while waiting. */
export function httpChip(status: number | null): Chip {
  if (status === null) return { label: '—', tone: 'neutral', title: 'No response head yet' };
  const tone: Tone =
    status >= 500
      ? 'error'
      : status >= 400
        ? 'warning'
        : status >= 200 && status < 300
          ? 'success'
          : 'neutral';
  return { label: String(status), tone, title: `HTTP status ${status} from the upstream` };
}

/**
 * How the response stream ended, in plain words. This is transport evidence
 * only: a complete stream is not a completed model call.
 */
export function streamChip(call: {
  terminal: Stored<AttemptTerminal> | null;
  state?: Stored<CallState>;
}): Chip {
  switch (call.terminal) {
    case null:
      return call.state === 'recovered'
        ? { label: 'Unknown', tone: 'warning', title: 'Ren restarted before the stream ended' }
        : { label: 'Streaming', tone: 'live', title: 'The response is still arriving' };
    case 'body_complete':
      return {
        label: 'Stream complete',
        tone: 'success',
        title: 'The response body reached its end',
      };
    case 'terminal_event':
      return {
        label: 'Stream complete',
        tone: 'success',
        title: "The call's terminal event arrived on the WebSocket",
      };
    case 'interrupted':
      return {
        label: 'Cut off',
        tone: 'warning',
        title: 'The response was let go before its end, usually because the client closed it',
      };
    case 'cancelled':
      return {
        label: 'Cancelled',
        tone: 'warning',
        title: 'Stopped by the client, an operator, or shutdown',
      };
    case 'stream_failed':
      return {
        label: 'Stream failed',
        tone: 'error',
        title: 'The response stream broke or went idle',
      };
    case 'not_sent':
      return {
        label: 'Not sent',
        tone: 'error',
        title: 'The upstream connection could not be made',
      };
    case 'refused':
      return {
        label: 'Refused',
        tone: 'error',
        title: 'The upstream refused the stream before processing',
      };
    case 'acceptance_unknown':
      return {
        label: 'Maybe accepted',
        tone: 'warning',
        title: 'No response head arrived; the upstream may have accepted the request',
      };
    default:
      return {
        label: title(unrecognised(call.terminal)),
        tone: 'neutral',
        title: 'A transport ending this UI does not know',
      };
  }
}

/** HTTP status and how the stream ended, e.g. `200 · stream complete`. */
export function transportState(call: {
  status: number | null;
  terminal: Stored<AttemptTerminal> | null;
  state?: Stored<CallState>;
}): string {
  return `${call.status ?? '—'} · ${streamChip(call).label.toLowerCase()}`;
}

/**
 * Whether a call's bodies were cut only where the per-body capture limit
 * cuts them: each body ended, and every short one stored exactly the limit.
 * That is capture by design, not loss; forwarding carried every byte.
 */
export function trimmedAtLimit(call: CallView, bodyLimit: number | null): boolean {
  if (bodyLimit === null || call.capture_complete) return false;
  const bodies = [call.request, call.response].filter((body) => body !== null);
  const short = bodies.filter(
    (body) => body.observed_bytes !== null && body.observed_bytes > body.stored_bytes,
  );
  return (
    short.length > 0 &&
    bodies.every((body) => body.ending === 'ended') &&
    short.every((body) => body.stored_bytes === bodyLimit)
  );
}

/** Whether the stored bodies hold everything that was observed. */
export function captureChip(call: CallView, bodyLimit: number | null = null): Chip {
  const life = lifecycle(call.state);
  switch (life) {
    case 'open':
      return { label: 'Recording', tone: 'live', title: 'Capture is still being written' };
    case 'recovered':
      return { label: 'Capture cut off', tone: 'warning', title: 'Ren restarted while recording' };
    case 'finished':
      if (trimmedAtLimit(call, bodyLimit))
        return {
          label: 'Trimmed at limit',
          tone: 'neutral',
          title: `A body passed the ${bytes(bodyLimit ?? 0)} capture limit, so only that much was stored, by design. Forwarding carried every byte.`,
        };
      return call.capture_complete
        ? { label: 'Captured', tone: 'success', title: 'Both bodies ended and were stored whole' }
        : {
            label: 'Partial capture',
            tone: 'warning',
            title: 'Not every observed byte was stored, or a body did not end',
          };
    case 'unknown':
      return { label: 'Capture unknown', tone: 'neutral', title: 'An unrecognised call state' };
    default:
      return assertNever(life);
  }
}

/** The model's outcome as a chip, separate from the transport. */
export function modelChip(call: CallView): Chip {
  if (bookkeeping(call))
    return {
      label: 'Quota probe',
      tone: 'neutral',
      title: "The client's own check of its quota, not its session's work; its 429 is expected",
    };
  const label = call.outcome
    ? `Model ${title(call.outcome)}`
    : lifecycle(call.state) === 'finished'
      ? 'No model outcome'
      : modelState(call);
  const chip = (tone: Tone): Chip => ({
    label,
    tone,
    title: 'What the model reported, separate from how the stream ended',
  });
  if (lifecycle(call.state) === 'open') return chip('live');
  const end = modelEnd(call.outcome);
  switch (end) {
    case 'completed':
      return chip('success');
    case 'stopped':
      return chip('warning');
    case 'failed':
      return chip('error');
    case 'none':
    case 'unknown':
      return chip('neutral');
    default:
      return assertNever(end);
  }
}

/** A count in a few characters: 89, 1.2k, 101.7k, 3.4M. */
export function compact(value: number): string {
  if (value < 1000) return String(value);
  if (value < 1_000_000) return `${(value / 1000).toFixed(value < 10_000 ? 1 : 0)}k`;
  return `${(value / 1_000_000).toFixed(1)}M`;
}

/** A count as `compact` writes it, or an em dash when none was reported. */
export function compactOrDash(value: number | null): string {
  return value === null ? '—' : compact(value);
}

/** A whole percentage, e.g. `87%`, or an em dash when there is none. */
export function percentOrDash(value: number | null): string {
  return value === null ? '—' : `${value}%`;
}

/**
 * Tokens in and out. Input counts cached tokens too, whichever way the
 * provider accounts them, since cached context is most of a coding agent's
 * input. The tooltip keeps the breakdown.
 */
export function tokenSummary(
  usage: Usage | null,
): { input: string; output: string; title: string } | null {
  if (!usage || (usage.input_tokens === null && usage.output_tokens === null)) return null;
  const cached = (usage.cache_read_tokens ?? 0) + (usage.cache_creation_tokens ?? 0);
  const fresh = usage.input_tokens ?? 0;
  const input = usage.accounting === 'excludes_cache' ? fresh + cached : fresh;
  const parts = [
    `${(usage.input_tokens ?? 0).toLocaleString()} input`,
    usage.cache_read_tokens ? `${usage.cache_read_tokens.toLocaleString()} cache read` : '',
    usage.cache_creation_tokens
      ? `${usage.cache_creation_tokens.toLocaleString()} cache write`
      : '',
    `${(usage.output_tokens ?? 0).toLocaleString()} output`,
  ].filter(Boolean);
  return {
    input: compact(input),
    output: usage.output_tokens === null ? '—' : compact(usage.output_tokens),
    title: `${parts.join(' · ')} (provider reported; not subscription allowance or charges)`,
  };
}

/** How much of a body the preview holds, as chips: complete in one, gaps spelled out. */
export function previewChips(preview: BodyPreview, bodyLimit: number | null = null): Chip[] {
  const chips: Chip[] = [];
  const observed = preview.observed_bytes;
  // Stored exactly up to the capture limit of a longer body: trimmed by design, not lost.
  const trimmed =
    bodyLimit !== null &&
    preview.ending === 'ended' &&
    observed !== null &&
    observed > preview.stored_bytes &&
    preview.stored_bytes === bodyLimit;
  const trimmedChip: Chip = {
    label: `Trimmed at ${bytes(bodyLimit ?? 0)}`,
    tone: 'neutral',
    title: `Only the first ${bytes(bodyLimit ?? 0)} of ${bytes(observed ?? 0)} were stored: the capture limit, by design. Forwarding carried every byte.`,
  };
  const whole =
    preview.extent === 'complete' && observed !== null && preview.stored_bytes === observed;
  chips.push({
    label: bytes(preview.captured_bytes),
    tone: 'neutral',
    title: `${bytes(preview.captured_bytes)} shown · ${bytes(preview.stored_bytes)} stored · ${observed === null ? 'observed size unknown' : `${bytes(observed)} observed`}`,
  });
  switch (preview.extent) {
    case 'complete':
      chips.push(
        whole
          ? {
              label: 'Complete',
              tone: 'success',
              title: 'Every observed byte was stored and is shown',
            }
          : { label: 'Complete so far', tone: 'live', title: 'Everything stored so far is shown' },
      );
      break;
    case 'partial':
      if (trimmed) {
        chips.push(trimmedChip);
        break;
      }
      chips.push({
        label: `${bytes(preview.stored_bytes)} of ${observed === null ? '?' : bytes(observed)} stored`,
        tone: 'warning',
        title: 'Not every observed byte was stored; missing bytes are not reconstructed',
      });
      break;
    case 'truncated':
      chips.push({
        label: 'Preview limit',
        tone: 'warning',
        title: 'The preview stops before the end of the stored body',
      });
      break;
    case 'gap':
      if (trimmed) {
        chips.push(trimmedChip);
        break;
      }
      chips.push({
        label: 'Stops at a gap',
        tone: 'warning',
        title: 'The preview stops before bytes that were not stored',
      });
      break;
    default:
      chips.push({
        label: title(unrecognised(preview.extent)),
        tone: 'neutral',
        title: 'An extent this UI does not know',
      });
  }
  if (preview.ending === 'open')
    chips.push({ label: 'Recording', tone: 'live', title: 'The body is still arriving' });
  else if (preview.ending === 'unfinished')
    chips.push({
      label: 'Never ended',
      tone: 'warning',
      title: 'The body never finished; only captured bytes are shown',
    });
  if (preview.decoding.applied !== 'none') {
    const name =
      preview.decoding.applied === 'gzip_magic'
        ? 'gzip'
        : preview.decoding.applied === 'zstd_magic'
          ? 'zstd'
          : title(preview.decoding.applied);
    const integrity = preview.decoding.integrity;
    chips.push({
      label: integrity === 'verified' ? `${name} ✓` : name,
      tone: integrity === 'failed' ? 'error' : integrity === 'prefix' ? 'warning' : 'neutral',
      title:
        integrity === 'verified'
          ? `Decoded from ${name}; integrity verified. Raw keeps the original bytes.`
          : integrity === 'prefix'
            ? `Decoded ${name} prefix; the compressed stream is incomplete, so integrity is unverified.`
            : integrity === 'failed'
              ? `The ${name} stream is incomplete, corrupt, or over decoder limits. Raw keeps the original bytes.`
              : `Decoded from ${name}. Raw keeps the original bytes.`,
    });
  }
  return chips;
}

/** Why an attempt ended and how far it got, when the daemon recorded it. */
export function attemptEnding(attempt: {
  terminal_cause: string | null;
  terminal_phase: string | null;
}): string | null {
  const parts = [attempt.terminal_cause, attempt.terminal_phase && `in ${attempt.terminal_phase}`]
    .filter((part): part is string => Boolean(part))
    .map((part) => part.replaceAll('_', ' '));
  return parts.length ? parts.join(' · ') : null;
}

/** A WebSocket channel's state, e.g. `open` or `client closed · 1000`; null for HTTP. */
export function channelState(channel: ChannelView): string | null {
  if (channel.transport !== 'websocket') return null;
  const parts = [
    channel.ending === null ? 'open' : channel.ending.replaceAll('_', ' '),
    channel.close_code === null ? null : `code ${channel.close_code}`,
    channel.unattributed_messages
      ? `${channel.unattributed_messages} message${channel.unattributed_messages === 1 ? '' : 's'} between calls (not stored)`
      : null,
  ];
  return parts.filter((part): part is string => part !== null).join(' · ');
}

/** How many WebSocket messages each body of a call holds. */
export function messageCounts(messages: MessageView[]): { request: number; response: number } {
  let request = 0;
  let response = 0;
  for (const message of messages) {
    if (message.direction === 'request') request += 1;
    else if (message.direction === 'response') response += 1;
  }
  return { request, response };
}

export function duration(call: { started_at: number; ended_at: number | null }): number | null {
  return call.ended_at === null ? null : Math.max(0, call.ended_at - call.started_at);
}
export function elapsed(ms: number | null) {
  return ms === null ? '—' : ms < 1000 ? `${Math.round(ms)} ms` : `${(ms / 1000).toFixed(2)} s`;
}
export const date = (ms: number) =>
  new Date(ms).toLocaleDateString([], { month: 'short', day: 'numeric' });

export function captureState(call: CallView, bodyLimit: number | null = null): string {
  // Open calls are still recording; completeness is final only once they end.
  const life = lifecycle(call.state);
  switch (life) {
    case 'open':
      return 'Recording';
    case 'recovered':
      return 'Interrupted capture';
    case 'finished':
      if (trimmedAtLimit(call, bodyLimit)) return 'Trimmed at the capture limit';
      return call.capture_complete ? 'Complete capture' : 'Partial capture';
    case 'unknown':
      return 'Capture unknown';
    default:
      return assertNever(life);
  }
}

/** `0.1.0 (00a590295015)`, as `ren --version` and `ren doctor` show it. */
/** A build as text; `—` before health has loaded. */
export function buildLabel(build: Build | null): string {
  if (!build) return '—';
  const revision = build.revision;
  switch (revision.kind) {
    case 'commit':
      return `${build.version} (${revision.commit.slice(0, 12)}${revision.tree === 'modified' ? ', modified' : ''})`;
    case 'unknown':
      return `${build.version} (unknown revision)`;
    case 'unrecognized':
      return `${build.version} (${revision.text})`;
    default:
      return assertNever(revision);
  }
}

/** How long something lasted, at a glance: 45 s, 12 min, 1 h 23 min. */
export function span(ms: number): string {
  const seconds = Math.max(0, Math.round(ms / 1000));
  if (seconds < 60) return `${seconds} s`;
  const minutes = Math.round(seconds / 60);
  if (minutes < 60) return `${minutes} min`;
  const hours = Math.floor(minutes / 60);
  return minutes % 60 ? `${hours} h ${minutes % 60} min` : `${hours} h`;
}

/** Recordings get one of eight colors, the same for an ID everywhere. A class,
 * not an inline style, so the CSP stays strict. */
export const SESSION_TONES = 8;
export function sessionTone(id: string): number {
  let hash = 0;
  for (const char of id) hash = (hash * 31 + char.charCodeAt(0)) >>> 0;
  return hash % SESSION_TONES;
}

/**
 * Between inline facts in a line of text, its space kept in the text: markup
 * whitespace between inline items collapses.
 */
export const SEPARATOR = '\u00a0· ';

/**
 * Whether a client sent the call for itself rather than for its session's
 * work, such as Claude Code's quota probe: its failures are not the work's.
 */
export function bookkeeping(call: CallView): boolean {
  const purpose = call.purpose;
  switch (purpose) {
    case null:
    case 'work':
      return false;
    case 'quota_probe':
      return true;
    default:
      unrecognised(purpose);
      return false;
  }
}

/** A call's row badge: how it ended, or, for bookkeeping, what it was, in grey. */
export function callBadge(call: CallView): { label: string; tone: Tone } {
  if (bookkeeping(call))
    return {
      label: `Quota probe${call.status === null ? '' : ` · ${call.status}`}`,
      tone: 'neutral',
    };
  return statusBadge(callStatus(call));
}
