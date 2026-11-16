import test from 'node:test';
import assert from 'node:assert/strict';
import { readdirSync, readFileSync } from 'node:fs';

// Markdown stands alone: its sources import only each other, Svelte, and
// the Markdown parser, never the app that hosts it.
const SOURCES = new URL('../src/', import.meta.url);
const IMPORT = /\bimport\s+(?:[^'"]*?\s+from\s+)?['"]([^'"]+)['"]/g;
const PARSER = ['mdast', 'mdast-util-from-markdown', 'mdast-util-gfm', 'micromark-extension-gfm'];

test('the sources import only each other, Svelte, and the parser', () => {
  const files = readdirSync(SOURCES).filter((name) => /\.(ts|svelte)$/.test(name));
  assert.ok(files.length > 0);
  for (const file of files) {
    const text = readFileSync(new URL(file, SOURCES), 'utf8');
    for (const [, specifier] of text.matchAll(IMPORT)) {
      const allowed =
        /^\.\/[^/]+$/.test(specifier!) ||
        specifier === 'svelte' ||
        specifier!.startsWith('svelte/') ||
        PARSER.includes(specifier!);
      assert.ok(allowed, `${file} imports ${specifier}`);
    }
  }
});
