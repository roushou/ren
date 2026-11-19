import { redirect } from '@sveltejs/kit';
import { OldAddress } from '#lib/server/oldAddress.ts';
import type { RequestHandler } from './$types';

/** Work lives under each project now: `/work?item=` leads to the item on
 * its project's Work page, and `/work` alone to Projects. */
export const GET: RequestHandler = async ({ url, request }) => {
  redirect(307, await OldAddress.work(request, url));
};
