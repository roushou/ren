/**
 * Redline's pure core: what a mark points at, and the brief an agent gets
 * from the marks on a page. The overlay reads the page (`RedlineLayer`);
 * nothing here touches the DOM, so the brief is tested without a browser.
 * Where a brief goes, and what the host adds to it, are the host's: it
 * passes a `Framing` in.
 */

/** The part of an element a selector is built from: what a DOM element has. */
export interface ElementLike {
  readonly tagName: string;
  readonly id: string;
  readonly classList: Iterable<string>;
  readonly parentElement: ElementLike | null;
  getAttribute(name: string): string | null;
}

/** What a mark points at, as the overlay read it from the page. */
export interface Target {
  /** A CSS selector from the nearest labelled region down to the element. */
  selector: string;
  /** The element's own text, trimmed and cut short; empty when it has none. */
  text: string;
  /** The source file and line Svelte's development build names, when known. */
  source: string | null;
  /** Its size on screen, in CSS pixels. */
  width: number;
  height: number;
}

/** One mark: what it points at, and what the operator wrote about it. */
export interface Mark {
  id: number;
  target: Target;
  note: string;
}

/** The page the marks were made on. */
export interface PageContext {
  /** Its path and query, as the address bar shows them. */
  path: string;
  title: string;
  width: number;
  height: number;
  theme: 'light' | 'dark';
}

/** What the host adds to a brief: lines about the page, after where it was
 * marked, and the paragraph that closes it, saying how to work. */
export interface Framing {
  about: readonly string[];
  closing: string | null;
}

export class Redline {
  /** How many ancestors a selector names, below its labelled region. */
  static readonly DEPTH = 3;
  /** The most text a target keeps. */
  static readonly TEXT = 80;

  /**
   * A readable selector for `element`: up to `DEPTH` steps of tag, id, and
   * classes (without Svelte's scoping hashes), from the nearest ancestor
   * with an `aria-label`, which names where on the page it is.
   */
  static selector(element: ElementLike): string {
    const steps: string[] = [];
    let at: ElementLike | null = element;
    while (at && steps.length < Redline.DEPTH) {
      steps.unshift(Redline.#step(at));
      at = at.parentElement;
    }
    for (let up: ElementLike | null = at; up; up = up.parentElement) {
      const label = up.getAttribute('aria-label');
      if (label) return `[aria-label="${label.replaceAll('"', '\\"')}"] ${steps.join(' > ')}`;
    }
    return steps.join(' > ');
  }

  static #step(element: ElementLike): string {
    const tag = element.tagName.toLowerCase();
    if (element.id) return `${tag}#${element.id}`;
    const classes = [...element.classList].filter((name) => !name.startsWith('svelte-'));
    return classes.length ? `${tag}.${classes.join('.')}` : tag;
  }

  /** What a mark is called in the pane: the element's text, else the last
   * step of its selector. */
  static name(target: Target): string {
    if (target.text) return `“${target.text}”`;
    const steps = target.selector.split(' > ');
    return steps[steps.length - 1] ?? target.selector;
  }

  /** An element's text as a target keeps it: one line, cut to `TEXT`. */
  static text(raw: string): string {
    const line = raw.replace(/\s+/g, ' ').trim();
    return line.length > Redline.TEXT ? `${line.slice(0, Redline.TEXT - 1)}…` : line;
  }

  /** The marks a brief carries: those with a note. */
  static noted<M extends Mark>(marks: readonly M[]): M[] {
    return marks.filter((mark) => mark.note.trim());
  }

  /** A branch name for the change: `redline/` and the first note's words. */
  static branch(marks: readonly Mark[]): string {
    const words = (marks[0]?.note ?? '')
      .toLowerCase()
      .replace(/[^a-z0-9]+/g, ' ')
      .trim()
      .split(' ')
      .filter((word) => word.length > 1)
      .slice(0, 5);
    return `redline/${words.length ? words.join('-') : 'change'}`;
  }

  /** `text` as a Markdown code span, fenced past any backticks inside. */
  static code(text: string): string {
    const longest = Math.max(0, ...(text.match(/`+/g) ?? []).map((run) => run.length));
    const fence = '`'.repeat(longest + 1);
    return longest ? `${fence} ${text} ${fence}` : `${fence}${text}${fence}`;
  }

  /** The brief an agent gets, in Markdown: what the first mark asks (a
   * session is named by its first line, so it stays plain), the page and
   * what `framing` says of it, each mark as a list item with what it points
   * at, and `framing`'s closing paragraph. Marks without a note are left
   * out. */
  static brief(page: PageContext, marks: readonly Mark[], framing: Framing): string {
    const noted = Redline.noted(marks);
    const first = noted[0]?.note.trim().split('\n')[0] ?? 'Changes to a page';
    const more = noted.length > 1 ? ` (and ${noted.length - 1} more)` : '';
    const lines = [
      `Redline: ${first}${more}`,
      '',
      `Marked on ${Redline.code(page.path)}${page.title ? ` (${page.title})` : ''}, ${page.width}×${page.height}, ${page.theme} theme.`,
      ...framing.about,
      '',
    ];
    noted.forEach((mark, index) => {
      const { target } = mark;
      // A note's later lines are indented, so they stay in its item.
      const note = mark.note.trim().split('\n').join('\n   ');
      lines.push(`${index + 1}. ${note}`);
      lines.push(
        `   - Element: ${Redline.code(target.selector)}${target.text ? `, “${target.text}”` : ''}`,
      );
      if (target.source) lines.push(`   - Source: ${Redline.code(target.source)}`);
      lines.push(`   - Size: ${Math.round(target.width)}×${Math.round(target.height)}`);
    });
    if (framing.closing) lines.push('', framing.closing);
    return lines.join('\n');
  }
}
