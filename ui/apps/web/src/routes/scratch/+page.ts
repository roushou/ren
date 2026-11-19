import { ManagementApi } from '#lib/api.ts';
import { SessionDeskLoad } from '#lib/desk/load.ts';
import type { PageLoad } from './$types';

/** Scratch: the desk, holding the sessions that work for no project. Every
 * project is read, archived ones too, to tell which those are; a list that
 * fails to read leaves every session here. */
export const load: PageLoad = async (event) => {
  const [desk, projects] = await Promise.all([
    SessionDeskLoad.load(event),
    new ManagementApi(event.fetch).projects().catch(() => []),
  ]);
  return { ...desk, projects };
};
