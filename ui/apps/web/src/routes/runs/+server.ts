import { redirect } from '@sveltejs/kit';
import type { RequestHandler } from './$types';

/**
 * Runs became sessions: old links land on the Sessions page with their
 * query kept, and a link to a run (`?run=`) shows the session it served.
 */
export const GET: RequestHandler = ({ url }) => {
  redirect(308, `/sessions${url.search}`);
};
