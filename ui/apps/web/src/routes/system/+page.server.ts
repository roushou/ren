import { PackagesRead } from '#lib/server/packagesRead.ts';
import type { PageServerLoad } from './$types';

export const load: PageServerLoad = (event) => PackagesRead.load(event);
