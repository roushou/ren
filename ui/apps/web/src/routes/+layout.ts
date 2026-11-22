import { Arrival } from '#lib/arrival.ts';
import { gatewaySnapshot, projectsData } from '#lib/loaders.ts';
import type { LayoutLoad } from './$types';

/** The gateway's state and every project's board, for the top bar, Pending
 * and the pages that show them. On a navigation that reads them again (a
 * refresh was under way), they arrive like any page's read: within the
 * grace period, or pending while the newest stays on screen. */
export const load: LayoutLoad = async ({ fetch, data }) => {
  const [gateway, board] = await Promise.all([
    gatewaySnapshot(fetch),
    Arrival.read('board', () => projectsData(fetch, data.runsControl)),
  ]);
  return { ...data, gateway, board };
};
