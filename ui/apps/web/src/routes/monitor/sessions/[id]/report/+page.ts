import { error } from '@sveltejs/kit';
import { RecordingId } from '#lib/api.ts';
import { reportSnapshot } from '#lib/loaders.ts';
import { RunsClient, Sessions } from '#lib/runs.ts';
import type { PageLoad } from './$types';
export const load: PageLoad = async ({ fetch, params, parent }) => {
  const recordingId = RecordingId.parse(params.id);
  if (!recordingId) error(404, 'This recording ID is invalid.');
  await parent(); // Serialize management reads after the shared snapshot.
  const desk = await new RunsClient(fetch).sessions().then(
    (bodies) => Sessions.byRecording(bodies).get(recordingId) ?? null,
    () => null,
  );
  return {
    report: await reportSnapshot(fetch, recordingId),
    recordingId,
    /** Where the session opens on its desk, when Ren drives it. */
    desk: desk ? Sessions.href(desk) : null,
  };
};
