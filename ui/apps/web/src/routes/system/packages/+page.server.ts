// The Packages page's reads, made on this server so a first load renders
// with them: through the same control proxy the browser uses, with the
// browser's Tailscale login, and never its cookies. On a navigation they
// arrive like any page's read (`Arrival`).
import { proxy } from '#lib/server/runsProxy.ts';
import { PackagesApi } from '#lib/packages.ts';
import { Arrival } from '#lib/arrival.ts';
import { readSnapshot } from '#lib/snapshots.ts';
import { RunsClient } from '#lib/runs.ts';
import type { PageServerLoad } from './$types';

export const load: PageServerLoad = async ({ request, isDataRequest }) => {
  const login = request.headers.get('tailscale-user-login');
  const proxied = (async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = new URL(String(input), request.url);
    // A GET from this server: no Origin, no cookies; the login as sent.
    return proxy.respond(
      new Request(url, {
        method: 'GET',
        headers: login === null ? {} : { 'tailscale-user-login': login },
        signal: init?.signal ?? null,
      }),
    );
  }) as typeof fetch;
  // Each client's sign-in is read with the listing, so the clients' rows show
  // it from the first paint; it fails when runs are off here.
  const [listing, clients] = await Promise.all([
    Arrival.over(
      'packages',
      readSnapshot('packages', () => new PackagesApi(proxied).listing()),
      isDataRequest,
    ),
    Arrival.over(
      'packages:clients',
      readSnapshot('packages:clients', () => new RunsClient(proxied).clients()),
      isDataRequest,
    ),
  ]);
  return { listing, clients };
};
