import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { runInNewContext } from 'node:vm';

async function themeInitializer() {
  const html = await readFile(new URL('../src/app.html', import.meta.url), 'utf8');
  const match = html.match(/<script id="theme-init"[^>]*>([\s\S]*?)<\/script>/);
  assert.ok(match, 'The page template must contain the early theme initializer');
  return match[1];
}

test('early theme initialization respects saved preference, system appearance, and unavailable storage', async () => {
  const script = await themeInitializer();
  for (const darkSystem of [false, true]) {
    for (const saved of ['light', 'dark', 'system', 'invalid', null]) {
      for (const storageBlocked of [false, true]) {
        const dataset: Record<string, string> = {};
        runInNewContext(script, {
          document: { documentElement: { dataset } },
          localStorage: {
            getItem(key: string) {
              assert.equal(key, 'tgv.theme');
              if (storageBlocked) throw new Error('Storage unavailable');
              return saved;
            },
          },
          matchMedia: () => ({ matches: darkSystem }),
        });
        const preference =
          !storageBlocked && (saved === 'light' || saved === 'dark') ? saved : 'system';
        assert.equal(dataset.themePreference, preference);
        assert.equal(
          dataset.theme,
          preference === 'system' ? (darkSystem ? 'dark' : 'light') : preference,
        );
      }
    }
  }
});
