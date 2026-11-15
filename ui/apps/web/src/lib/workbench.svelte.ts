import { createContext } from 'svelte';
import type { CallView } from './api.ts';
import { SvelteMap } from 'svelte/reactivity';
import type { CallId } from './api.ts';
import { LiveFollow, type LiveState } from './live.ts';
import { Streaming, type CallProgress } from './streaming.ts';

/** The layout's part of every page's toolbar. */
export interface ToolbarControls {
  refresh(): void;
  /** A refresh or a navigation is under way. */
  loading(): boolean;
  /** Why the shown data may be out of date; empty when it is not. */
  problem(): string;
}

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
  selected = $state<CallView | undefined>();
  callCount = $state(0);
  activeCount = $state(0);
  /**
   * What the toolbar offers from the layout: refreshing, whether a refresh
   * or navigation is under way, and why the shown data may be out of date.
   * The layout sets it as it starts, before any page renders a toolbar.
   */
  controls: ToolbarControls | null = null;

  /** Keep a streaming call's newest reply so far. */
  stream(update: CallProgress) {
    Streaming.keep(this.streamed, update);
  }
}
export const [getWorkbench, setWorkbench] = createContext<Workbench>();
