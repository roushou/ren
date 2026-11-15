import { Memory, type RunsStart } from '#lib/runs.ts';
import { RunsStartRead } from '#lib/server/runsStart.ts';
import { Arrival } from '#lib/arrival.ts';
import type { PageServerLoad } from './$types';

/**
 * What this server read for the page: everything it starts with, nothing
 * because runs are off here, or nothing yet, on a navigation in the
 * browser whose read was not in within the grace period, where the page
 * reads it itself and draws its outline meanwhile.
 */
export type Opening = { kind: 'read'; start: RunsStart } | { kind: 'off' } | { kind: 'unread' };

class OpeningRead {
  /** On a navigation: what was read within the grace period, else nothing
   * yet, and the page reads it itself while it draws its outline. */
  static async within(read: Promise<RunsStart>): Promise<Opening> {
    const within = await Arrival.within(read, Arrival.GRACE_MS);
    switch (within.state) {
      case 'in':
        return { kind: 'read', start: within.value };
      case 'late':
        return { kind: 'unread' };
      default: {
        const never: never = within;
        return never;
      }
    }
  }
}

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
      ? await OpeningRead.within(RunsStartRead.read(request))
      : { kind: 'read', start: await RunsStartRead.read(request) };
  return { opening, remembered };
};
