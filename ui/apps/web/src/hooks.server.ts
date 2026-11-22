import type { Handle } from '@sveltejs/kit/hooks';
import { HtmlCompression } from '#lib/server/compression.ts';

export const handle: Handle = async ({ event, resolve }) => {
  const response = await resolve(event);
  // SSR HTML and Kit data responses contain private recorded traffic too.
  response.headers.set('cache-control', 'no-store');
  response.headers.set('x-content-type-options', 'nosniff');
  response.headers.set('referrer-policy', 'no-referrer');
  return HtmlCompression.apply(event.request, response);
};
