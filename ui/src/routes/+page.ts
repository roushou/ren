import { overviewSnapshot } from '#lib/loaders.ts';
import type { PageLoad } from './$types';
export const load: PageLoad = async ({ fetch }) => ({ overview: await overviewSnapshot(fetch) });
