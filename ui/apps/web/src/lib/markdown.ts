/**
 * Markdown as the UI renders it: CommonMark with GitHub's extensions
 * (tables, strikethrough, task lists, autolinks, footnotes), read by the
 * standard parser (`mdast-util-from-markdown`) and turned here into a closed,
 * typed tree that components render as elements, never as HTML.
 *
 * The text is untrusted (captured model output, prompts), so this is where
 * it is made inert: raw HTML becomes text, a link keeps its address but is
 * not clickable, an image is named but never loaded, and nesting deeper than
 * `DEPTH` is kept as its source text, which bounds rendering recursion.
 * Agents wrap injected context in tags on their own lines
 * (`<system-reminder>`); such a region is read as a `tagged` block, shown
 * folded under its name.
 */
import type { AlignType, Definition, Nodes, PhrasingContent, RootContent } from 'mdast';
import { fromMarkdown } from 'mdast-util-from-markdown';
import { gfmFromMarkdown } from 'mdast-util-gfm';
import { gfm } from 'micromark-extension-gfm';

export type Inline =
  | { kind: 'text'; text: string }
  | { kind: 'code'; text: string }
  | { kind: 'strong'; children: Inline[] }
  | { kind: 'em'; children: Inline[] }
  | { kind: 'strike'; children: Inline[] }
  | { kind: 'link'; children: Inline[]; href: string }
  | { kind: 'image'; alt: string; href: string }
  /** A hard line break. */
  | { kind: 'break' }
  /** A footnote's mark in the text. */
  | { kind: 'footnote'; label: string };

/** A table column's alignment. */
export type Align = 'left' | 'right' | 'center' | null;

/** One list item; `checked` is set for a task list's items. */
export interface ListItem {
  checked: boolean | null;
  children: Block[];
}

export type Block =
  | { kind: 'heading'; level: 1 | 2 | 3 | 4 | 5 | 6; children: Inline[] }
  | { kind: 'paragraph'; children: Inline[] }
  | { kind: 'code'; lang: string; text: string }
  | { kind: 'quote'; children: Block[] }
  | { kind: 'list'; ordered: boolean; start: number; items: ListItem[] }
  | { kind: 'rule' }
  | { kind: 'table'; align: Align[]; header: Inline[][]; rows: Inline[][][] }
  /** A footnote's text, under its label. */
  | { kind: 'footnote'; label: string; children: Block[] }
  /** `<name>…</name>` spanning lines, as agents wrap injected context
   * (`<system-reminder>`). Shown folded, under its name. */
  | { kind: 'tagged'; name: string; children: Block[] };

/** A stretch of the text: Markdown, or a tagged region. */
type Segment = { kind: 'markdown'; text: string } | { kind: 'tagged'; name: string; text: string };

/** An opening tag at a line's start, named as agents name them (a dash or
 * underscore inside, so HTML's own tags stay HTML, which is text). */
const TAG_OPEN = /^ {0,3}<([a-z][a-z0-9]*[-_][a-z0-9_-]{0,62})>(.*)$/;
const FENCE = /^ {0,3}(`{3,}|~{3,})/;

export class MarkdownTree {
  /** Nesting deeper than this is kept as text. */
  static readonly DEPTH = 8;

  /** `text` as blocks. */
  static parse(text: string, depth = 0): Block[] {
    return MarkdownTree.#segments(text).flatMap((segment): Block[] => {
      switch (segment.kind) {
        case 'markdown':
          return new MarkdownTree(segment.text).#blocks(
            fromMarkdown(segment.text, {
              extensions: [gfm()],
              mdastExtensions: [gfmFromMarkdown()],
            }).children,
            depth,
          );
        case 'tagged':
          return [
            {
              kind: 'tagged',
              name: segment.name,
              children:
                depth + 1 >= MarkdownTree.DEPTH
                  ? MarkdownTree.#plain(segment.text)
                  : MarkdownTree.parse(segment.text, depth + 1),
            },
          ];
        default: {
          const never: never = segment;
          return never;
        }
      }
    });
  }

  /** One paragraph's inline spans, for text known to be a single line. */
  static inline(text: string): Inline[] {
    const [first] = MarkdownTree.parse(text);
    return first?.kind === 'paragraph' ? first.children : [];
  }

  /**
   * The text cut into Markdown and tagged regions: a tag opening a line,
   * outside a code fence, starts a region that runs to its closing tag; an
   * unclosed tag stays text.
   */
  static #segments(text: string): Segment[] {
    const lines = text.split('\n');
    const out: Segment[] = [];
    let markdown: string[] = [];
    let fence: string | null = null;
    const flush = () => {
      if (markdown.length) out.push({ kind: 'markdown', text: markdown.join('\n') });
      markdown = [];
    };
    for (let i = 0; i < lines.length; i++) {
      const line = lines[i] ?? '';
      const marker = FENCE.exec(line)?.[1] ?? null;
      if (fence !== null) {
        if (marker && marker[0] === fence[0] && marker.length >= fence.length) fence = null;
        markdown.push(line);
        continue;
      }
      if (marker) {
        fence = marker;
        markdown.push(line);
        continue;
      }
      const tag = TAG_OPEN.exec(line);
      const close = tag ? `</${tag[1]}>` : null;
      const end = close ? lines.findIndex((later, at) => at >= i && later.includes(close)) : -1;
      if (!tag || !close || end === -1) {
        markdown.push(line);
        continue;
      }
      // The region's text: the opening line's rest, up to the first closing
      // tag on the line that holds it; what follows that tag reads on.
      const rest = tag[2] ?? '';
      const closing = end === i ? rest : (lines[end] ?? '');
      const at = closing.indexOf(close);
      const body =
        end === i
          ? rest.slice(0, at)
          : [rest, ...lines.slice(i + 1, end), closing.slice(0, at)].join('\n');
      const after = closing.slice(at + close.length);
      flush();
      out.push({ kind: 'tagged', name: tag[1] ?? '', text: body });
      if (after.trim()) lines[end] = after;
      i = after.trim() ? end - 1 : end;
    }
    flush();
    return out;
  }

  /** Text kept as it is, as one paragraph. */
  static #plain(text: string): Block[] {
    return text.trim() ? [{ kind: 'paragraph', children: [{ kind: 'text', text }] }] : [];
  }

  readonly #source: string;
  /** Link and image definitions, by identifier, for references. */
  readonly #definitions = new Map<string, Definition>();

  private constructor(source: string) {
    this.#source = source;
  }

  /** A node's text in the source, for what is kept as text. */
  #text(node: Nodes): string {
    const start = node.position?.start.offset;
    const end = node.position?.end.offset;
    return start === undefined || end === undefined ? '' : this.#source.slice(start, end);
  }

  #blocks(nodes: readonly RootContent[], depth: number): Block[] {
    for (const node of nodes) {
      if (node.type === 'definition') this.#definitions.set(node.identifier, node);
    }
    return nodes.flatMap((node) => this.#block(node, depth));
  }

  /** Blocks nested one level down, or their text past the depth bound. */
  #nested(node: Nodes, children: readonly RootContent[], depth: number): Block[] {
    return depth + 1 >= MarkdownTree.DEPTH
      ? MarkdownTree.#plain(this.#text(node))
      : this.#blocks(children, depth + 1);
  }

  #block(node: RootContent, depth: number): Block[] {
    switch (node.type) {
      case 'heading':
        return [
          { kind: 'heading', level: node.depth, children: this.#inlines(node.children, depth) },
        ];
      case 'paragraph':
        return [{ kind: 'paragraph', children: this.#inlines(node.children, depth) }];
      case 'code':
        return [{ kind: 'code', lang: node.lang ?? '', text: node.value }];
      case 'blockquote':
        return [{ kind: 'quote', children: this.#nested(node, node.children, depth) }];
      case 'list':
        if (depth + 1 >= MarkdownTree.DEPTH) return MarkdownTree.#plain(this.#text(node));
        return [
          {
            kind: 'list',
            ordered: node.ordered ?? false,
            start: node.start ?? 1,
            items: node.children.map((item) => ({
              checked: item.checked ?? null,
              children: this.#blocks(item.children, depth + 1),
            })),
          },
        ];
      case 'thematicBreak':
        return [{ kind: 'rule' }];
      case 'table': {
        const [head, ...rows] = node.children;
        const width = head?.children.length ?? 0;
        // As GFM reads a table: a short row gets empty cells, a long one is cut.
        const cells = (row: typeof head) =>
          Array.from({ length: width }, (_, column) =>
            this.#inlines(row?.children[column]?.children ?? [], depth),
          );
        return [
          {
            kind: 'table',
            align: (node.align ?? []).map((align: AlignType) => align ?? null),
            header: cells(head),
            rows: rows.map(cells),
          },
        ];
      }
      case 'footnoteDefinition':
        return [
          {
            kind: 'footnote',
            label: node.label ?? node.identifier,
            children: this.#nested(node, node.children, depth),
          },
        ];
      case 'definition':
        // Read for references; not shown.
        return [];
      case 'html':
        return MarkdownTree.#plain(node.value);
      default:
        // Anything else is kept as its source text, never dropped.
        return MarkdownTree.#plain(this.#text(node));
    }
  }

  /** Spans nested one level down, or their text past the depth bound. */
  #nestedInlines(node: Nodes, children: readonly PhrasingContent[], depth: number): Inline[] {
    return depth + 1 >= MarkdownTree.DEPTH
      ? [{ kind: 'text', text: this.#text(node) }]
      : this.#inlines(children, depth + 1);
  }

  /** Spans, with neighbouring text joined (raw HTML read as text splits it). */
  #inlines(nodes: readonly PhrasingContent[], depth: number): Inline[] {
    const out: Inline[] = [];
    for (const inline of nodes.flatMap((node) => this.#inline(node, depth))) {
      const last = out[out.length - 1];
      if (last?.kind === 'text' && inline.kind === 'text') {
        out[out.length - 1] = { kind: 'text', text: last.text + inline.text };
      } else {
        out.push(inline);
      }
    }
    return out;
  }

  #inline(node: PhrasingContent, depth: number): Inline[] {
    switch (node.type) {
      case 'text':
        return [{ kind: 'text', text: node.value }];
      case 'inlineCode':
        return [{ kind: 'code', text: node.value }];
      case 'strong':
        return [{ kind: 'strong', children: this.#nestedInlines(node, node.children, depth) }];
      case 'emphasis':
        return [{ kind: 'em', children: this.#nestedInlines(node, node.children, depth) }];
      case 'delete':
        return [{ kind: 'strike', children: this.#nestedInlines(node, node.children, depth) }];
      case 'link':
        return [
          {
            kind: 'link',
            href: node.url,
            children: this.#nestedInlines(node, node.children, depth),
          },
        ];
      case 'image':
        return [{ kind: 'image', alt: node.alt ?? '', href: node.url }];
      case 'linkReference': {
        const children = this.#nestedInlines(node, node.children, depth);
        const target = this.#definitions.get(node.identifier);
        return target ? [{ kind: 'link', href: target.url, children }] : children;
      }
      case 'imageReference': {
        const target = this.#definitions.get(node.identifier);
        return target
          ? [{ kind: 'image', alt: node.alt ?? '', href: target.url }]
          : [{ kind: 'text', text: this.#text(node) }];
      }
      case 'break':
        return [{ kind: 'break' }];
      case 'footnoteReference':
        return [{ kind: 'footnote', label: node.label ?? node.identifier }];
      case 'html':
        return [{ kind: 'text', text: node.value }];
      default:
        // Anything else is kept as its source text, never dropped.
        return [{ kind: 'text', text: this.#text(node) }];
    }
  }
}
