import { defineEnvVars } from '@sveltejs/kit/env';
import { devTrafficOrigin, gatewayOrigin } from '#lib/server/config.ts';
import { RunsAccessConfig } from '#lib/server/control.ts';

export const variables = defineEnvVars({
  REN_GATEWAY_URL: {
    description: 'Daemon origin reached by the UI server. Never the UI server itself.',
    schema: (value) => gatewayOrigin(value ?? 'http://127.0.0.1:8787'),
  },
  REN_PUBLIC_GATEWAY_URL: {
    description: 'Optional daemon origin shown in client launch commands, e.g. its Tailscale URL.',
    schema: (value) => (value === undefined ? undefined : gatewayOrigin(value)),
  },
  REN_DEV_TRAFFIC_URL: {
    description:
      'Dev slot only (`cargo xtask dev`): the loopback origin of its fake traffic controls, which the dev toolbar uses.',
    schema: (value) => (value === undefined ? undefined : devTrafficOrigin(value)),
  },
  REN_CONTROL_TOKEN_FILE: {
    description:
      "Optional: the daemon's control token file (`<data dir>/control.token`). Set, the UI can start and drive agent runs; unset, it stays read-only.",
    schema: (value) => RunsAccessConfig.tokenFile(value),
  },
  REN_CONTROL_LOGINS: {
    description:
      'Optional, comma-separated Tailscale logins allowed to drive runs through the UI. Set it whenever the UI is published with Tailscale Serve.',
    schema: (value) => RunsAccessConfig.logins(value),
  },
});
