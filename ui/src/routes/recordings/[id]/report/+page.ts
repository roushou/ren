import { error } from '@sveltejs/kit';
import { RecordingId } from '#lib/api.ts';
import { reportSnapshot } from '#lib/loaders.ts';
import type { PageLoad } from './$types';
export const load: PageLoad = async ({ fetch, params, parent }) => {
  const sessionId = RecordingId.parse(params.id);
  if (!sessionId) error(404, 'This recording ID is invalid.');
  await parent(); // Serialize management reads after the shared snapshot.
  return { report: await reportSnapshot(fetch, sessionId), sessionId };
};
