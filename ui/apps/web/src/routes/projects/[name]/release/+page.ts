import { error } from '@sveltejs/kit';
import { ApiError, ManagementApi, type Project, type ReleaseBody } from '#lib/api.ts';
import { Work, WorkApi, type WorkItem } from '#lib/work.ts';
import type { PageLoad } from './$types';

/** What a project's Release page reads; a read that fails leaves its part
 * empty. */
export interface ReleaseData {
  project: Project;
  body: ReleaseBody | null;
  /** Why the release did not read, when it did not. */
  failed: string | null;
  /** The project's work items, under way and done: what was shipped, and
   * what waits. */
  work: WorkItem[];
}

export const load: PageLoad = async ({ fetch, params, parent }): Promise<ReleaseData> => {
  await parent();
  const api = new ManagementApi(fetch);
  const work = new WorkApi(fetch);
  const [project, release, current, archived] = await Promise.all([
    api.project(params.name).catch((failure: unknown) => {
      if (failure instanceof ApiError && failure.status === 404) {
        error(404, `No project is named ${params.name}.`);
      }
      throw failure;
    }),
    api.projectRelease(params.name).then(
      (body) => ({ body, failed: null }),
      (failure: unknown) => ({
        body: null,
        failed: failure instanceof Error ? failure.message : 'The release did not read.',
      }),
    ),
    work.list('current').catch(() => []),
    work.list('archived').catch(() => []),
  ]);
  return { project, ...release, work: Work.of([...current, ...archived], project) };
};
