import { ManagementApi, type ReleaseBody, type ReleaseView } from '#lib/api.ts';
import { WorkApi, type WorkItem } from '#lib/work.ts';
import type { PageLoad } from './$types';

/** A project built by its contract, with its release. */
export interface ProjectRelease {
  name: string;
  release: ReleaseView;
}

/** What the Release view reads; a read that fails leaves its part empty. */
export interface ReleaseData {
  /** Ren's project's name, whose release this is; null while none is Ren. */
  project: string | null;
  body: ReleaseBody | null;
  /** Why the release did not read, when it did not. */
  failed: string | null;
  projects: ProjectRelease[];
  /** Work items under way and done: what was shipped, and what waits. */
  work: WorkItem[];
}

export const load: PageLoad = async ({ fetch, parent }): Promise<ReleaseData> => {
  await parent();
  const api = new ManagementApi(fetch);
  const work = new WorkApi(fetch);
  const failed = (failure: unknown) =>
    failure instanceof Error ? failure.message : 'The release did not read.';
  const [listed, current, archived] = await Promise.all([
    api.projects().then(
      (projects) => ({ projects, failed: null }),
      (failure: unknown) => ({ projects: [], failed: failed(failure) }),
    ),
    work.list('current').catch(() => []),
    work.list('archived').catch(() => []),
  ]);
  const ren = ManagementApi.ren(listed.projects);
  const body = ren
    ? await api.projectRelease(ren.name).then(
        (body) => ({ body, failed: listed.failed }),
        (failure: unknown) => ({ body: null, failed: failed(failure) }),
      )
    : { body: null, failed: listed.failed };
  const projects = listed.projects.flatMap((project) =>
    project.factory !== 'ren' && project.release
      ? [{ name: project.name, release: project.release }]
      : [],
  );
  return { project: ren?.name ?? null, ...body, projects, work: [...current, ...archived] };
};
