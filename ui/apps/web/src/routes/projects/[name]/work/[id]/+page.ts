import { error } from '@sveltejs/kit';
import { Arrival } from '#lib/arrival.ts';
import { ApiError, ManagementApi, type Project, type Pull } from '#lib/api.ts';
import type { ProjectsData } from '#lib/loaders.ts';
import { ProjectBoard } from '#lib/projects.ts';
import { PullWatch } from '#lib/release.ts';
import { WorkApi, WorkError, WorkItemId, type WorkItem } from '#lib/work.ts';
import type { PageLoad } from './$types';

/** One work item of a project: what was marked, where it stands, and the
 * evidence it carries. */
export interface WorkItemData {
  project: Project;
  item: WorkItem;
  /** The project's open pull requests that name it. */
  pulls: Pull[];
  /** The repository they are on; null when none is watched. */
  repository: string | null;
}

/** The item arrives like any page's read: within the grace period, or
 * pending while the page draws its outline. */
export const load: PageLoad = async ({ fetch, params, parent }) => {
  const id = WorkItemId.parse(params.id);
  if (!id) error(404, 'No work item has that id.');
  const { board } = await parent();
  if (ProjectBoard.unknown(Arrival.value<ProjectsData>(board), params.name)) {
    error(404, `No project is named ${params.name}.`);
  }
  const management = new ManagementApi(fetch);
  const work = await Arrival.read(`work:${id}`, async (): Promise<WorkItemData> => {
    const [project, item, watch] = await Promise.all([
      management.project(params.name).catch((failure: unknown) => {
        if (failure instanceof ApiError && failure.status === 404) {
          throw new Error(`No project is named ${params.name}.`);
        }
        throw failure;
      }),
      new WorkApi(fetch).item(id).catch((failure: unknown) => {
        if (failure instanceof WorkError && failure.status === 404) {
          throw new Error('No work item has that id.');
        }
        throw failure;
      }),
      management.projectPulls(params.name).catch(() => null),
    ]);
    return {
      project,
      item,
      pulls: PullWatch.pulls(watch).filter((pull) => pull.work_items.includes(item.id)),
      repository: PullWatch.repository(watch),
    };
  });
  return { work };
};
