import { error } from '@sveltejs/kit';
import { RecordingId } from '#lib/api.ts';
import { deskLinks, trafficSnapshot } from '#lib/loaders.ts';
import type { PageLoad } from './$types';
export const load: PageLoad = async ({ fetch, url, params, parent }) => {
  const recordingId = RecordingId.parse(params.id);
  if (!recordingId) error(404, 'This recording ID is invalid.');
  await parent(); // Serialize management reads after the shared snapshot.
  const [traffic, desks] = await Promise.all([
    trafficSnapshot(fetch, url, recordingId),
    deskLinks(fetch),
  ]);
  /** Which recordings are sessions Ren drives, for the link to this one's desk. */
  return { traffic, recordingId, desks };
};
