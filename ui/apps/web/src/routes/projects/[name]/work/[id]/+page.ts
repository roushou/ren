import { error } from '@sveltejs/kit';
import { ApiError, ManagementApi, type Project } from '#lib/api.ts';
import { WorkApi, WorkError, WorkItemId, type WorkItem } from '#lib/work.ts';
import type { PageLoad } from './$types';

/** One work item of a project: what was marked, where it stands, and the
 * evidence it carries. */
export interface WorkItemData {
  project: Project;
  item: WorkItem;
}

export const load: PageLoad = async ({ fetch, params }): Promise<WorkItemData> => {
  const id = WorkItemId.parse(params.id);
  if (!id) error(404, 'No work item has that id.');
  const [project, item] = await Promise.all([
    new ManagementApi(fetch).project(params.name).catch((failure: unknown) => {
      if (failure instanceof ApiError && failure.status === 404) {
        error(404, `No project is named ${params.name}.`);
      }
      throw failure;
    }),
    new WorkApi(fetch).item(id).catch((failure: unknown) => {
      if (failure instanceof WorkError && failure.status === 404) {
        error(404, 'No work item has that id.');
      }
      throw failure;
    }),
  ]);
  return { project, item };
};
