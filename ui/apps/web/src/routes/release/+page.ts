import { ManagementApi, type ProjectRelease, type ReleaseBody } from '#lib/api.ts';
import { WorkApi, type WorkItem } from '#lib/work.ts';
import type { PageLoad } from './$types';

/** What the Release view reads; a read that fails leaves its part empty. */
export interface ReleaseData {
  body: ReleaseBody | null;
  /** Why the release did not read, when it did not. */
  failed: string | null;
  projects: ProjectRelease[];
  /** Work items under way and done: what was shipped, and what waits. */
  work: WorkItem[];
}

export const load: PageLoad = async ({ fetch, parent }): Promise<ReleaseData> => {
  await parent();
  const api = new ManagementApi(fetch);
  const work = new WorkApi(fetch);
  const [body, projects, current, archived] = await Promise.all([
    api.releaseBody().then(
      (body) => ({ body, failed: null }),
      (failure: unknown) => ({
        body: null,
        failed: failure instanceof Error ? failure.message : 'The release did not read.',
      }),
    ),
    api.projectReleases().catch(() => []),
    work.list('current').catch(() => []),
    work.list('archived').catch(() => []),
  ]);
  return { ...body, projects, work: [...current, ...archived] };
};
