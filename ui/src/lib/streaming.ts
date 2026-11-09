/**
 * Calls' replies while they stream, from the live feed's `progress` events:
 * the redacted end of the reply so far, the tools begun, and whether the
 * model is thinking. They are never
 * stored, so they only fill the gap until the finished call is read.
 */
import { CallId } from './api.ts';
import { array, boolean, object, string } from './decode.ts';

export interface CallProgress {
  call: CallId;
  /** The end of the reply so far. */
  reply: string;
  /** Whether the reply so far is longer than `reply`. */
  cut: boolean;
  tools: string[];
  /** Whether the model is thinking (its thinking text is not sent). */
  thinking: boolean;
}

/** `thinking`, which a daemon before it was added leaves out. */
const thinking = (value: unknown, path: string): boolean =>
  value === undefined ? false : boolean(value, path);

const callProgress = object<CallProgress>({
  call: CallId.decode,
  reply: string,
  cut: boolean,
  tools: array(string),
  thinking,
});

export class Streaming {
  /** Calls whose progress is kept; the oldest goes first. */
  static readonly MAX_CALLS = 200;

  /** One `progress` event's data, or null when it is malformed. */
  static parse(data: string): CallProgress | null {
    try {
      return callProgress(JSON.parse(data), 'progress');
    } catch {
      return null;
    }
  }

  /** Keep `update` as its call's newest, dropping the oldest calls past the bound. */
  static keep(calls: Map<CallId, CallProgress>, update: CallProgress): void {
    calls.delete(update.call);
    calls.set(update.call, update);
    for (const call of calls.keys()) {
      if (calls.size <= Streaming.MAX_CALLS) break;
      calls.delete(call);
    }
  }
}
