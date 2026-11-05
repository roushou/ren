// Thin browser shell for `LiveFollow`: owns the EventSource, timers, and the
// coalesced refresh. All decisions are made by the pure state machine.
import { LiveFollow, type LiveAction, type LiveEvent, type LiveState } from './live.ts';
import { assertNever } from './decode.ts';
import type { ObservationSeq } from './api.ts';

export interface LiveHooks {
  /** Refresh page data. */
  refresh: () => Promise<void>;
  /** Reload all page data after the resume position was pruned. */
  reload: () => Promise<void>;
}

const refreshDelayMs = 250;
const messageTypes = ['observation', 'lagged', 'gone'] as const;

export class LiveConnection {
  state = $state<LiveState>(LiveFollow.initial);
  readonly #hooks: LiveHooks;
  #source: EventSource | null = null;
  #connectTimer: ReturnType<typeof setTimeout> | undefined;
  #refreshTimer: ReturnType<typeof setTimeout> | undefined;
  #refreshing: Promise<void> | null = null;
  #again = false;
  #hiddenPending = false;
  #destroyed = false;

  constructor(hooks: LiveHooks) {
    this.#hooks = hooks;
    document.addEventListener('visibilitychange', this.#visible);
  }

  dispatch(event: LiveEvent) {
    if (this.#destroyed) return;
    const step = LiveFollow.step(this.state, event);
    if (step.kind === 'ignored') return;
    this.state = step.state;
    for (const action of step.actions) this.#run(action);
  }

  /** Refresh now. Runs one at a time; a request during a run queues one more. */
  refresh(): Promise<void> {
    if (this.#refreshing) {
      this.#again = true;
      return this.#refreshing;
    }
    this.#refreshing = (async () => {
      try {
        do {
          this.#again = false;
          await this.#hooks.refresh();
        } while (this.#again && !this.#destroyed);
      } finally {
        this.#refreshing = null;
      }
    })();
    return this.#refreshing;
  }

  destroy() {
    this.#destroyed = true;
    this.#close();
    clearTimeout(this.#refreshTimer);
    document.removeEventListener('visibilitychange', this.#visible);
  }

  #run(action: LiveAction) {
    switch (action.kind) {
      case 'connect':
        this.#close();
        this.#connectTimer = setTimeout(() => this.#open(action.after), action.delayMs);
        return;
      case 'disconnect':
        this.#close();
        return;
      case 'refresh':
        this.#schedule();
        return;
      case 'reload':
        void this.#hooks
          .reload()
          .catch(() => {})
          .finally(() => this.dispatch({ kind: 'reloaded' }));
        return;
      default:
        assertNever(action);
    }
  }

  #open(after: ObservationSeq | null) {
    if (typeof EventSource === 'undefined') {
      this.dispatch({ kind: 'unsupported' });
      return;
    }
    const source = new EventSource(after === null ? '/api/live' : `/api/live?after=${after}`);
    this.#source = source;
    // Events from a source we already replaced or closed are dropped here.
    const from = (event: LiveEvent | null) => {
      if (event && source === this.#source) this.dispatch(event);
    };
    source.addEventListener('open', () => from({ kind: 'opened' }));
    for (const type of messageTypes) {
      source.addEventListener(type, (event) =>
        from(LiveFollow.message(type, event.data, event.lastEventId)),
      );
    }
    source.addEventListener('error', (event) =>
      from(
        // A server-sent `error` event carries data; a connection error does not.
        event instanceof MessageEvent
          ? LiveFollow.message('error', String(event.data ?? ''), event.lastEventId)
          : {
              kind: 'dropped',
              retry: source.readyState === EventSource.CLOSED ? 'closed' : 'browser',
            },
      ),
    );
  }

  #close() {
    clearTimeout(this.#connectTimer);
    this.#connectTimer = undefined;
    this.#source?.close();
    this.#source = null;
  }

  #schedule() {
    if (this.#refreshTimer !== undefined) return;
    this.#refreshTimer = setTimeout(() => {
      this.#refreshTimer = undefined;
      if (document.hidden) this.#hiddenPending = true;
      else void this.refresh();
    }, refreshDelayMs);
  }

  #visible = () => {
    if (!document.hidden && this.#hiddenPending) {
      this.#hiddenPending = false;
      this.#schedule();
    }
  };
}
