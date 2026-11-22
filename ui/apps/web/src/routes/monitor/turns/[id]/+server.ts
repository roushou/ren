import { error, redirect } from '@sveltejs/kit';
import { ManagementApi, TurnId } from '#lib/api.ts';
import type { RequestHandler } from './$types';

/**
 * A turn is read beside its session's calls: `/monitor/turns/<id>` leads to
 * that session with the turn's panel open, so a link to a turn still lands
 * on it.
 */
export const GET: RequestHandler = async ({ params, fetch }) => {
  const id = TurnId.parse(params.id);
  if (!id) error(404, 'This turn ID is invalid.');
  const turn = await new ManagementApi(fetch).turn(id).catch(() => null);
  if (!turn) error(404, 'No turn has that ID.');
  redirect(
    307,
    `/monitor/sessions/${encodeURIComponent(turn.recording)}?${new URLSearchParams({ turn: id })}`,
  );
};
