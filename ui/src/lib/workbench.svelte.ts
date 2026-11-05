import { createContext } from 'svelte';
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
}
export const [getWorkbench, setWorkbench] = createContext<Workbench>();
