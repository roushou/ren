import { ManagementApi, type Project } from '#lib/api.ts';
import { RunsClient, type RunSnapshot, type SessionBody } from '#lib/runs.ts';
import { WorkApi, type WorkItem } from '#lib/work.ts';
import type { PageLoad } from './$types';

/** What the Projects view reads; a list that fails to read is left empty. */
export interface ProjectsData {
  projects: Project[];
  /** Why the projects did not read, when they did not. */
  failed: string | null;
  /** Work items under way, across every project. */
  work: WorkItem[];
  /** The runs the daemon holds, for the approvals they wait on; none when
   * this UI only reads. */
  live: RunSnapshot[];
  /** The current sessions, for the project each run works for. */
  sessions: SessionBody[];
}

export const load: PageLoad = async ({ fetch, parent }): Promise<ProjectsData> => {
  const { runsControl } = await parent();
  const api = new ManagementApi(fetch);
  const runs = new RunsClient(fetch);
  const [listed, work, live, sessions] = await Promise.all([
    api.projects().then(
      (projects) => ({ projects, failed: null }),
      (failure: unknown) => ({
        projects: [],
        failed: failure instanceof Error ? failure.message : 'The projects did not read.',
      }),
    ),
    new WorkApi(fetch).list('current').catch(() => []),
    runsControl ? runs.live().catch(() => []) : Promise.resolve([]),
    runsControl ? runs.sessions('current').catch(() => []) : Promise.resolve([]),
  ]);
  return { ...listed, work, live, sessions };
};
