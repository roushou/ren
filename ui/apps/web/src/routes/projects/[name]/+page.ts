import { error } from '@sveltejs/kit';
import { ApiError, ManagementApi, type Project } from '#lib/api.ts';
import { RunsClient, Sessions, type DeskSession, type RecordedRun } from '#lib/runs.ts';
import { WorkApi, type WorkItem } from '#lib/work.ts';
import type { PageLoad } from './$types';

/** What a project's page reads: the project, its work, its sessions, and
 * their runs. A list that fails to read is left empty. */
export interface ProjectData {
  project: Project;
  work: WorkItem[];
  /** Its current sessions, each with the worktree it works in; none when
   * this UI only reads. */
  sessions: DeskSession[];
  /** Its sessions' runs, newest first. */
  runs: RecordedRun[];
}

export const load: PageLoad = async ({ fetch, params, parent }): Promise<ProjectData> => {
  const { runsControl } = await parent();
  const api = new ManagementApi(fetch);
  const work = new WorkApi(fetch);
  const runs = new RunsClient(fetch);
  const [project, current, archived, bodies, live, recorded] = await Promise.all([
    api.project(params.name).catch((failure: unknown) => {
      if (failure instanceof ApiError && failure.status === 404) {
        error(404, `No project is named ${params.name}.`);
      }
      throw failure;
    }),
    work.list('current', undefined, params.name).catch(() => []),
    work.list('archived', undefined, params.name).catch(() => []),
    runsControl ? runs.sessions('current', undefined, params.name).catch(() => []) : [],
    runsControl ? runs.live().catch(() => []) : [],
    runs.recorded(50, 'current', undefined, params.name).catch(() => []),
  ]);
  const sessions = bodies.flatMap((body) => Sessions.desk(body, live) ?? []);
  return { project, work: [...current, ...archived], sessions, runs: recorded };
};
