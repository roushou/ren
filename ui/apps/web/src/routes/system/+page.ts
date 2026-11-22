import { githubWatches } from '#lib/loaders.ts';
import type { PageLoad } from './$types';

/** System's summary: the layout's gateway read, the packages the server
 * read, and the GitHub watches. */
export const load: PageLoad = async ({ fetch, data, parent }) => {
  await parent();
  return { ...data, github: await githubWatches(fetch) };
};
