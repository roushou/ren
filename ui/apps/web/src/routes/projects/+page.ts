import { projectSparks } from '#lib/loaders.ts';
import type { PageLoad } from './$types';

/** The Projects view reads what the layout read for Pending, the same board,
 * and each project's calls over the last day. */
export const load: PageLoad = async ({ parent, fetch }) => {
  const [{ board }, sparks] = await Promise.all([parent(), projectSparks(fetch, Date.now())]);
  return { board, sparks };
};
