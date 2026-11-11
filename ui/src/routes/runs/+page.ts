import { RunId } from '#lib/runs.ts';
import type { PageLoad } from './$types';

// Runs are read and followed in the browser: their state changes by the second.
export const load: PageLoad = async ({ url, parent }) => {
  const { runsControl } = await parent();
  return { runsControl, selected: RunId.parse(url.searchParams.get('run')) };
};
