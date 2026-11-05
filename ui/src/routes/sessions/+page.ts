import { sessionsSnapshot } from '#lib/loaders.ts';
import type { PageLoad } from './$types';
export const load: PageLoad = async ({ fetch, parent }) => {
  await parent();
  return { sessions: await sessionsSnapshot(fetch) };
};
