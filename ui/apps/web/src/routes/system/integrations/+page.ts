import { ManagementApi, type Pulls } from '#lib/api.ts';
import type { PageLoad } from './$types';

/** Each current project's GitHub watch; null where its pulls did not read. */
export interface IntegrationsData {
  watches: { project: string; watch: Pulls | null }[];
  failed: string | null;
}

export const load: PageLoad = async ({ fetch, parent }): Promise<IntegrationsData> => {
  await parent();
  const api = new ManagementApi(fetch);
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
};
