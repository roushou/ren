import { browser } from '$app/env';
import { LiveReader, overviewSnapshot } from '#lib/loaders.ts';
import type { PageLoad } from './$types';

// In the browser one reader keeps the Live board's turn cache between refreshes.
const reader = browser ? new LiveReader() : null;

export const load: PageLoad = async ({ fetch }) => ({
  overview: await overviewSnapshot(fetch, reader ?? new LiveReader()),
});
