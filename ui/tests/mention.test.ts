import test from 'node:test';
import assert from 'node:assert/strict';
import { Mentions } from '../src/lib/mention.ts';
import type { ClientCommand, FileMatch } from '../src/lib/runs.ts';

const command = (name: string, sigil: '/' | '$' = '/', description = ''): ClientCommand => ({
  name,
  description,
  hint: null,
  kind: 'command',
  sigil,
  invocation: `${sigil}${name}`,
});

test('a slash opens commands only at the start of the prompt; @ opens files anywhere', () => {
  assert.deepEqual(Mentions.at('/rev', 4), {
    kind: 'command',
    sigil: '/',
    query: 'rev',
    start: 0,
    end: 4,
  });
  assert.deepEqual(Mentions.at('  /', 3), {
    kind: 'command',
    sigil: '/',
    query: '',
    start: 2,
    end: 3,
  });
  assert.equal(Mentions.at('look at /etc/hosts', 18), null);
  assert.equal(Mentions.at('/src/lib', 8), null);
  assert.deepEqual(Mentions.at('fix @src/up and more', 11), {
    kind: 'file',
    query: 'src/up',
    start: 4,
    end: 11,
  });
  // The caret inside a word: the whole word is replaced, the typed part searched.
  assert.deepEqual(Mentions.at('see @runs.ts', 8), {
    kind: 'file',
    query: 'run',
    start: 4,
    end: 12,
  });
  assert.deepEqual(Mentions.at('use $rel', 8), {
    kind: 'command',
    sigil: '$',
    query: 'rel',
    start: 4,
    end: 8,
  });
  assert.equal(Mentions.at('costs $5,000', 12), null);
  assert.equal(Mentions.at('plain words', 5), null);
});

test('commands rank by name, then by description; $ offers only $ commands', () => {
  const listed = [
    command('code-review', '/', 'Review the diff'),
    command('review', '/', ''),
    command('release-notes', '$', 'Write notes'),
    command('compact', '/', 'Summarize to review later'),
  ];
  const slash = Mentions.at('/rev', 4);
  assert.equal(slash?.kind, 'command');
  if (slash?.kind !== 'command') return;
  assert.deepEqual(
    Mentions.commands(listed, slash).map((entry) => entry.name),
    ['review', 'code-review', 'compact'],
  );
  const dollar = Mentions.at('$', 1);
  if (dollar?.kind !== 'command') return assert.fail('a $ mention');
  assert.deepEqual(
    Mentions.commands(listed, dollar).map((entry) => entry.name),
    ['release-notes'],
  );
});

test('choosing writes the invocation or @path and puts the caret after it', () => {
  const at = Mentions.at('/rev please', 4)!;
  assert.deepEqual(Mentions.command('/rev please', at, command('review')), {
    text: '/review please',
    caret: 8,
  });
  const file: FileMatch = { path: 'src/lib/runs.ts', kind: 'file', positions: [] };
  const mention = Mentions.at('open @ru', 8)!;
  assert.deepEqual(Mentions.file('open @ru', mention, file), {
    text: 'open @src/lib/runs.ts ',
    caret: 22,
  });
  const folder: FileMatch = { path: 'src/lib', kind: 'directory', positions: [] };
  assert.deepEqual(Mentions.file('open @li', Mentions.at('open @li', 8)!, folder), {
    text: 'open @src/lib/',
    caret: 14,
  });
});

test('matched characters are marked in runs', () => {
  assert.deepEqual(Mentions.marked('src/runs.ts', [4, 5, 6]), [
    { text: 'src/', hit: false },
    { text: 'run', hit: true },
    { text: 's.ts', hit: false },
  ]);
});
