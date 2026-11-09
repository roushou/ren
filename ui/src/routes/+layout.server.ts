import { TGV_DEV_TRAFFIC_URL, TGV_GATEWAY_URL, TGV_PUBLIC_GATEWAY_URL } from '$app/env/private';
import type { LayoutServerLoad } from './$types';

export const load: LayoutServerLoad = () => ({
  gatewayUrl: TGV_PUBLIC_GATEWAY_URL ?? TGV_GATEWAY_URL,
  /** Whether the dev toolbar's traffic controls are reachable. */
  devTraffic: TGV_DEV_TRAFFIC_URL !== undefined,
});
