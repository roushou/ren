// Which kinds of slide-over the operator pinned beside the page (call
// details, a step, a recording's report). A pin is this browser's
// convenience: kept in local storage when it can be, and fine without.

const KEY = 'ren.slide-pins';

/** The kinds of slide-over that can be pinned. */
export const SLIDE_KINDS = ['call', 'package', 'report', 'step', 'turn'] as const;
export type SlideKind = (typeof SLIDE_KINDS)[number];

class SlidePins {
  #pinned = $state<SlideKind[]>(SlidePins.#read());

  has(kind: SlideKind): boolean {
    return this.#pinned.includes(kind);
  }

  toggle(kind: SlideKind): void {
    this.#pinned = this.has(kind)
      ? this.#pinned.filter((each) => each !== kind)
      : [...this.#pinned, kind];
    try {
      localStorage.setItem(KEY, JSON.stringify(this.#pinned));
    } catch {
      // Storage refused (a private window, a preview): the pin lasts the visit.
    }
  }

  /** What this browser kept, read as kinds; anything else it held is
   * dropped. */
  static #read(): SlideKind[] {
    try {
      const stored: unknown = JSON.parse(globalThis.localStorage?.getItem(KEY) ?? '[]');
      return Array.isArray(stored)
        ? stored.filter((each): each is SlideKind => SLIDE_KINDS.includes(each))
        : [];
    } catch {
      return [];
    }
  }
}

export const pins = new SlidePins();
