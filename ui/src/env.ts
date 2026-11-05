import { defineEnvVars } from '@sveltejs/kit/env';
import { gatewayOrigin } from '#lib/server/config.ts';

export const variables = defineEnvVars({
  TGV_GATEWAY_URL: {
    description: 'Daemon origin reached by the UI server. Never the UI server itself.',
    schema: (value) => gatewayOrigin(value ?? 'http://127.0.0.1:8787'),
  },
  TGV_PUBLIC_GATEWAY_URL: {
    description: 'Optional daemon origin shown in client launch commands, e.g. its Tailscale URL.',
    schema: (value) => (value === undefined ? undefined : gatewayOrigin(value)),
  },
});
