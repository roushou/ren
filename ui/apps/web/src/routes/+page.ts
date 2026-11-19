import { ManagementApi, type Project } from '#lib/api.ts';
import { WorkApi, type WorkItem } from '#lib/work.ts';
import type { PageLoad } from './$types';

/** What the Projects view reads; work that fails to read leaves it empty. */
export interface ProjectsData {
  projects: Project[];
  /** Why the projects did not read, when they did not. */
  failed: string | null;
  /** Work items under way, across every project. */
  work: WorkItem[];
}

export const load: PageLoad = async ({ fetch, parent }): Promise<ProjectsData> => {
  await parent();
  const api = new ManagementApi(fetch);
  const [listed, work] = await Promise.all([
    api.projects().then(
      (projects) => ({ projects, failed: null }),
      (failure: unknown) => ({
        projects: [],
        failed: failure instanceof Error ? failure.message : 'The projects did not read.',
      }),
    ),
    new WorkApi(fetch).list('current').catch(() => []),
  ]);
  return { ...listed, work };
};
