import { error } from '@sveltejs/kit';
import { ApiError, ManagementApi } from '#lib/api.ts';
import { Work, WorkApi, type WorkItem } from '#lib/work.ts';
import type { PageLoad } from './$types';

/** The listing, or why there is none. */
export type WorkListing =
  { kind: 'ready'; items: WorkItem[] } | { kind: 'failed'; message: string };

/** A project's work items on one shelf, read whole and kept to the
 * project's, so Ren's holds those from before projects too. */
export const load: PageLoad = async ({ fetch, parent, params, url }) => {
  await parent();
  const shelf = url.searchParams.get('shelf') === 'archived' ? 'archived' : 'current';
  const api = new ManagementApi(fetch);
  const [project, items, projects] = await Promise.all([
    api.project(params.name).catch((failure: unknown) => {
      if (failure instanceof ApiError && failure.status === 404) {
        error(404, `No project is named ${params.name}.`);
      }
      throw failure;
    }),
    new WorkApi(fetch)
      .list(shelf)
      .then((items): WorkListing => ({ kind: 'ready', items }))
      .catch((failure: unknown): WorkListing => ({
        kind: 'failed',
        message: failure instanceof Error ? failure.message : 'The work items did not read.',
      })),
    // Whose page each item is about, for its brief.
    api.projects().catch(() => []),
  ]);
  const listing: WorkListing =
    items.kind === 'ready' ? { kind: 'ready', items: Work.of(items.items, project) } : items;
  return { project, listing, projects, shelf, selected: url.searchParams.get('item') };
};
