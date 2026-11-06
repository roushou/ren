import { createContext, type Snippet } from 'svelte';
import type { CallView } from './api.ts';
import { LiveFollow, type LiveState } from './live.ts';

export class Workbench {
  /** Follow `/api/live` and refresh when calls change. */
  automatic = $state(true);
  refreshing = $state(false);
  /** Completed page refreshes; views that load their own data reload on change. */
  refreshed = $state(0);
  manual = $state(false);
  live = $state<LiveState>(LiveFollow.initial);
  selected = $state<CallView | undefined>();
  callCount = $state(0);
  activeCount = $state(0);
  /** What the current view puts in the scope bar: what you are looking at,
   * and the controls that change it. Unset, the bar names the view. */
  scope = $state.raw<Snippet | undefined>();

  /** Show `snippet` in the scope bar while the calling component lives.
   * Call it from a component's script, where effects can be created. */
  showScope(snippet: Snippet) {
    $effect(() => {
      this.scope = snippet;
      return () => {
        if (this.scope === snippet) this.scope = undefined;
      };
    });
  }
}
export const [getWorkbench, setWorkbench] = createContext<Workbench>();
