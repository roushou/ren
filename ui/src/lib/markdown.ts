/**
 * A small Markdown reader for captured model text. It produces a typed tree
 * that components render as elements, never as HTML: raw HTML stays text,
 * links are shown with their address but are not clickable, and images are
 * named, never loaded. Captures are untrusted, so nothing in them can fetch
 * or run anything.
 *
 * Supported: ATX headings, paragraphs, fenced code, block quotes, bullet
 * and numbered lists (nested by indentation), thematic breaks, pipe tables,
 * and inline code, strong, emphasis, strikethrough, links, and images.
 */

export type Inline =
  | { kind: 'text'; text: string }
  | { kind: 'code'; text: string }
  | { kind: 'strong'; children: Inline[] }
  | { kind: 'em'; children: Inline[] }
  | { kind: 'strike'; children: Inline[] }
  | { kind: 'link'; children: Inline[]; href: string }
  | { kind: 'image'; alt: string; href: string };

export type Block =
  | { kind: 'heading'; level: 1 | 2 | 3 | 4 | 5 | 6; children: Inline[] }
  | { kind: 'paragraph'; children: Inline[] }
  | { kind: 'code'; lang: string; text: string }
  | { kind: 'quote'; children: Block[] }
  | { kind: 'list'; ordered: boolean; start: number; items: Block[][] }
  | { kind: 'rule' }
  | { kind: 'table'; header: Inline[][]; rows: Inline[][][] }
  /** `<name>…</name>` spanning lines, as agents wrap injected context
   * (`<system-reminder>`). Shown folded, under its name. */
  | { kind: 'tagged'; name: string; children: Block[] };

/** Deeper nesting than this is read as plain text, which bounds recursion. */
const MAX_DEPTH = 8;

const fence = /^ {0,3}(`{3,}|~{3,})\s*([^`\s]*)[^`]*$/;
const heading = /^ {0,3}(#{1,6})(?:[ \t]+(.*?))?(?:[ \t]+#+)?[ \t]*$/;
const rule = /^ {0,3}([-*_])(?:[ \t]*\1){2,}[ \t]*$/;
const bullet = /^( *)([-*+])[ \t]+(.*)$/;
const numbered = /^( *)(\d{1,9})[.)][ \t]+(.*)$/;
const quote = /^ {0,3}>[ \t]?(.*)$/;
const tagOpen = /^ {0,3}<([a-z][a-z0-9]*[-_][a-z0-9_-]{0,62})>(.*)$/;
const tableRule = /^ *\|?(?: *:?-+:? *\|)+ *:?-+:? *\|? *$|^ *\|? *:?-+:? *\| *$/;

function blank(line: string) {
  return line.trim() === '';
}

function listMarker(line: string) {
  const m = bullet.exec(line) ?? numbered.exec(line);
  if (!m) return null;
  const ordered = /\d/.test(m[2]);
  return {
    indent: m[1].length,
    ordered,
    start: ordered ? Number(m[2]) : 1,
    // The content column: continuation lines indented this far belong to the item.
    column: line.length - m[3].length,
    text: m[3],
  };
}

function cells(line: string): string[] {
  let row = line.trim();
  if (row.startsWith('|')) row = row.slice(1);
  if (row.endsWith('|') && !row.endsWith('\\|')) row = row.slice(0, -1);
  const out: string[] = [];
  let cell = '';
  for (let i = 0; i < row.length; i++) {
    if (row[i] === '\\' && row[i + 1] === '|') {
      cell += '|';
      i++;
    } else if (row[i] === '|') {
      out.push(cell.trim());
      cell = '';
    } else cell += row[i];
  }
  out.push(cell.trim());
  return out;
}

/** Whether `line` starts a block other than a paragraph continuation. */
function interrupts(line: string) {
  return (
    fence.test(line) ||
    heading.test(line) ||
    rule.test(line) ||
    quote.test(line) ||
    listMarker(line) !== null
  );
}

export function parseMarkdown(text: string, depth = 0): Block[] {
  const lines = text.replace(/\r\n?/g, '\n').split('\n');
  const blocks: Block[] = [];
  let i = 0;
  while (i < lines.length) {
    const line = lines[i];
    if (blank(line)) {
      i++;
      continue;
    }
    if (depth >= MAX_DEPTH) {
      blocks.push({
        kind: 'paragraph',
        children: [{ kind: 'text', text: lines.slice(i).join('\n') }],
      });
      break;
    }
    const tag = tagOpen.exec(line);
    if (tag) {
      const close = `</${tag[1]}>`;
      let end = i;
      while (end < lines.length && !lines[end].includes(close)) end++;
      if (end < lines.length) {
        const inner = [tag[2], ...lines.slice(i + 1, end + 1)].join('\n');
        const body = inner.slice(0, inner.lastIndexOf(close));
        const after = lines[end].slice(lines[end].indexOf(close) + close.length);
        blocks.push({ kind: 'tagged', name: tag[1], children: parseMarkdown(body, depth + 1) });
        if (after.trim()) lines[end] = after;
        else end++;
        i = end;
        continue;
      }
    }
    const open = fence.exec(line);
    if (open) {
      const marker = open[1];
      const body: string[] = [];
      i++;
      while (i < lines.length && !lines[i].trimStart().startsWith(marker)) body.push(lines[i++]);
      i++; // the closing fence, or past the end
      blocks.push({ kind: 'code', lang: open[2], text: body.join('\n') });
      continue;
    }
    const h = heading.exec(line);
    if (h) {
      blocks.push({
        kind: 'heading',
        level: h[1].length as 1 | 2 | 3 | 4 | 5 | 6,
        children: parseInline(h[2] ?? '', depth),
      });
      i++;
      continue;
    }
    if (rule.test(line)) {
      blocks.push({ kind: 'rule' });
      i++;
      continue;
    }
    if (quote.test(line)) {
      const body: string[] = [];
      while (i < lines.length && !blank(lines[i])) {
        const q = quote.exec(lines[i]);
        body.push(q ? q[1] : lines[i]);
        i++;
      }
      blocks.push({ kind: 'quote', children: parseMarkdown(body.join('\n'), depth + 1) });
      continue;
    }
    const marker = listMarker(line);
    if (marker) {
      const items: Block[][] = [];
      while (i < lines.length) {
        const item = listMarker(lines[i]);
        if (!item || item.indent !== marker.indent || item.ordered !== marker.ordered) break;
        const body = [item.text];
        i++;
        let gap = false;
        while (i < lines.length) {
          const next = lines[i];
          if (blank(next)) {
            gap = true;
            body.push('');
            i++;
            continue;
          }
          const indent = next.length - next.trimStart().length;
          const sibling = listMarker(next);
          if (indent >= item.column || (indent > marker.indent && sibling)) {
            body.push(next.slice(Math.min(indent, item.column)));
            gap = false;
            i++;
          } else if (!gap && !sibling && !interrupts(next)) {
            // A lazy continuation of the item's paragraph.
            body.push(next.trim());
            i++;
          } else break;
        }
        while (body.length && body.at(-1) === '') body.pop();
        items.push(parseMarkdown(body.join('\n'), depth + 1));
      }
      blocks.push({ kind: 'list', ordered: marker.ordered, start: marker.start, items });
      continue;
    }
    if (line.includes('|') && i + 1 < lines.length && tableRule.test(lines[i + 1])) {
      const header = cells(line);
      i += 2;
      const rows: Inline[][][] = [];
      while (i < lines.length && !blank(lines[i]) && lines[i].includes('|')) {
        const row = cells(lines[i++]);
        rows.push(header.map((_, c) => parseInline(row[c] ?? '', depth)));
      }
      blocks.push({ kind: 'table', header: header.map((c) => parseInline(c, depth)), rows });
      continue;
    }
    const body = [line];
    i++;
    while (i < lines.length && !blank(lines[i]) && !interrupts(lines[i])) body.push(lines[i++]);
    blocks.push({ kind: 'paragraph', children: parseInline(body.join('\n'), depth) });
  }
  return blocks;
}

/** Inline spans; anything unmatched stays literal text. */
export function parseInline(text: string, depth = 0): Inline[] {
  const out: Inline[] = [];
  let plain = '';
  const flush = () => {
    if (plain) out.push({ kind: 'text', text: plain });
    plain = '';
  };
  const nested = (inner: string) =>
    depth + 1 >= MAX_DEPTH
      ? [{ kind: 'text' as const, text: inner }]
      : parseInline(inner, depth + 1);
  let i = 0;
  while (i < text.length) {
    const rest = text.slice(i);
    const ch = text[i];
    if (ch === '\\' && i + 1 < text.length && /[\\`*_{}[\]()#+\-.!|~<>]/.test(text[i + 1])) {
      plain += text[i + 1];
      i += 2;
      continue;
    }
    if (ch === '`') {
      const run = /^`+/.exec(rest)![0];
      const close = text.indexOf(run, i + run.length);
      if (close !== -1) {
        flush();
        const code = text.slice(i + run.length, close);
        out.push({
          kind: 'code',
          text:
            code.length > 2 && code.startsWith(' ') && code.endsWith(' ')
              ? code.slice(1, -1)
              : code,
        });
        i = close + run.length;
        continue;
      }
      plain += run;
      i += run.length;
      continue;
    }
    if (ch === '!' || ch === '[') {
      const m = /^(!?)\[([^\]\n]*)\]\(([^()\s]*(?:\([^()\s]*\))?[^()\s]*)(?:\s+"[^"]*")?\)/.exec(
        rest,
      );
      if (m) {
        flush();
        if (m[1]) out.push({ kind: 'image', alt: m[2], href: m[3] });
        else out.push({ kind: 'link', children: nested(m[2]), href: m[3] });
        i += m[0].length;
        continue;
      }
    }
    const pair = /^(\*\*|__)(?=\S)([\s\S]*?\S)\1/.exec(rest) ?? null;
    if (pair) {
      flush();
      out.push({ kind: 'strong', children: nested(pair[2]) });
      i += pair[0].length;
      continue;
    }
    const strike = /^~~(?=\S)([\s\S]*?\S)~~/.exec(rest);
    if (strike) {
      flush();
      out.push({ kind: 'strike', children: nested(strike[1]) });
      i += strike[0].length;
      continue;
    }
    // `_` emphasis only at word boundaries, so snake_case stays text.
    const before = i === 0 ? ' ' : text[i - 1];
    const em =
      ch === '*'
        ? /^\*(?=[^\s*])([\s\S]*?[^\s*])\*(?!\*)/.exec(rest)
        : ch === '_' && !/\w/.test(before)
          ? /^_(?=\S)([\s\S]*?\S)_(?!\w)/.exec(rest)
          : null;
    if (em) {
      flush();
      out.push({ kind: 'em', children: nested(em[1]) });
      i += em[0].length;
      continue;
    }
    plain += ch;
    i++;
  }
  flush();
  return out;
}
