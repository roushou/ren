import { TGV_DEV_CONSOLE_URL, TGV_GATEWAY_URL, TGV_PUBLIC_GATEWAY_URL } from '$app/env/private';
import type { LayoutServerLoad } from './$types';

export const load: LayoutServerLoad = () => ({
  gatewayUrl: TGV_PUBLIC_GATEWAY_URL ?? TGV_GATEWAY_URL,
  devConsoleUrl: TGV_DEV_CONSOLE_URL ?? null,
});
