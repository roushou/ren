// The Tools page's first read, made on this server so the page renders with
// it: through the same control proxy the browser uses, with the browser's
// Tailscale login, and never its cookies.
import { proxy } from '#lib/server/runsProxy.ts';
import { ToolsApi, type ToolListing } from '#lib/tools.ts';
import { Arrival } from '#lib/arrival.ts';
import type { PageServerLoad } from './$types';

export type ToolsInitial =
  { kind: 'ready'; listing: ToolListing } | { kind: 'failed'; message: string };

/** The read as the page receives it: done, or still on its way. */
export type ToolsArriving = ToolsInitial | { kind: 'pending'; read: Promise<ToolsInitial> };

/** The listing on a navigation: whole if it is in within the grace period, else streamed. */
class Tools {
  static async arriving(read: Promise<ToolsInitial>): Promise<ToolsArriving> {
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
  const api = new ToolsApi((async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = new URL(String(input), request.url);
    // A GET from this server: no Origin, no cookies; the login as sent.
    return proxy.respond(
      new Request(url, {
        method: 'GET',
        headers: login === null ? {} : { 'tailscale-user-login': login },
        signal: init?.signal ?? null,
      }),
    );
  }) as typeof fetch);
  const read: Promise<ToolsInitial> = api.listing().then(
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
  const initial: ToolsArriving = isDataRequest ? await Tools.arriving(read) : await read;
  return { initial };
};
