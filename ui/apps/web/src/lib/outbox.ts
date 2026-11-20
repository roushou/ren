// The prompts the operator has sent that the conversation does not show yet:
// each is drawn the moment it is sent, marked with how far it has got, and
// leaves once the client's own echo of it is in the conversation. What is not
// known is said as unknown, never as sent.
// Pure: no browser APIs; tested in tests/outbox.test.ts.
import type { UploadedImage } from './attachments.ts';
import { RunCommandError, type SessionId } from './runs.ts';

/** How a prompt on its way is delivered: a new turn, or into the running one. */
export type OutgoingDelivery = 'start' | 'steer';

/** Whether sending it also starts the session's client, which takes a while. */
export type Starts = 'client' | 'nothing';

/** How far a prompt on its way has got. */
export type Progress =
  /** Asked of the daemon; no answer yet. */
  | { kind: 'sending'; starts: Starts }
  /** The daemon took it and handed it to the client; its echo is on its way. */
  | { kind: 'sent' }
  /** The daemon refused it: it was not sent. */
  | { kind: 'refused'; reason: string }
  /** No answer came back: it may or may not have been sent. */
  | { kind: 'unknown'; reason: string };

/** One prompt on its way. */
export interface Outgoing {
  /** Names it here, until it leaves. */
  key: number;
  session: SessionId;
  text: string;
  images: UploadedImage[];
  delivery: OutgoingDelivery;
  /** How many prompts with its text the conversation showed when it was sent. */
  seen: number;
  progress: Progress;
}

/** What a prompt to send says, before it has a place in the outbox. */
export interface Prompting {
  session: SessionId;
  text: string;
  images: UploadedImage[];
  delivery: OutgoingDelivery;
}

/**
 * Every session's prompts on their way, oldest first. Immutable: each change
 * returns a new outbox.
 */
export class Outbox {
  readonly items: readonly Outgoing[];
  readonly #next: number;

  constructor(items: readonly Outgoing[] = [], next = 0) {
    this.items = items;
    this.#next = next;
  }

  /**
   * `prompt` is being sent, while the conversation shows `shown` (its
   * prompts' texts); `starts` says whether the session's client starts for it.
   */
  sending(
    prompt: Prompting,
    shown: readonly string[],
    starts: Starts,
  ): { outbox: Outbox; key: number } {
    const key = this.#next;
    const seen = shown.filter((text) => text === prompt.text).length;
    const item: Outgoing = { ...prompt, key, seen, progress: { kind: 'sending', starts } };
    return { outbox: new Outbox([...this.items, item], key + 1), key };
  }

  /** The daemon took prompt `key`. */
  sent(key: number): Outbox {
    return this.#progress(key, { kind: 'sent' });
  }

  /**
   * Sending prompt `key` failed with `error`: a refusal the daemon answered
   * means it was not sent; anything else (no answer, a timeout, the daemon
   * or this UI server failing) leaves it unknown.
   */
  failed(key: number, error: unknown): Outbox {
    return this.#progress(key, Outbox.outcome(error));
  }

  /** What a failed send says about its prompt. */
  static outcome(error: unknown): Extract<Progress, { kind: 'refused' | 'unknown' }> {
    if (error instanceof RunCommandError && error.status >= 400 && error.status < 500) {
      return { kind: 'refused', reason: error.message };
    }
    // A failed fetch is a TypeError; a timeout, a TimeoutError; the daemon
    // or this UI server failing, a refusal with its own sentence.
    const reason =
      error instanceof RunCommandError
        ? error.message
        : error instanceof Error && error.name === 'TimeoutError'
          ? 'No answer came in time.'
          : error instanceof TypeError
            ? 'The connection failed.'
            : 'No answer came.';
    return { kind: 'unknown', reason };
  }

  /** Take prompt `key` out: edited, sent again, or dismissed. */
  without(key: number): Outbox {
    return new Outbox(
      this.items.filter((item) => item.key !== key),
      this.#next,
    );
  }

  /** `session`'s prompts on their way, oldest first. */
  of(session: SessionId): Outgoing[] {
    return this.items.filter((item) => item.session === session);
  }

  /** Whether one of `session`'s prompts has no answer from the daemon yet. */
  waiting(session: SessionId): boolean {
    return this.items.some((item) => item.session === session && item.progress.kind === 'sending');
  }

  /**
   * The outbox once `session`'s conversation shows `shown` (its prompts'
   * texts): a prompt shown more often than when it was sent is the client's
   * echo of it, whatever was known of it before, and it leaves. Two waiting
   * with one text take the echoes in order.
   */
  shown(session: SessionId, shown: readonly string[]): Outbox {
    const echoed = new Map<string, number>();
    const items = this.items.filter((item) => {
      if (item.session !== session) return true;
      const count = shown.filter((text) => text === item.text).length;
      const taken = echoed.get(item.text) ?? 0;
      if (count - taken > item.seen) {
        echoed.set(item.text, taken + 1);
        return false;
      }
      return true;
    });
    return items.length === this.items.length ? this : new Outbox(items, this.#next);
  }

  #progress(key: number, progress: Progress): Outbox {
    return new Outbox(
      this.items.map((item) => (item.key === key ? { ...item, progress } : item)),
      this.#next,
    );
  }
}
