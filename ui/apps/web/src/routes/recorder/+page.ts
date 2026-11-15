import { recorderSnapshot } from '#lib/loaders.ts';
import type { PageLoad } from './$types';
export const load: PageLoad = async ({ fetch, parent }) => {
  await parent(); // Serialize management reads after the shared snapshot.
  return { recorder: await recorderSnapshot(fetch) };
};
