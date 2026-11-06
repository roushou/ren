import type {
  AttemptTerminal,
  Build,
  CallState,
  CallView,
  ChannelView,
  MessageView,
  ModelOutcome,
} from './api.ts';
import type { Stored } from './decode.ts';
import { title } from './api.ts';
import { assertNever, unrecognised } from './decode.ts';

/**
 * What a row can truthfully claim about a call. Derived from the call's
 * lifecycle state, transport terminal, HTTP status, and model outcome; an
 * unknown or recovered outcome is never rounded up to success.
 */
export type CallStatus =
  | { kind: 'in_progress' }
  /** TGV answered without forwarding: a limit, a pause, a block. */
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
      return { label: `TGV · ${title(status.rejection)}`, tone: 'warning' };
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

/** HTTP status and transport terminal, e.g. `200 · body EOF`. */
export function transportState(call: {
  status: number | null;
  terminal: Stored<AttemptTerminal> | null;
  state?: Stored<CallState>;
}): string {
  if (call.terminal)
    return `${call.status ?? '—'} · ${call.terminal === 'body_complete' ? 'body EOF' : title(call.terminal)}`;
  if (call.state === 'recovered') return 'Unknown · restart';
  return call.status === null ? 'Transport pending' : `${call.status} · pending`;
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

export function captureState(call: CallView): string {
  // Open calls are still recording; completeness is final only once they end.
  const life = lifecycle(call.state);
  switch (life) {
    case 'open':
      return 'Recording';
    case 'recovered':
      return 'Interrupted capture';
    case 'finished':
      return call.capture_complete ? 'Complete capture' : 'Partial capture';
    case 'unknown':
      return 'Capture unknown';
    default:
      return assertNever(life);
  }
}

/** `0.1.0 (00a590295015)`, as `tgv --version` and `tgv doctor` show it. */
export function buildLabel(build: Build | null): string {
  if (!build) return 'not reported (daemon predates build reporting)';
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
