import { browser } from '$app/env';
import { RecordingsReader } from '#lib/loaders.ts';
import { RunsClient, Sessions } from '#lib/runs.ts';
import type { PageLoad } from './$types';

// In the browser one reader keeps each recording's newest turn between refreshes.
const reader = browser ? new RecordingsReader() : null;

export const load: PageLoad = async ({ fetch, parent, url }) => {
  await parent();
  const shelf = url.searchParams.get('shelf') === 'archived' ? 'archived' : 'current';
  const project = url.searchParams.get('project');
  return {
    recordings: await (reader ?? new RecordingsReader()).read(fetch, shelf, project),
    /** The sessions Ren drives, by recording; none when it drives none here. */
    desks: await new RunsClient(fetch).sessions().then(Sessions.byRecording, () => new Map()),
    shelf,
    project,
  };
};
