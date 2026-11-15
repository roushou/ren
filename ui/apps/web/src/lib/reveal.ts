/**
 * Showing streamed text smoothly. Updates arrive in steps (a few a second),
 * so a reveal moves what is shown toward the newest text at a steady rate,
 * set when each update arrives so it catches up about when the next one is
 * due. Pure: the caller feeds it updates and elapsed time, and draws `text`.
 */
export class Reveal {
  /** Characters a second, at least: a model's typical output speed. */
  static readonly MIN_RATE = 80;
  /** How long a reveal takes to catch up with a new update. */
  static readonly CATCH_UP_MS = 300;

  #target = '';
  /** Characters shown, fractional so slow rates still advance. */
  #shown = 0;
  /** Characters a second, for the current update. */
  #rate = Reveal.MIN_RATE;

  /** Start from `text` already shown in full (what a card first draws). */
  constructor(text = '') {
    this.#target = text;
    this.#shown = text.length;
  }

  /** A newer update. Text that no longer continues what is shown (its start
   * was cut away) is shown at once; otherwise the rest is revealed. */
  update(target: string): void {
    const shown = this.text;
    this.#target = target;
    if (!target.startsWith(shown)) {
      this.#shown = target.length;
      return;
    }
    const behind = target.length - shown.length;
    this.#rate = Math.max(Reveal.MIN_RATE, (behind * 1000) / Reveal.CATCH_UP_MS);
  }

  /** Reveal more, for `elapsedMs` that passed. */
  advance(elapsedMs: number): void {
    this.#shown = Math.min(this.#target.length, this.#shown + (this.#rate * elapsedMs) / 1000);
  }

  /** Show everything now (reduced motion, or a finished call). */
  finish(): void {
    this.#shown = this.#target.length;
  }

  get text(): string {
    return this.#target.slice(0, Math.floor(this.#shown));
  }

  get done(): boolean {
    return Math.floor(this.#shown) >= this.#target.length;
  }
}
