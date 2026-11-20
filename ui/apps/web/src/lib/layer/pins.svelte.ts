// Which kinds of slide-over the operator pinned beside the page (call
// details, a step, a recording's report). A pin is this browser's
// convenience: kept in local storage when it can be, and fine without.

const KEY = 'ren.slide-pins';

class SlidePins {
  #pinned = $state<string[]>(SlidePins.#read());

  has(kind: string): boolean {
    return this.#pinned.includes(kind);
  }

  toggle(kind: string): void {
    this.#pinned = this.has(kind)
      ? this.#pinned.filter((each) => each !== kind)
      : [...this.#pinned, kind];
    try {
      localStorage.setItem(KEY, JSON.stringify(this.#pinned));
    } catch {
      // Storage refused (a private window, a preview): the pin lasts the visit.
    }
  }

  static #read(): string[] {
    try {
      const stored: unknown = JSON.parse(globalThis.localStorage?.getItem(KEY) ?? '[]');
      return Array.isArray(stored)
        ? stored.filter((each): each is string => typeof each === 'string')
        : [];
    } catch {
      return [];
    }
  }
}

export const pins = new SlidePins();
