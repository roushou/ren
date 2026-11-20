import test from 'node:test';
import assert from 'node:assert/strict';
import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join } from 'node:path';
import { Marks, marks } from '../src/lib/marks.ts';

const SRC = new URL('../src/', import.meta.url).pathname;

/** Every Svelte and CSS file under `src`. */
function sources(dir: string): string[] {
  return readdirSync(dir).flatMap((name) => {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) return sources(path);
    return /\.(svelte|css)$/.test(name) ? [path] : [];
  });
}

test('each mark has its own word', () => {
  const words = marks.map((mark) => Marks.word(mark));
  assert.equal(new Set(words).size, marks.length);
  for (const word of words) assert.ok(word.length > 0);
});

test('each mark has a shape in the glyph and a colour in the theme', () => {
  const glyph = readFileSync(join(SRC, 'lib/Glyph.svelte'), 'utf8');
  const style = readFileSync(join(SRC, 'lib/style.css'), 'utf8');
  for (const mark of marks) {
    assert.match(glyph, new RegExp(`\\.${mark}\\b[^{]*\\{`), `Glyph.svelte draws no ${mark}`);
    assert.match(style, new RegExp(`\\.state-${mark}\\b[^{]*\\{`), `style.css colours no ${mark}`);
  }
});

test('states are drawn by the glyph, never by a dot of a view’s own', () => {
  const offenders = sources(SRC).flatMap((path) => {
    const text = readFileSync(path, 'utf8');
    return /status-dot|phase-dot|class="dot[\s"]/.test(text) ? [path.slice(SRC.length)] : [];
  });
  assert.deepEqual(offenders, [], 'draw a state with Glyph.svelte and a mark from marks.ts');
});
