// The Packages page's first read, made on this server so the page renders with
// it: through the same control proxy the browser uses, with the browser's
// Tailscale login, and never its cookies.
import { proxy } from '#lib/server/runsProxy.ts';
import { PackagesApi, type PackageListing } from '#lib/packages.ts';
import { Arrival } from '#lib/arrival.ts';
import { RunsClient, type RunClient } from '#lib/runs.ts';
import type { PageServerLoad } from './$types';

export type PackagesInitial =
  { kind: 'ready'; listing: PackageListing } | { kind: 'failed'; message: string };

/** The read as the page receives it: done, or still on its way. */
export type PackagesArriving =
  PackagesInitial | { kind: 'pending'; read: Promise<PackagesInitial> };

/** The listing on a navigation: whole if it is in within the grace period, else streamed. */
class PackagesRead {
  /** The clients' sign-ins, if they are in within the grace period; else the
   * page reads them itself. */
  static async inTime(read: Promise<RunClient[] | null>): Promise<RunClient[] | null> {
    const within = await Arrival.within(read, Arrival.GRACE_MS);
    switch (within.state) {
      case 'in':
        return within.value;
      case 'late':
        return null;
      default: {
        const never: never = within;
        return never;
      }
    }
  }

  static async arriving(read: Promise<PackagesInitial>): Promise<PackagesArriving> {
    const within = await Arrival.within(read, Arrival.GRACE_MS);
    switch (within.state) {
      case 'in':
        return within.value;
      case 'late':
        return { kind: 'pending', read };
      default: {
        const never: never = within;
        return never;
      }
    }
  }
}

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
  const api = new PackagesApi(proxied);
  // Each client's sign-in, read with the listing so the clients' rows show
  // it from the first paint; null when runs are off or it was not in time.
  const clientsRead = new RunsClient(proxied).clients().catch(() => null);
  const read: Promise<PackagesInitial> = api.listing().then(
    (listing) => ({ kind: 'ready', listing }),
    (error: unknown) => ({
      kind: 'failed',
      message: error instanceof Error ? error.message : String(error),
    }),
  );
  // A first load is read in full, so the page renders as it will stay. On a
  // navigation in the browser a read that is in within the grace period
  // arrives whole too; a later one is streamed, and the page draws its
  // outline until it arrives.
  const [initial, clients] = await Promise.all([
    isDataRequest ? PackagesRead.arriving(read) : read,
    isDataRequest ? PackagesRead.inTime(clientsRead) : clientsRead,
  ]);
  return { initial: initial satisfies PackagesArriving, clients };
};
