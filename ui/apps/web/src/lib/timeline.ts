// The Traffic page's overview of the calls it lists: each recording as a
// lane with a bar per call from its start to its end, and the figures above
// the list (how many, how many failed, how long they took, how many are
// still open). Read from the listed page of calls alone, so it says what
// that page holds, not the whole time range. Pure: no DOM, no clock.

import type { AttemptView, CallView, RecordingId } from './api.ts';
import { bookkeeping, callStatus, httpChip, streamChip, type Tone } from './display.ts';
import { assertNever } from './decode.ts';
import type { Mark } from './marks.ts';

/** How one call stands in a row or on a lane, as a mark and its words. */
export interface CallMark {
  mark: Mark;
  label: string;
}

/** One call on a lane, placed in the lane's 0–100 span. */
export interface LaneBar {
  call: CallView;
  /** Where it starts, and how wide it runs, as shares of the window. */
  left: number;
  width: number;
  mark: Mark;
}

/** A recording's calls in the window; `recording` null for calls with none. */
export interface Lane {
  recording: RecordingId | null;
  bars: LaneBar[];
  /** Whether a call of it is still open. */
  live: boolean;
}

/** The figures over the list. */
export interface Figures {
  calls: number;
  /** Calls a minute over the window; null for a window under a second. */
  perMinute: number | null;
  failed: number;
  /** The median and 95th percentile of finished calls' durations, in ms. */
  p50: number | null;
  p95: number | null;
  open: number;
  /** Quota probes and other calls the client made for itself. */
  probes: number;
}

/** One upstream try of a call, placed across the call's span. */
export interface AttemptBar {
  attempt: AttemptView;
  left: number;
  width: number;
  mark: Mark;
  /** Its HTTP status and how its stream ended, in words. */
  label: string;
}

/** A call's prompt split by where its tokens came from, and its output. */
export interface TokenSplit {
  /** The whole prompt: read from the cache, written to it, and new. */
  context: number;
  read: number;
  written: number;
  fresh: number;
  output: number | null;
}

/** The most lanes drawn; the quietest recordings past it are counted. */
const LANES = 6;
/** The narrowest bar, as a share of the window, so a short call shows. */
const MIN_WIDTH = 0.4;

export class CallTimeline {
  /** The window, in ms since the epoch. */
  readonly from: number;
  readonly to: number;
  readonly lanes: Lane[];
  /** Recordings with calls in the window past the lanes drawn. */
  readonly folded: number;
  readonly figures: Figures;

  private constructor(from: number, to: number, lanes: Lane[], folded: number, figures: Figures) {
    this.from = from;
    this.to = to;
    this.lanes = lanes;
    this.folded = folded;
    this.figures = figures;
  }

  /** `calls` over the window from the first's start to the last's end, or
   * to `now` while one is open. */
  static of(calls: readonly CallView[], now: number): CallTimeline {
    if (!calls.length) return new CallTimeline(now, now, [], 0, CallTimeline.#figures([], 0));
    const from = Math.min(...calls.map((call) => call.started_at));
    const to = Math.max(...calls.map((call) => call.ended_at ?? now));
    const span = Math.max(1, to - from);

    const byRecording = new Map<RecordingId | null, CallView[]>();
    for (const call of calls) {
      const own = byRecording.get(call.recording) ?? [];
      own.push(call);
      byRecording.set(call.recording, own);
    }
    const busiest = [...byRecording].toSorted(
      ([, a], [, b]) =>
        b.length - a.length ||
        Math.max(...b.map((call) => call.started_at)) -
          Math.max(...a.map((call) => call.started_at)),
    );
    const lanes = busiest.slice(0, LANES).map(([recording, own]): Lane => ({
      recording,
      live: own.some((call) => call.ended_at === null),
      bars: own
        .toSorted((a, b) => a.started_at - b.started_at)
        .map((call) => {
          const left = ((call.started_at - from) / span) * 100;
          const width = (((call.ended_at ?? now) - call.started_at) / span) * 100;
          return {
            call,
            left,
            width: Math.min(Math.max(MIN_WIDTH, width), 100 - left),
            mark: CallTimeline.mark(call).mark,
          };
        }),
    }));
    return new CallTimeline(
      from,
      to,
      lanes,
      Math.max(0, busiest.length - LANES),
      CallTimeline.#figures(calls, span),
    );
  }

  /** How `call` stands: open, ended well, failed, or past knowing. A call
   * the client made for itself reads as information, whatever it got. */
  static mark(call: CallView): CallMark {
    if (bookkeeping(call)) return { mark: 'info', label: 'Quota probe' };
    const status = callStatus(call);
    switch (status.kind) {
      case 'in_progress':
        return { mark: 'responding', label: 'In progress' };
      case 'model_completed':
        return { mark: 'passed', label: 'Model completed' };
      case 'http_complete':
        return { mark: 'passed', label: 'HTTP complete' };
      case 'error':
        return {
          mark: 'failed',
          label:
            status.cause === 'http_status'
              ? `Failed, HTTP ${call.status ?? ''}`.trim()
              : status.cause === 'transport'
                ? 'Failed in transport'
                : 'The model failed',
        };
      case 'rejected':
        return { mark: 'warning', label: `Refused by Ren: ${status.rejection}` };
      case 'interrupted':
        return {
          mark: 'warning',
          label: status.terminal === 'cancelled' ? 'Cancelled' : 'Interrupted',
        };
      case 'incomplete':
        return { mark: 'warning', label: `Incomplete: ${status.outcome}` };
      case 'recovered':
        return { mark: 'unknown', label: 'Unknown: Ren restarted during it' };
      case 'unknown':
        return { mark: 'unknown', label: 'Unknown' };
      default:
        return assertNever(status);
    }
  }

  /** The prompt `call` sent, in tokens, and the share of it read from the
   * cache; null when the provider reported no usage. */
  static context(call: CallView): { tokens: number; cached: number } | null {
    const split = CallTimeline.tokens(call);
    if (!split) return null;
    return {
      tokens: split.context,
      cached: split.context > 0 ? Math.min(1, split.read / split.context) : 0,
    };
  }

  /** `call`'s prompt split three ways, as its provider's accounting counts
   * the cache; null when it reported no input. */
  static tokens(call: CallView): TokenSplit | null {
    const usage = call.usage;
    if (!usage || usage.input_tokens === null) return null;
    const read = usage.cache_read_tokens ?? 0;
    const written = usage.cache_creation_tokens ?? 0;
    const excludes = usage.accounting === 'excludes_cache';
    const context = excludes ? usage.input_tokens + read + written : usage.input_tokens;
    return {
      context,
      read,
      written,
      fresh: Math.max(0, excludes ? usage.input_tokens : usage.input_tokens - read - written),
      output: usage.output_tokens,
    };
  }

  /** `call`'s attempts across its span, from its start to its end (or to
   * `now` while open), each marked by how it ended. */
  static attempts(call: CallView, attempts: readonly AttemptView[], now: number): AttemptBar[] {
    const end = call.ended_at ?? now;
    const span = Math.max(1, end - call.started_at);
    return attempts.map((attempt) => {
      const stream = streamChip(attempt);
      const status = attempt.status === null ? null : httpChip(attempt.status);
      const worst: Tone =
        status?.tone === 'error' || stream.tone === 'error'
          ? 'error'
          : status?.tone === 'warning' || stream.tone === 'warning'
            ? 'warning'
            : stream.tone;
      const left = ((attempt.started_at - call.started_at) / span) * 100;
      return {
        attempt,
        left: Math.max(0, left),
        width: Math.min(
          Math.max(MIN_WIDTH, (((attempt.ended_at ?? end) - attempt.started_at) / span) * 100),
          100 - Math.max(0, left),
        ),
        mark: CallTimeline.#toneMark(worst),
        label: [status ? `HTTP ${status.label}` : null, stream.label]
          .filter((part) => part !== null)
          .join(' · '),
      };
    });
  }

  static #toneMark(tone: Tone): Mark {
    switch (tone) {
      case 'success':
        return 'passed';
      case 'error':
        return 'failed';
      case 'warning':
        return 'warning';
      case 'live':
        return 'responding';
      case 'neutral':
        return 'idle';
      default:
        return assertNever(tone);
    }
  }

  static #figures(calls: readonly CallView[], span: number): Figures {
    const durations = calls
      .flatMap((call) => (call.ended_at === null ? [] : [call.ended_at - call.started_at]))
      .toSorted((a, b) => a - b);
    const at = (share: number) =>
      durations.length ? durations[Math.floor(share * (durations.length - 1))]! : null;
    return {
      calls: calls.length,
      perMinute: span >= 1000 ? calls.length / (span / 60_000) : null,
      failed: calls.filter((call) => CallTimeline.mark(call).mark === 'failed').length,
      p50: at(0.5),
      p95: at(0.95),
      open: calls.filter((call) => call.ended_at === null).length,
      probes: calls.filter(bookkeeping).length,
    };
  }
}
