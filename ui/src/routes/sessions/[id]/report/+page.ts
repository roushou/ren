import { error } from '@sveltejs/kit';
import { SessionId } from '#lib/api.ts';
import { reportSnapshot } from '#lib/loaders.ts';
import type { PageLoad } from './$types';
export const load: PageLoad = async ({ fetch, params, parent }) => {
  const sessionId = SessionId.parse(params.id);
  if (!sessionId) error(404, 'This session ID is invalid.');
  await parent(); // Serialize management reads after the shared snapshot.
  return { report: await reportSnapshot(fetch, sessionId), sessionId };
};
