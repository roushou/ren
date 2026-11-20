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

test('actions come first: a session in each project, a canary to promote; work items follow', () => {
  const all = Commands.all(projects, {
    promotable: [{ project: 'acme-web', commit: 'a3f9c21e04b2ffff' }],
    work: [
      { title: 'Billing totals overflow', project: 'acme-web', href: '/projects/acme-web/work/w1' },
      { title: 'No home yet', project: null, href: null },
    ],
  });
  assert.deepEqual(
    all.slice(0, 3).map((c) => [c.kind, c.label, c.detail]),
    [
      ['act', 'Promote a3f9c21e04b2 to production', 'acme-web'],
      ['act', 'New session', 'acme-api'],
      ['act', 'New session', 'acme-web'],
    ],
  );
  assert.equal(all[0]?.href, '/projects/acme-web/release?promote=a3f9c21e04b2ffff');
  assert.equal(all.at(-1)?.label, 'Billing totals overflow');
  assert.ok(!all.some((c) => c.label === 'No home yet'), 'an item with no page is not offered');
  assert.deepEqual(
    Commands.match(all, 'prom').map((c) => c.kind),
    ['act', 'search'],
  );
});
