import { gatewaySnapshot } from '#lib/loaders.ts';
import type { LayoutLoad } from './$types';

export const load: LayoutLoad = async ({ fetch, data }) => ({
  ...data,
  gateway: await gatewaySnapshot(fetch),
});
