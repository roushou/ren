import { browser } from '$app/env';
import { SessionsReader } from '#lib/loaders.ts';
import type { PageLoad } from './$types';

// In the browser one reader keeps each session's newest turn between refreshes.
const reader = browser ? new SessionsReader() : null;

export const load: PageLoad = async ({ fetch, parent }) => {
  await parent();
  return { sessions: await (reader ?? new SessionsReader()).read(fetch) };
};
