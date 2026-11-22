import type { Handle } from '@sveltejs/kit/hooks';
import { HtmlCompression } from '#lib/server/compression.ts';
import { Views } from '#lib/view.ts';

export const handle: Handle = async ({ event, resolve }) => {
  // Inspect's and Host's pages moved under Monitor and System: an old
  // address (a bookmark, a link in a brief) lands on the same page.
  const moved = Views.moved(event.url.pathname);
  if (moved) {
    return new Response(null, {
      status: 308,
      headers: { location: `${moved}${event.url.search}`, 'cache-control': 'no-store' },
    });
  }
  const response = await resolve(event);
  // SSR HTML and Kit data responses contain private recorded traffic too.
  response.headers.set('cache-control', 'no-store');
  response.headers.set('x-content-type-options', 'nosniff');
  response.headers.set('referrer-policy', 'no-referrer');
  return HtmlCompression.apply(event.request, response);
};
