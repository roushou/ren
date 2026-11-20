import { redirect } from '@sveltejs/kit';
import { ProjectBoard } from '#lib/projects.ts';
import type { PageLoad } from './$types';

/** A project opens on its sessions. */
export const load: PageLoad = ({ params, url }) => {
  redirect(307, `${ProjectBoard.href(params.name)}${url.search}`);
};
