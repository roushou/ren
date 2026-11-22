import { error } from '@sveltejs/kit';
import { Arrival } from '#lib/arrival.ts';
import { ApiError, ManagementApi } from '#lib/api.ts';
import { SessionDeskLoad } from '#lib/desk/load.ts';
import type { ProjectsData } from '#lib/loaders.ts';
import { ProjectBoard } from '#lib/projects.ts';
import type { PageLoad } from './$types';

/** A project's sessions: the desk, holding only those working for it. The
 * project comes from the layout's board; it is read here only when the
 * board has not read. */
export const load: PageLoad = async (event) => {
  const { board } = await event.parent();
  const projects = Arrival.value<ProjectsData>(board);
  if (ProjectBoard.unknown(projects, event.params.name)) {
    error(404, `No project is named ${event.params.name}.`);
  }
  const known = ProjectBoard.known(projects, event.params.name);
  const [desk, project] = await Promise.all([
    SessionDeskLoad.load(event),
    known ??
      new ManagementApi(event.fetch).project(event.params.name).catch((failure: unknown) => {
        if (failure instanceof ApiError && failure.status === 404) {
          error(404, `No project is named ${event.params.name}.`);
        }
        throw failure;
      }),
  ]);
  return { ...desk, project };
};
