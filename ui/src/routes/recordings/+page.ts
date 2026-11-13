import { browser } from '$app/env';
import { RecordingsReader } from '#lib/loaders.ts';
import type { PageLoad } from './$types';

// In the browser one reader keeps each session's newest turn between refreshes.
const reader = browser ? new RecordingsReader() : null;

export const load: PageLoad = async ({ fetch, parent, url }) => {
  await parent();
  const shelf = url.searchParams.get('shelf') === 'archived' ? 'archived' : 'current';
  return { sessions: await (reader ?? new RecordingsReader()).read(fetch, shelf), shelf };
};
