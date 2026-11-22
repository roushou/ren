import { redirect } from '@sveltejs/kit';
import { OldAddress } from '#lib/server/oldAddress.ts';
import type { RequestHandler } from './$types';

/**
 * Sessions live under their project now, or in Scratch: `/sessions`
 * itself leads to the session or run it names. `/sessions/<rest>` is a
 * session as the gateway recorded it, now under Monitor's Sessions, with
 * its query kept.
 */
export const GET: RequestHandler = async ({ params, url, request }) => {
  if (!params.path) redirect(307, await OldAddress.sessions(request, url));
  redirect(308, `/monitor/sessions/${params.path}${url.search}`);
};
