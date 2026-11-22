import test from 'node:test';
import assert from 'node:assert/strict';
import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join } from 'node:path';

const SRC = new URL('../src/', import.meta.url).pathname;
const LEVELS = ['toolbar', 'popover', 'slide', 'modal', 'toast'] as const;

/** Every Svelte and CSS file under `src`. */
function sources(dir: string): string[] {
  return readdirSync(dir).flatMap((name) => {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) return sources(path);
    return /\.(svelte|css)$/.test(name) ? [path] : [];
  });
}

test('each layer has its elevation in both themes, and a modal its scrim', () => {
  const style = readFileSync(join(SRC, 'lib/style.css'), 'utf8');
  for (const name of [...LEVELS.map((level) => `--elev-${level}`), '--scrim']) {
    assert.equal(
      style.match(new RegExp(`^\\s*${name}:`, 'gm'))?.length,
      2,
      `${name} is set once for light and once for dark`,
    );
  }
});

test('the base casts no shadow: every shadow is a layer’s, an inset rule, or a focus ring', () => {
  const offenders = sources(SRC).flatMap((path) =>
    [...readFileSync(path, 'utf8').matchAll(/box-shadow:\s*([^;]+);/g)]
      .map((match) => match[1]!.trim())
      .filter(
        (value) =>
          !/^var\(--elev-(toolbar|popover|slide|modal|toast)\)$/.test(value) &&
          !value.startsWith('inset') &&
          !value.startsWith('0 0 0 ') &&
          value !== 'none',
      )
      .map((value) => `${path.slice(SRC.length)}: ${value}`),
  );
  assert.deepEqual(offenders, [], 'give a raised element a layer from lib/layer/');
});

// Skeleton's widths are free-form (`42%`, `16em`) and predate this rule; on a
// server-rendered page the CSP drops them and the outline shows full width.
const INLINE_STYLE_EXEMPT = ['lib/Skeleton.svelte'];

test('no element carries inline style: the CSP refuses it, so widths and colours are classes', () => {
  const offenders = sources(SRC)
    .filter((path) => path.endsWith('.svelte'))
    .filter((path) => !INLINE_STYLE_EXEMPT.includes(path.slice(SRC.length)))
    .flatMap((path) =>
      /[\s<]style(=|:[a-z-])/.test(readFileSync(path, 'utf8')) ? [path.slice(SRC.length)] : [],
    );
  assert.deepEqual(offenders, []);
});
