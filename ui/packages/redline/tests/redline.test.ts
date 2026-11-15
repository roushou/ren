import test from 'node:test';
import assert from 'node:assert/strict';
import {
  Redline,
  type ElementLike,
  type Framing,
  type Mark,
  type PageContext,
} from '../src/redline.ts';

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
  path: '/settings?tab=keys',
  title: 'Acme - Settings',
  width: 1280,
  height: 800,
  theme: 'light',
};

const bare: Framing = { about: [], closing: null };

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

test("a mark is named by the element's text, else its selector's last step", () => {
  const target = { selector: '[aria-label="Run"] div.entry > span.chip', width: 1, height: 1 };
  assert.equal(Redline.name({ ...target, text: 'completed', source: null }), '“completed”');
  assert.equal(Redline.name({ ...target, text: '', source: null }), 'span.chip');
});

test('only noted marks are carried', () => {
  const marks = [mark(1, 'Fix it.'), mark(2, '  '), mark(3, 'And this.')];
  assert.deepEqual(
    Redline.noted(marks).map((entry) => entry.id),
    [1, 3],
  );
});

test("the brief lists each noted mark, between the host's lines and its closing", () => {
  const framing: Framing = {
    about: ['This page is `settings.svelte`.'],
    closing: 'Answer in place.',
  };
  assert.equal(
    Redline.brief(
      page,
      [mark(1, 'Move it to the end.'), mark(2, '  '), mark(3, 'Bigger.')],
      framing,
    ),
    [
      'Redline: Move it to the end. (and 1 more)',
      '',
      'Marked on `/settings?tab=keys` (Acme - Settings), 1280×800, light theme.',
      'This page is `settings.svelte`.',
      '',
      '1. Move it to the end.',
      '   - Element: `[aria-label="Run"] div.entry.tool > span.chip.success`, “completed”',
      '   - Source: `src/routes/sessions/+page.svelte:861`',
      '   - Size: 62×18',
      '2. Bigger.',
      '   - Element: `[aria-label="Run"] div.entry.tool > span.chip.success`, “completed”',
      '   - Size: 62×18',
      '',
      'Answer in place.',
    ].join('\n'),
  );
});

test('a brief with nothing from the host ends with its marks', () => {
  assert.equal(
    Redline.brief({ ...page, title: '' }, [mark(2, 'Fix.')], bare),
    [
      'Redline: Fix.',
      '',
      'Marked on `/settings?tab=keys`, 1280×800, light theme.',
      '',
      '1. Fix.',
      '   - Element: `[aria-label="Run"] div.entry.tool > span.chip.success`, “completed”',
      '   - Size: 62×18',
    ].join('\n'),
  );
});

test("a brief's code spans fence past backticks inside, and a note's lines stay in its item", () => {
  assert.equal(Redline.code('a.b'), '`a.b`');
  assert.equal(Redline.code('a `b` c'), '`` a `b` c ``');
  const brief = Redline.brief(page, [mark(1, 'First line\nsecond line')], bare);
  assert.ok(brief.includes('1. First line\n   second line\n   - Element:'));
});
