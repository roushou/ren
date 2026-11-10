/**
 * Where the gateway runs, read from the address clients reach it at: the
 * node's name and the kind of network, which the header shows instead of
 * the address. The part of an address that identifies the operator (a
 * tailnet's name, a public domain) can be masked for screenshots and screen
 * sharing; copying always uses the real address. Pure.
 */
import { assertNever } from './decode.ts';

/** How clients reach the gateway. */
export type Network =
  /** Only this machine: loopback. */
  | 'loopback'
  /** A Tailscale tailnet: a `*.ts.net` name or a Tailscale address. */
  | 'tailnet'
  /** A private network: a private address, a single-label or `.local` name. */
  | 'private'
  /** Anything else, reachable from the internet. */
  | 'public';

export class GatewayPlace {
  static readonly MASK = '•••';

  /** The address as given, an origin such as `https://ren.tail1.ts.net:8443`. */
  readonly origin: string;
  readonly network: Network;
  /** The node's name (an address's first label); null for an IP address. */
  readonly node: string | null;
  /** The origin with its identifying part masked; the origin when nothing is. */
  readonly masked: string;

  private constructor(origin: string, network: Network, node: string | null, masked: string) {
    this.origin = origin;
    this.network = network;
    this.node = node;
    this.masked = masked;
  }

  /** The place of `origin`; an address that does not parse is public and
   * shown masked whole. */
  static parse(origin: string): GatewayPlace {
    let url: URL;
    try {
      url = new URL(origin);
    } catch {
      return new GatewayPlace(origin, 'public', null, GatewayPlace.MASK);
    }
    const host = url.hostname.replace(/^\[|\]$/g, '').toLowerCase();
    const network = GatewayPlace.network(host);
    const ip = GatewayPlace.isIp(host);
    const node = ip ? null : (host.split('.')[0] ?? null);
    const secret = GatewayPlace.secret(host, network, ip);
    const masked = secret ? origin.replace(url.hostname, secret) : origin;
    return new GatewayPlace(origin, network, node, masked);
  }

  /** What the header shows: the node and how it is reached. */
  get label(): string {
    switch (this.network) {
      case 'loopback':
        return 'this machine';
      case 'tailnet':
      case 'private':
      case 'public':
        return this.node
          ? `${this.node} · ${GatewayPlace.networkLabel(this.network)}`
          : GatewayPlace.networkLabel(this.network);
      default:
        return assertNever(this.network);
    }
  }

  /** Whether the address has a part worth masking. */
  get maskable(): boolean {
    return this.masked !== this.origin;
  }

  /** `text` with every occurrence of the address masked. */
  mask(text: string): string {
    return this.maskable ? text.replaceAll(this.origin, this.masked) : text;
  }

  static networkLabel(network: Network): string {
    switch (network) {
      case 'loopback':
        return 'loopback';
      case 'tailnet':
        return 'tailnet';
      case 'private':
        return 'local network';
      case 'public':
        return 'public';
      default:
        return assertNever(network);
    }
  }

  private static network(host: string): Network {
    if (host === 'localhost' || host.endsWith('.localhost') || host === '::1') return 'loopback';
    if (host.endsWith('.ts.net')) return 'tailnet';
    const v4 = GatewayPlace.ipv4(host);
    if (v4) {
      const [a, b] = v4;
      if (a === 127) return 'loopback';
      // Tailscale assigns 100.64.0.0/10.
      if (a === 100 && b >= 64 && b <= 127) return 'tailnet';
      if (a === 10 || (a === 172 && b >= 16 && b <= 31) || (a === 192 && b === 168))
        return 'private';
      if (a === 169 && b === 254) return 'private';
      return 'public';
    }
    if (host.includes(':')) {
      if (host.startsWith('fd7a:115c:a1e0:')) return 'tailnet';
      if (/^f[cd]/.test(host) || host.startsWith('fe80:')) return 'private';
      return 'public';
    }
    if (!host.includes('.') || host.endsWith('.local') || host.endsWith('.lan')) return 'private';
    return 'public';
  }

  /** The masked host, or null when nothing in it identifies the operator. */
  private static secret(host: string, network: Network, ip: boolean): string | null {
    switch (network) {
      case 'loopback':
      case 'private':
        return null;
      case 'tailnet':
        // `node.tailnet.ts.net`: the tailnet's name identifies its owner.
        return ip ? GatewayPlace.MASK : `${host.split('.')[0]}.${GatewayPlace.MASK}.ts.net`;
      case 'public':
        return ip ? GatewayPlace.MASK : `${host.split('.')[0]}.${GatewayPlace.MASK}`;
      default:
        return assertNever(network);
    }
  }

  private static isIp(host: string): boolean {
    return GatewayPlace.ipv4(host) !== null || host.includes(':');
  }

  private static ipv4(host: string): number[] | null {
    const parts = host.split('.');
    if (parts.length !== 4 || !parts.every((part) => /^\d{1,3}$/.test(part))) return null;
    const octets = parts.map(Number);
    return octets.every((octet) => octet <= 255) ? octets : null;
  }
}
