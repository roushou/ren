// Live follow of `/api/live`: a browser-independent state machine. The Svelte
// wrapper (`live.svelte.ts`) feeds it EventSource events and runs its actions.
import { observationView, type ObservationSeq } from './api.ts';
import { assertNever, isRecord } from './decode.ts';

export type LiveState =
  /** Not following: switched off, or EventSource is unavailable. */
  | { kind: 'closed'; reason: 'paused' | 'unsupported' }
  /** Opening the feed, or waiting to reopen it after it dropped. */
  | {
      kind: 'connecting';
      last: ObservationSeq | null;
      failures: number;
      problem: 'none' | 'disconnected' | 'store_unavailable';
    }
  /** The feed is open; observations trigger refreshes. */
  | { kind: 'following'; last: ObservationSeq | null }
  /** The resume position was pruned: reloading everything before reconnecting fresh. */
  | { kind: 'reloading' };

export type LiveEvent =
  | { kind: 'enable' }
  | { kind: 'disable' }
  | { kind: 'unsupported' }
  | { kind: 'opened' }
  | { kind: 'observation'; seq: ObservationSeq | null }
  | { kind: 'lagged' }
  | { kind: 'gone' }
  /** The server's `error` event: the store is unavailable and the stream ends. */
  | { kind: 'store_error' }
  /**
   * The connection dropped. `browser` means EventSource reconnects by itself
   * (sending `Last-Event-ID`); `closed` means it gave up and we must reopen.
   */
  | { kind: 'dropped'; retry: 'browser' | 'closed' }
  | { kind: 'reloaded' };

export type LiveAction =
  /** Open a new EventSource after `delayMs`, resuming after `after` when set. */
  | { kind: 'connect'; after: ObservationSeq | null; delayMs: number }
  | { kind: 'disconnect' }
  /** Refresh page data (the wrapper debounces). */
  | { kind: 'refresh' }
  /** Reload all page data, then report `reloaded`. */
  | { kind: 'reload' };

export type LiveStep =
  | { kind: 'moved'; state: LiveState; actions: LiveAction[] }
  /** The event does not apply in this state (for example, from a closed feed). */
  | { kind: 'ignored'; state: LiveState; event: LiveEvent };

const moved = (state: LiveState, ...actions: LiveAction[]): LiveStep => ({
  kind: 'moved',
  state,
  actions,
});

export class LiveFollow {
  static readonly initial: LiveState = { kind: 'closed', reason: 'paused' };
  static readonly maxDelayMs = 30_000;

  /** Reconnect backoff: 1 s, 2 s, 4 s, … capped at 30 s. */
  static delay(failures: number): number {
    return Math.min(LiveFollow.maxDelayMs, 1000 * 2 ** Math.max(0, failures - 1));
  }

  static step(state: LiveState, event: LiveEvent): LiveStep {
    const ignored: LiveStep = { kind: 'ignored', state, event };
    switch (event.kind) {
      case 'enable':
        return state.kind === 'closed' && state.reason === 'paused'
          ? moved(
              { kind: 'connecting', last: null, failures: 0, problem: 'none' },
              { kind: 'connect', after: null, delayMs: 0 },
            )
          : ignored;
      case 'disable':
        return state.kind === 'closed'
          ? ignored
          : moved({ kind: 'closed', reason: 'paused' }, { kind: 'disconnect' });
      case 'unsupported':
        return moved({ kind: 'closed', reason: 'unsupported' }, { kind: 'disconnect' });
      case 'opened':
        // Anything may have changed while the feed was not open.
        return state.kind === 'connecting'
          ? moved({ kind: 'following', last: state.last }, { kind: 'refresh' })
          : ignored;
      case 'observation': {
        if (state.kind !== 'following' && state.kind !== 'connecting') return ignored;
        // Replays after a reconnect repeat positions already seen.
        if (event.seq !== null && state.last !== null && event.seq <= state.last) return ignored;
        const last = event.seq ?? state.last;
        return moved({ kind: 'following', last }, { kind: 'refresh' });
      }
      case 'lagged':
        return state.kind === 'following' ? moved(state, { kind: 'refresh' }) : ignored;
      case 'gone':
        // The browser would resume from the pruned position again: close first.
        return state.kind === 'following' || state.kind === 'connecting'
          ? moved({ kind: 'reloading' }, { kind: 'disconnect' }, { kind: 'reload' })
          : ignored;
      case 'store_error':
        return state.kind === 'following' || state.kind === 'connecting'
          ? moved({
              kind: 'connecting',
              last: state.last,
              failures: state.kind === 'connecting' ? state.failures + 1 : 1,
              problem: 'store_unavailable',
            })
          : ignored;
      case 'dropped': {
        if (state.kind !== 'following' && state.kind !== 'connecting') return ignored;
        const failures = state.kind === 'connecting' ? state.failures + 1 : 1;
        const problem =
          state.kind === 'connecting' && state.problem === 'store_unavailable'
            ? 'store_unavailable'
            : 'disconnected';
        const next: LiveState = { kind: 'connecting', last: state.last, failures, problem };
        return event.retry === 'browser'
          ? moved(next)
          : moved(
              next,
              { kind: 'disconnect' },
              {
                kind: 'connect',
                after: state.last,
                delayMs: LiveFollow.delay(failures),
              },
            );
      }
      case 'reloaded':
        return state.kind === 'reloading'
          ? moved(
              { kind: 'connecting', last: null, failures: 0, problem: 'none' },
              { kind: 'connect', after: null, delayMs: 0 },
            )
          : ignored;
      default:
        return assertNever(event);
    }
  }

  /** Parse one SSE message from `/api/live` into an event. */
  static message(type: string, data: string, lastEventId: string): LiveEvent | null {
    switch (type) {
      case 'observation':
        return { kind: 'observation', seq: LiveFollow.#seq(data, lastEventId) };
      case 'lagged':
        return { kind: 'lagged' };
      case 'gone':
        return { kind: 'gone' };
      case 'error':
        return { kind: 'store_error' };
      default:
        return null;
    }
  }

  /** The observation's position, or null when the data is malformed (still a change). */
  static #seq(data: string, lastEventId: string): ObservationSeq | null {
    try {
      const value: unknown = JSON.parse(data);
      if (!isRecord(value)) return null;
      const observation = observationView(value, 'observation');
      return /^\d+$/.test(lastEventId) && Number(lastEventId) !== observation.seq
        ? null
        : observation.seq;
    } catch {
      return null;
    }
  }

  /** The "Updated …" area: connection state, then when data was last loaded. */
  static summary(state: LiveState, updated: string | null): string {
    if (state.kind === 'closed' && state.reason === 'paused') return updated ?? 'Connecting…';
    return updated ? `${LiveFollow.describe(state)} · ${updated}` : LiveFollow.describe(state);
  }

  /** Short text for the connection-state area. */
  static describe(state: LiveState): string {
    switch (state.kind) {
      case 'closed':
        return state.reason === 'paused' ? 'Live updates off' : 'Live updates unavailable';
      case 'connecting':
        return state.problem === 'store_unavailable'
          ? 'Storage unavailable · reconnecting…'
          : state.problem === 'disconnected'
            ? 'Live feed lost · reconnecting…'
            : 'Connecting…';
      case 'following':
        return 'Live';
      case 'reloading':
        return 'History pruned · reloading…';
      default:
        return assertNever(state);
    }
  }
}
