import { redirect } from '@sveltejs/kit';
import type { RequestHandler } from './$types';

/**
 * Recordings were called sessions: old links and bookmarks land on the
 * same page under its new name, with their query kept.
 */
export const GET: RequestHandler = ({ params, url }) => {
  const rest = params.path ? `/${params.path}` : '';
  redirect(308, `/recordings${rest}${url.search}`);
};
