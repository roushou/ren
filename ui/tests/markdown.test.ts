import test from 'node:test';
import assert from 'node:assert/strict';
import { parseInline, parseMarkdown, type Block } from '../src/lib/markdown.ts';

const text = (t: string) => ({ kind: 'text', text: t });

test('the blocks models write most often', () => {
  const doc = parseMarkdown(
    [
      '## What Doppel is',
      '',
      'It runs as three processes:',
      '- **Rust**: the `doppel` CLI',
      '- *Swift*: one process per VM',
      '',
      '1. first',
      '2. second',
      '',
      '```rust',
      'fn main() {}',
      '```',
      '',
      '> quoted',
      '',
      '---',
    ].join('\n'),
  );
  assert.deepEqual(
    doc.map((b) => b.kind),
    ['heading', 'paragraph', 'list', 'list', 'code', 'quote', 'rule'],
  );
  assert.deepEqual(doc[0], { kind: 'heading', level: 2, children: [text('What Doppel is')] });
  const bullets = doc[2] as Extract<Block, { kind: 'list' }>;
  assert.equal(bullets.ordered, false);
  assert.equal(bullets.items.length, 2);
  assert.deepEqual(bullets.items[0], [
    {
      kind: 'paragraph',
      children: [
        { kind: 'strong', children: [text('Rust')] },
        text(': the '),
        { kind: 'code', text: 'doppel' },
        text(' CLI'),
      ],
    },
  ]);
  const ordered = doc[3] as Extract<Block, { kind: 'list' }>;
  assert.equal(ordered.ordered, true);
  assert.equal(ordered.start, 1);
  assert.deepEqual(doc[4], { kind: 'code', lang: 'rust', text: 'fn main() {}' });
});

test('nested lists and lazy continuation lines', () => {
  const [list] = parseMarkdown('- a\n  - b\n  - c\n- d\ncontinued');
  assert.equal(list.kind, 'list');
  if (list.kind !== 'list') return;
  assert.equal(list.items.length, 2);
  assert.deepEqual(
    list.items[0].map((b) => b.kind),
    ['paragraph', 'list'],
  );
  assert.deepEqual(list.items[1], [{ kind: 'paragraph', children: [text('d\ncontinued')] }]);
});

test('pipe tables', () => {
  const [table] = parseMarkdown('| Crate | Role |\n|---|:--|\n| `ren` | CLI |\n| x \\| y |');
  assert.equal(table.kind, 'table');
  if (table.kind !== 'table') return;
  assert.deepEqual(table.header, [[text('Crate')], [text('Role')]]);
  assert.deepEqual(table.rows[0], [[{ kind: 'code', text: 'ren' }], [text('CLI')]]);
  assert.deepEqual(table.rows[1], [[text('x | y')], []]);
});

test('HTML stays text, and links and images are kept as data', () => {
  assert.deepEqual(parseInline('<img src=x onerror=alert(1)>'), [
    text('<img src=x onerror=alert(1)>'),
  ]);
  assert.deepEqual(parseInline('see [docs](https://example.com/a_(b)) and ![logo](/x.png)'), [
    text('see '),
    { kind: 'link', children: [text('docs')], href: 'https://example.com/a_(b)' },
    text(' and '),
    { kind: 'image', alt: 'logo', href: '/x.png' },
  ]);
  const [paragraph] = parseMarkdown('<script>alert(1)</script>');
  assert.deepEqual(paragraph, { kind: 'paragraph', children: [text('<script>alert(1)</script>')] });
});

test('snake_case, unmatched markers, and escapes stay literal', () => {
  assert.deepEqual(parseInline('call snake_case_name or 2 * 3'), [
    text('call snake_case_name or 2 * 3'),
  ]);
  assert.deepEqual(parseInline('\\*not em\\* and `unclosed'), [text('*not em* and `unclosed')]);
  assert.deepEqual(parseInline('_em_ ~~gone~~'), [
    { kind: 'em', children: [text('em')] },
    text(' '),
    { kind: 'strike', children: [text('gone')] },
  ]);
});

test('an unclosed fence runs to the end, and deep nesting is bounded', () => {
  assert.deepEqual(parseMarkdown('```\nopen'), [{ kind: 'code', lang: '', text: 'open' }]);
  const deep = parseMarkdown('>'.repeat(50) + ' bottom');
  let depth = 0;
  let block: Block | undefined = deep[0];
  while (block?.kind === 'quote') {
    depth++;
    block = block.children[0];
  }
  assert.ok(depth <= 8, `${depth}`);
  assert.equal(block?.kind, 'paragraph');
});

test('tagged context folds under its name, and stray tags stay text', () => {
  const doc = parseMarkdown(
    '<system-reminder>Codebase notes\n\n# Rules\n</system-reminder>\n\nWhat does this do?',
  );
  assert.deepEqual(
    doc.map((b) => b.kind),
    ['tagged', 'paragraph'],
  );
  const tagged = doc[0] as Extract<Block, { kind: 'tagged' }>;
  assert.equal(tagged.name, 'system-reminder');
  assert.deepEqual(
    tagged.children.map((b) => b.kind),
    ['paragraph', 'heading'],
  );
  // A tag never closed is just text.
  assert.deepEqual(parseMarkdown('<note> open'), [
    { kind: 'paragraph', children: [text('<note> open')] },
  ]);
  // One-line tags close on their own line.
  const [inline] = parseMarkdown('<command-name>/review</command-name>');
  assert.equal(inline.kind, 'tagged');
});
