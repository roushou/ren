import { WorkApi, type WorkItem } from '#lib/work.ts';
import type { PageLoad } from './$types';

/** The listing, or why there is none. */
export type WorkListing =
  { kind: 'ready'; items: WorkItem[] } | { kind: 'failed'; message: string };

export const load: PageLoad = async ({ fetch, parent, url }) => {
  await parent();
  const shelf = url.searchParams.get('shelf') === 'archived' ? 'archived' : 'current';
  const listing: WorkListing = await new WorkApi(fetch)
    .list(shelf)
    .then((items): WorkListing => ({ kind: 'ready', items }))
    .catch((failure: unknown): WorkListing => ({
      kind: 'failed',
      message: failure instanceof Error ? failure.message : 'The work items did not read.',
    }));
  return { listing, shelf, selected: url.searchParams.get('item') };
};
