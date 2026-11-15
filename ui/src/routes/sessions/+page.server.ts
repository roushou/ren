import { Memory, type RunsStart } from '#lib/runs.ts';
import { RunsStartRead } from '#lib/server/runsStart.ts';
import type { PageServerLoad } from './$types';

/**
 * What this server read for the page: everything it starts with, nothing
 * because runs are off here, or nothing yet, on a navigation in the
 * browser, where the page reads it itself and draws its outline meanwhile.
 */
export type Opening = { kind: 'read'; start: RunsStart } | { kind: 'off' } | { kind: 'unread' };

// A first load renders with what runs, places, and clients are now; the feed
// keeps it current from there. The URL is not read here, so choosing a run
// does not read everything again.
export const load: PageServerLoad = async ({ request, parent, cookies, isDataRequest }) => {
  const { runsControl } = await parent();
  /** The choices last started with, as this browser kept them. */
  const remembered = cookies.get(Memory.COOKIE) ?? null;
  const opening: Opening = !runsControl
    ? { kind: 'off' }
    : isDataRequest
      ? { kind: 'unread' }
      : { kind: 'read', start: await RunsStartRead.read(request) };
  return { opening, remembered };
};
