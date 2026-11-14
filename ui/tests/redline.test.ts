import test from 'node:test';
import assert from 'node:assert/strict';
import { Redline, type ElementLike, type Mark, type PageContext } from '../src/lib/redline.ts';

function element(
  tagName: string,
  classes: string[] = [],
  parent: ElementLike | null = null,
  attributes: Record<string, string> = {},
  id = '',
): ElementLike {
  return {
    tagName: tagName.toUpperCase(),
    id,
    classList: classes,
    parentElement: parent,
    getAttribute: (name) => attributes[name] ?? null,
  };
}

const page: PageContext = {
  path: '/sessions?session=s1',
  route: '/sessions',
  title: 'Ren - Sessions',
  width: 1400,
  height: 820,
  theme: 'dark',
  revision: { kind: 'commit', commit: '97aa93b'.padEnd(40, '0'), tree: 'clean' },
};

function mark(id: number, note: string): Mark {
  return {
    id,
    note,
    target: {
      selector: '[aria-label="Run"] div.entry.tool > span.chip.success',
      text: 'completed',
      source: id === 1 ? 'src/routes/sessions/+page.svelte:861' : null,
      width: 62.4,
      height: 18,
    },
  };
}

test('a selector names the labelled region, then the element and two ancestors', () => {
  const region = element('section', ['run-main'], null, { 'aria-label': 'Run' });
  const convo = element('div', ['conversation', 'svelte-1x2y3z'], region);
  const entry = element('div', ['entry', 'tool'], convo);
  const chip = element('span', ['chip', 'success', 'svelte-1x2y3z'], entry);
  assert.equal(
    Redline.selector(chip),
    '[aria-label="Run"] div.conversation > div.entry.tool > span.chip.success',
  );
  // An id stands for the element; with no labelled region, the steps alone.
  const lone = element('button', ['x'], element('main', [], null, {}, 'top'));
  assert.equal(Redline.selector(lone), 'main#top > button.x');
});

test('text is one line, cut short', () => {
  assert.equal(Redline.text('  run\n  the   tests '), 'run the tests');
  const long = Redline.text('a'.repeat(200));
  assert.equal(long.length, Redline.TEXT);
  assert.ok(long.endsWith('…'));
});

test('the branch is named from the first note', () => {
  assert.equal(
    Redline.branch([mark(1, 'The status chip floats mid-row; move it!')]),
    'redline/the-status-chip-floats-mid',
  );
  assert.equal(Redline.branch([]), 'redline/change');
});

test('the brief lists each noted mark with what it points at', () => {
  const brief = Redline.brief(page, [mark(1, 'Move it to the end.'), mark(2, '  ')]);
  assert.equal(
    brief,
    [
      'Redline: Move it to the end.',
      '',
      'Marked on /sessions?session=s1 (Ren - Sessions), 1400×820, dark theme.',
      "In Ren's UI this page is ui/src/routes/sessions/+page.svelte.",
      'This page was served by a build of commit 97aa93b00000; the checkout may have moved on since.',
      '',
      '1. Move it to the end.',
      '   Element: [aria-label="Run"] div.entry.tool > span.chip.success "completed"',
      '   Source: src/routes/sessions/+page.svelte:861',
      '   Size: 62×18',
      '',
      "Answer what the notes ask without changing anything. If a note asks for a change, make it on a new branch, redline/move-it-to-the-end, in a git worktree of this repository, not in the checkout itself, and run the project's checks. Don't install or restart anything: I'll review the branch.",
    ].join('\n'),
  );
  assert.equal(Redline.routeFile('/'), 'ui/src/routes/+page.svelte');
});

test('the build is named when the daemon says what it was built from', () => {
  assert.equal(
    Redline.built({ kind: 'commit', commit: 'a'.repeat(40), tree: 'modified' }),
    'This page was served by a build of commit aaaaaaaaaaaa with uncommitted changes; the checkout may have moved on since.',
  );
  assert.equal(Redline.built({ kind: 'unknown' }), null);
  assert.equal(Redline.built(null), null);
});

test('the source checkout is an absolute path, or absent', () => {
  assert.equal(Redline.checkout(undefined), undefined);
  assert.equal(Redline.checkout(''), undefined);
  assert.equal(Redline.checkout('/home/you/ren/'), '/home/you/ren');
  assert.equal(Redline.checkout('/'), '/');
  assert.throws(() => Redline.checkout('ren'), /absolute/);
});
