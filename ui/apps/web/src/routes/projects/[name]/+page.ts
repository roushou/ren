import { error } from '@sveltejs/kit';
import { ApiError, ManagementApi, type Project } from '#lib/api.ts';
import { PreviewsApi, type PreviewList } from '#lib/previews.ts';
import { PullsApi, type Pulls } from '#lib/pulls.ts';
import { RunsClient, type RunSnapshot, type SessionBody } from '#lib/runs.ts';
import { Work, WorkApi, type WorkItem } from '#lib/work.ts';
import type { PageLoad } from './$types';

/** What a project's Overview reads: the project, its current work, its
 * pull requests on GitHub, and, when this UI drives sessions, its current
 * sessions and the runs the daemon holds (for what is running and what
 * waits). A list that fails to read is left empty; pull requests that
 * fail to read are null. */
export interface ProjectData {
  project: Project;
  /** Its current work items; Ren's holds those from before projects. */
  work: WorkItem[];
  sessions: SessionBody[];
  live: RunSnapshot[];
  pulls: Pulls | null;
  /** The factory's previews; none for a project other than Ren itself. */
  previews: PreviewList | null;
}

export const load: PageLoad = async ({ fetch, params, parent }): Promise<ProjectData> => {
  const { runsControl } = await parent();
  const runs = new RunsClient(fetch);
  const [project, work, sessions, live, pulls, previews] = await Promise.all([
    new ManagementApi(fetch).project(params.name).catch((failure: unknown) => {
      if (failure instanceof ApiError && failure.status === 404) {
        error(404, `No project is named ${params.name}.`);
      }
      throw failure;
    }),
    new WorkApi(fetch).list('current').catch(() => []),
    runsControl ? runs.sessions('current', undefined, params.name).catch(() => []) : [],
    runsControl ? runs.live().catch(() => []) : [],
    new PullsApi(fetch).of(params.name).catch(() => null),
    new PreviewsApi(fetch).list(params.name).catch(() => null),
  ]);
  return { project, work: Work.of(work, project), sessions, live, pulls, previews };
};
