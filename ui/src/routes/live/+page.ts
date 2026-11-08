import { browser } from '$app/env';
import { LiveReader } from '#lib/loaders.ts';
import type { PageLoad } from './$types';

// The browser keeps one reader, so unchanged turns are not read again on
// every live refresh. The server reads afresh for each request.
const reader = browser ? new LiveReader() : null;

export const load: PageLoad = async ({ fetch }) => ({
  live: await (reader ?? new LiveReader()).read(fetch),
});
