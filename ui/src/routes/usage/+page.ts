import { usageSnapshot } from '#lib/loaders.ts';
import type { PageLoad } from './$types';
export const load: PageLoad = async ({ fetch, url }) => ({
  // Read by name, so only a change of range reloads the page.
  usage: await usageSnapshot(fetch, url.searchParams.get('range')),
});
