import { error } from '@sveltejs/kit';
import { ApiError, ManagementApi } from '#lib/api.ts';
import { SessionDeskLoad } from '#lib/desk/load.ts';
import type { PageLoad } from './$types';

/** A project's sessions: the desk, holding only those working for it. */
export const load: PageLoad = async (event) => {
  const [desk, project] = await Promise.all([
    SessionDeskLoad.load(event),
    new ManagementApi(event.fetch).project(event.params.name).catch((failure: unknown) => {
      if (failure instanceof ApiError && failure.status === 404) {
        error(404, `No project is named ${event.params.name}.`);
      }
      throw failure;
    }),
  ]);
  return { ...desk, project };
};
