import { ManagementApi } from '#lib/api.ts';
import { WorkApi, type WorkItem } from '#lib/work.ts';
import type { PageLoad } from './$types';

/** The listing, or why there is none. */
export type WorkListing =
  { kind: 'ready'; items: WorkItem[] } | { kind: 'failed'; message: string };

export const load: PageLoad = async ({ fetch, parent, url }) => {
  await parent();
  const shelf = url.searchParams.get('shelf') === 'archived' ? 'archived' : 'current';
  const [listing, projects] = await Promise.all([
    new WorkApi(fetch)
      .list(shelf)
      .then((items): WorkListing => ({ kind: 'ready', items }))
      .catch((failure: unknown): WorkListing => ({
        kind: 'failed',
        message: failure instanceof Error ? failure.message : 'The work items did not read.',
      })),
    // Whose page each item is about, for its brief and where it is sent.
    new ManagementApi(fetch).projects().catch(() => []),
  ]);
  return { listing, projects, shelf, selected: url.searchParams.get('item') };
};
