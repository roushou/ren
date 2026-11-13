/**
 * Redline's state for one page view: whether marking is on, the marks made,
 * and the one whose note is being written. The layout owns it; the layer,
 * the panel, and the status line's toggle read it.
 */
import { Redline, type Mark, type Target } from './redline.ts';

/** A mark, with the element it points at while that element is on the page. */
export interface Placed extends Mark {
  element: Element;
}

export class RedlineDesk {
  open = $state(false);
  marks = $state<Placed[]>([]);
  /** The mark whose note is being written. */
  editing = $state<number | null>(null);
  #next = 1;

  toggle() {
    this.open = !this.open;
    if (!this.open) this.drop(this.editing);
  }

  /** Mark `element`, and start its note. */
  add(element: Element) {
    const box = element.getBoundingClientRect();
    const target: Target = {
      selector: Redline.selector(element),
      text: Redline.text(element.textContent ?? ''),
      source: RedlineDesk.source(element),
      width: box.width,
      height: box.height,
    };
    const id = this.#next++;
    this.marks.push({ id, target, note: '', element });
    this.editing = id;
  }

  /** Keep `note` on mark `id`; a blank note drops the mark. */
  save(id: number, note: string) {
    const mark = this.marks.find((entry) => entry.id === id);
    if (mark) mark.note = note.trim();
    if (!note.trim()) this.drop(id);
    this.editing = null;
  }

  drop(id: number | null) {
    if (id === null) return;
    this.marks = this.marks.filter((mark) => mark.id !== id);
    if (this.editing === id) this.editing = null;
  }

  clear() {
    this.marks = [];
    this.editing = null;
    this.#next = 1;
  }

  /** The source file and line Svelte's development build attaches to an
   * element or its nearest ancestor; production builds attach none. */
  static source(element: Element): string | null {
    let at: Element | null = element;
    for (let depth = 0; at && depth < 6; depth += 1) {
      const meta = (at as Element & { __svelte_meta?: { loc?: { file?: string; line?: number } } })
        .__svelte_meta;
      if (meta?.loc?.file) return `${meta.loc.file}:${meta.loc.line ?? 0}`;
      at = at.parentElement;
    }
    return null;
  }
}
