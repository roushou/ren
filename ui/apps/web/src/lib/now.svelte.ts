// The time, read reactively: what reads `current` (a derived, an effect, the
// markup) reads it again every `everyMs`, and the clock ticks only while
// something reads it.
import { createSubscriber } from 'svelte/reactivity';

export class Now {
  readonly #subscribe: () => void;

  constructor(everyMs: number) {
    this.#subscribe = createSubscriber((update) => {
      const ticking = setInterval(update, everyMs);
      return () => clearInterval(ticking);
    });
  }

  get current(): number {
    this.#subscribe();
    return Date.now();
  }
}
