import test from 'node:test';
import assert from 'node:assert/strict';
import { HtmlCompression } from '../src/lib/server/compression.ts';

test('pages are gzipped for a browser that accepts it, and nothing else is', () => {
  const html = 'text/html; charset=utf-8';
  assert.equal(HtmlCompression.wanted('gzip, deflate, br', html, null), true);
  assert.equal(HtmlCompression.wanted('*', html, null), true);
  // Not accepted, refused by quality, or already encoded.
  assert.equal(HtmlCompression.wanted(null, html, null), false);
  assert.equal(HtmlCompression.wanted('br', html, null), false);
  assert.equal(HtmlCompression.wanted('gzip;q=0, br', html, null), false);
  assert.equal(HtmlCompression.wanted('*;q=0', html, null), false);
  assert.equal(HtmlCompression.wanted('gzip', html, 'br'), false);
  // Data responses keep their streamed pieces.
  assert.equal(HtmlCompression.wanted('gzip', 'application/json', null), false);
});

test('a gzipped page reads back as the page', async () => {
  const page = '<!doctype html><p>' + 'rows '.repeat(2000) + '</p>';
  const response = HtmlCompression.apply(
    new Request('http://ui.test/', { headers: { 'accept-encoding': 'gzip' } }),
    new Response(page, { headers: { 'content-type': 'text/html', 'content-length': '1' } }),
  );
  assert.equal(response.headers.get('content-encoding'), 'gzip');
  assert.equal(response.headers.get('content-length'), null);
  assert.match(response.headers.get('vary') ?? '', /accept-encoding/);
  const body = response.body!.pipeThrough(new DecompressionStream('gzip'));
  assert.equal(await new Response(body).text(), page);
});
