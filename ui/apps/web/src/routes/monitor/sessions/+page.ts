import { browser } from '$app/env';
import { RecordingsReader, deskLinks } from '#lib/loaders.ts';
import type { PageLoad } from './$types';

// In the browser one reader keeps each recording's newest turn between refreshes.
const reader = browser ? new RecordingsReader() : null;

export const load: PageLoad = async ({ fetch, parent, url }) => {
  await parent();
  const shelf = url.searchParams.get('shelf') === 'archived' ? 'archived' : 'current';
  const project = url.searchParams.get('project');
  const [recordings, desks] = await Promise.all([
    (reader ?? new RecordingsReader()).read(fetch, shelf, project),
    deskLinks(fetch),
  ]);
  /** `desks`: the sessions Ren drives, by recording. */
  return { recordings, desks, shelf, project };
};
