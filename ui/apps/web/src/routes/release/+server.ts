import { redirect } from '@sveltejs/kit';
import { ManagementApi } from '#lib/api.ts';
import { RunsStartRead } from '#lib/server/runsStart.ts';
import type { RequestHandler } from './$types';

/** Release lives under each project now: `/release` leads to Ren's, and
 * to Projects while no project is Ren. */
export const GET: RequestHandler = async ({ request }) => {
  const projects = await new ManagementApi(RunsStartRead.fetcher(request))
    .projects()
    .catch(() => []);
  const ren = ManagementApi.ren(projects);
  redirect(307, ren ? `/projects/${encodeURIComponent(ren.name)}/release` : '/');
};
