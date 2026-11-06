import test from 'node:test';
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { dirname, join } from 'node:path';
import { announcerStyleHash, directives } from '../src/csp.ts';

test('the CSP allows only the route announcer style, by its current hash', () => {
  const kit = dirname(createRequire(import.meta.url).resolve('@sveltejs/kit/package.json'));
  const root = readFileSync(join(kit, 'src/runtime/components/root.svelte'), 'utf8');
  const style = /id="svelte-announcer"[\s\S]*?style="([^"]*)"/.exec(root)?.[1];
  assert.ok(style, 'SvelteKit no longer renders an announcer style attribute');
  const hash = `sha256-${createHash('sha256').update(style).digest('base64')}`;
  assert.equal(hash, announcerStyleHash);
  assert.deepEqual(directives['style-src-attr'], ['unsafe-hashes', announcerStyleHash]);
  // No blanket inline allowance anywhere.
  for (const sources of Object.values(directives))
    assert.ok(!(sources as string[] | undefined)?.includes('unsafe-inline'));
});
