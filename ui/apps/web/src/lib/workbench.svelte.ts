import { createContext } from 'svelte';
import { SvelteMap } from 'svelte/reactivity';
import type { CallId } from './api.ts';
import { LiveFollow, type LiveState } from './live.ts';
import { Streaming, type CallProgress } from './streaming.ts';

export class Workbench {
  /** Follow `/api/live` and refresh when calls change. */
  automatic = $state(true);
  refreshing = $state(false);
  /** Completed page refreshes; views that load their own data reload on change. */
  refreshed = $state(0);
  manual = $state(false);
  live = $state<LiveState>(LiveFollow.initial);
  /** Streaming calls' replies so far, from the live feed; never stored. */
  readonly streamed = new SvelteMap<CallId, CallProgress>();

  /** Keep a streaming call's newest reply so far. */
  stream(update: CallProgress) {
    Streaming.keep(this.streamed, update);
  }
}
export const [getWorkbench, setWorkbench] = createContext<Workbench>();
