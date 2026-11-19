// The Sessions page's first reads, made on this server so the page renders with
// them: through the same control proxy and management surface the browser
// uses, with the browser's Tailscale login, and never its cookies.
import { REN_GATEWAY_URL } from '$app/env/private';
import { RunsClient, type RunsStart } from '#lib/runs.ts';
import { managementResponse } from './management.ts';
import { proxy } from './runsProxy.ts';

export class RunsStartRead {
  /** The reads for `request`, the page's own request. */
  static async read(request: Request): Promise<RunsStart> {
    return new RunsClient(RunsStartRead.fetcher(request)).opening();
  }

  /** A `fetch` for reads made here on behalf of `request`: control reads
   * through the proxy, the rest through the management surface. */
  static fetcher(request: Request): typeof fetch {
    const login = request.headers.get('tailscale-user-login');
    return (async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = new URL(String(input), request.url);
      // A GET from this server: no Origin, no cookies; the login as sent.
      const inner = new Request(url, {
        method: 'GET',
        headers: login === null ? {} : { 'tailscale-user-login': login },
        signal: init?.signal ?? null,
      });
      return url.pathname.startsWith('/control/')
        ? proxy.respond(inner)
        : managementResponse(REN_GATEWAY_URL, inner);
    }) as typeof fetch;
  }
}
