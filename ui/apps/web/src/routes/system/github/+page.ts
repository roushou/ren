import { Arrival } from '#lib/arrival.ts';
import { ManagementApi, type Pulls } from '#lib/api.ts';
import type { PageLoad } from './$types';

/** Each current project's GitHub watch; null where its pulls did not read. */
export interface GitHubData {
  watches: { project: string; watch: Pulls | null }[];
  failed: string | null;
}

/** The watches arrive like any page's read: within the grace period, or
 * pending while the page draws its outline. */
export const load: PageLoad = async ({ fetch, parent }) => {
  await parent();
  const api = new ManagementApi(fetch);
  const github = await Arrival.read('github', async (): Promise<GitHubData> => {
    try {
      const projects = (await api.projects()).filter((project) => project.archived_at === null);
      const watches = await Promise.all(
        projects.map(async (project) => ({
          project: project.name,
          watch: await api.projectPulls(project.name).catch(() => null),
        })),
      );
      return { watches, failed: null };
    } catch (failure) {
      return {
        watches: [],
        failed: failure instanceof Error ? failure.message : 'The projects did not read.',
      };
    }
  });
  return { github };
};
