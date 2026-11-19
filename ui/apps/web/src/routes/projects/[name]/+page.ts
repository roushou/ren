import { error } from '@sveltejs/kit';
import { ApiError, ManagementApi, type Project } from '#lib/api.ts';
import { RunsClient, type RecordedRun } from '#lib/runs.ts';
import { WorkApi, type WorkItem, type Workspace } from '#lib/work.ts';
import type { PageLoad } from './$types';

/** What a project's page reads: the project, its work, its workspaces,
 * and its runs. A list that fails to read is left empty. */
export interface ProjectData {
  project: Project;
  work: WorkItem[];
  /** Its current sessions' worktrees. */
  workspaces: Workspace[];
  /** Its sessions' runs, newest first. */
  runs: RecordedRun[];
}

export const load: PageLoad = async ({ fetch, params, parent }): Promise<ProjectData> => {
  await parent();
  const api = new ManagementApi(fetch);
  const work = new WorkApi(fetch);
  const runs = new RunsClient(fetch);
  const [project, current, archived, workspaces, recorded] = await Promise.all([
    api.project(params.name).catch((failure: unknown) => {
      if (failure instanceof ApiError && failure.status === 404) {
        error(404, `No project is named ${params.name}.`);
      }
      throw failure;
    }),
    work.list('current', undefined, params.name).catch(() => []),
    work.list('archived', undefined, params.name).catch(() => []),
    work.workspaces(params.name).catch(() => []),
    runs.recorded(50, 'current', undefined, params.name).catch(() => []),
  ]);
  return { project, work: [...current, ...archived], workspaces, runs: recorded };
};
