import type { ProjectsData } from '#lib/loaders.ts';
import type { PageLoad } from './$types';

/** The Projects view reads what the layout read for Pending: the same board. */
export const load: PageLoad = async ({ parent }): Promise<ProjectsData> => (await parent()).board;
