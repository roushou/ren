import { Arrival } from '#lib/arrival.ts';
import { ManagementApi } from '#lib/api.ts';
import { SessionDeskLoad } from '#lib/desk/load.ts';
import type { ProjectsData } from '#lib/loaders.ts';
import type { PageLoad } from './$types';

/** Scratch: the desk, holding the sessions that work for no project. Every
 * project, archived ones too, tells which those are: the layout's board has
 * them, and they are read here only when it has not read; a list that fails
 * to read leaves every session here. */
export const load: PageLoad = async (event) => {
  const { board } = await event.parent();
  const read = Arrival.value<ProjectsData>(board);
  const [desk, projects] = await Promise.all([
    SessionDeskLoad.load(event),
    read && read.failed === null
      ? read.projects
      : new ManagementApi(event.fetch).projects().catch(() => []),
  ]);
  return { ...desk, projects };
};
