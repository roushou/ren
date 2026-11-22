import { redirect } from '@sveltejs/kit';
import { OldAddress } from '#lib/server/oldAddress.ts';
import type { RequestHandler } from './$types';

/**
 * Sessions live under their project now, or in Scratch: `/sessions`
 * itself leads to the session or run it names. Recordings were called
 * sessions before that: `/sessions/<rest>` lands on the same page under
 * its new name, with its query kept.
 */
export const GET: RequestHandler = async ({ params, url, request }) => {
  if (!params.path) redirect(307, await OldAddress.sessions(request, url));
  redirect(308, `/monitor/recordings/${params.path}${url.search}`);
};
