import { searchSnapshot } from '#lib/loaders.ts';
import type { PageLoad } from './$types';
export const load: PageLoad = async ({ fetch, url, parent }) => {
  await parent(); // Serialize management reads after the shared snapshot.
  return { search: await searchSnapshot(fetch, url) };
};
