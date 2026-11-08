// The `tgv run` command that starts a client's session through this gateway.
import type { Harness } from './api.ts';

/** The harnesses `tgv run` launches, by the name it takes for each. */
export const launchClients = [
  { run: 'claude', harness: 'claude_code' },
  { run: 'codex', harness: 'codex' },
  { run: 'pi', harness: 'pi' },
] as const satisfies readonly { run: string; harness: Harness }[];

export type LaunchClient = (typeof launchClients)[number]['run'];

export class LaunchCommand {
  /** The daemon's default listener, which `tgv run` reaches without `--gateway`. */
  static readonly defaultOrigins: readonly string[] = [
    'http://127.0.0.1:8787',
    'http://localhost:8787',
  ];

  readonly client: LaunchClient;
  /** The gateway to name, or `null` for the default listener. */
  readonly gateway: string | null;

  private constructor(client: LaunchClient, gateway: string | null) {
    this.client = client;
    this.gateway = gateway;
  }

  /** The command for `client`, naming `gatewayOrigin` unless it is the default listener. */
  static for(client: LaunchClient, gatewayOrigin: string): LaunchCommand {
    return new LaunchCommand(
      client,
      LaunchCommand.defaultOrigins.includes(gatewayOrigin) ? null : gatewayOrigin,
    );
  }

  get text(): string {
    return `tgv run ${this.client}${this.gateway === null ? '' : ` --gateway ${this.gateway}`}`;
  }
}
