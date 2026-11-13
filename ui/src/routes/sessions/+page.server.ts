import { Memory } from '#lib/runs.ts';
import { RunsStartRead } from '#lib/server/runsStart.ts';
import type { PageServerLoad } from './$types';

// The page renders with what runs, places, and clients are now; the feed
// keeps it current from there. The URL is not read here, so choosing a run
// does not read everything again.
export const load: PageServerLoad = async ({ request, parent, cookies }) => {
  const { runsControl } = await parent();
  /** The choices last started with, as this browser kept them. */
  const remembered = cookies.get(Memory.COOKIE) ?? null;
  return {
    initial: runsControl ? await RunsStartRead.read(request) : null,
    remembered,
  };
};
