import test from 'node:test';
import assert from 'node:assert/strict';
import { Commands } from '../src/lib/commands.ts';

const projects = [{ name: 'acme-api' }, { name: 'acme-web' }];

test('the palette offers every page, then each project’s own', () => {
  const all = Commands.all(projects);
  const hrefs = all.map((command) => command.href);
  assert.ok(hrefs.includes('/traffic') && hrefs.includes('/tools') && hrefs.includes('/'));
  assert.ok(hrefs.includes('/projects/acme-api/release'));
  assert.equal(new Set(hrefs).size, hrefs.length, 'no page is offered twice');
  assert.equal(all.find((c) => c.href === '/traffic')?.detail, 'Inspect');
  assert.equal(all.find((c) => c.href === '/projects/acme-web/sessions')?.detail, 'acme-web');
});

test('typing narrows by every word, label or place, and offers a search last', () => {
  const all = Commands.all(projects);
  assert.equal(Commands.match(all, '').length, all.length, 'nothing typed shows everything');
  const release = Commands.match(all, 'web rel');
  assert.deepEqual(
    release.map((c) => c.href),
    ['/projects/acme-web/release', '/search?q=web%20rel'],
  );
  const search = Commands.match(all, 'TRAFFIC');
  assert.equal(search[0]?.href, '/traffic', 'case does not matter');
  assert.equal(search.at(-1)?.kind, 'search');
});
