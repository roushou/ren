import { gatewaySnapshot, projectsData } from '#lib/loaders.ts';
import type { LayoutLoad } from './$types';

export const load: LayoutLoad = async ({ fetch, data }) => {
  const [gateway, board] = await Promise.all([
    gatewaySnapshot(fetch),
    projectsData(fetch, data.runsControl),
  ]);
  return { ...data, gateway, board };
};
