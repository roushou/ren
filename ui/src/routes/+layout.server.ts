import {
  REN_CONTROL_TOKEN_FILE,
  REN_DEV_TRAFFIC_URL,
  REN_GATEWAY_URL,
  REN_PUBLIC_GATEWAY_URL,
  REN_SOURCE_DIR,
} from '$app/env/private';
import type { LayoutServerLoad } from './$types';

export const load: LayoutServerLoad = () => ({
  gatewayUrl: REN_PUBLIC_GATEWAY_URL ?? REN_GATEWAY_URL,
  /** Whether the dev toolbar's traffic controls are reachable. */
  devTraffic: REN_DEV_TRAFFIC_URL !== undefined,
  /** Whether this UI server drives runs; the daemon may still have them off. */
  runsControl: REN_CONTROL_TOKEN_FILE !== undefined,
  /** Ren's source checkout, where Redline sends its briefs; null when unset. */
  redlineSource: REN_SOURCE_DIR ?? null,
});
