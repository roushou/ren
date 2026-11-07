import { error } from '@sveltejs/kit';
import { TurnId } from '#lib/api.ts';
import { turnSnapshot } from '#lib/loaders.ts';
import type { PageLoad } from './$types';
export const load: PageLoad = async ({ fetch, params, parent }) => {
  const turnId = TurnId.parse(params.id);
  if (!turnId) error(404, 'This turn ID is invalid.');
  await parent(); // Serialize management reads after the shared snapshot.
  return { turn: await turnSnapshot(fetch, turnId), turnId };
};
