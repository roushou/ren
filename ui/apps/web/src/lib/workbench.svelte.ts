import { createContext } from 'svelte';
import { SvelteMap } from 'svelte/reactivity';
import { navigating } from '$app/state';
import type { CallId } from './api.ts';
import { LiveFollow, type LiveState } from './live.ts';
import type { LiveConnection } from './live.svelte.ts';
import { Streaming, type CallProgress } from './streaming.ts';

export class Workbench {
  /** Follow `/api/live` and refresh when calls change. */
  automatic = $state(true);
  /** Completed page refreshes; views that load their own data reload on change. */
  refreshed = $state(0);
  /** A refresh the operator asked for is under way. */
  manual = $state(false);
  /** The live feed, once the layout has opened it. */
  connection = $state.raw<LiveConnection | null>(null);

  /** How the live feed stands. */
  get live(): LiveState {
    return this.connection?.state ?? LiveFollow.initial;
  }

  /** A refresh or a navigation is under way. */
  get loading(): boolean {
    return this.manual || !!navigating.to;
  }
  /** Streaming calls' replies so far, from the live feed; never stored. */
  readonly streamed = new SvelteMap<CallId, CallProgress>();

  /** Keep a streaming call's newest reply so far. */
  stream(update: CallProgress) {
    Streaming.keep(this.streamed, update);
  }
}
export const [getWorkbench, setWorkbench] = createContext<Workbench>();
