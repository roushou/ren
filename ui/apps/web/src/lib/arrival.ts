/**
 * A view's data as its page receives it: read already, or still on its way.
 *
 * On a navigation in the browser, a read that finishes within a short grace
 * period arrives ready, so the page renders once, complete, as on a first
 * load. Only a read that takes longer is handed over pending: the page then
 * renders at once, with the newest value this browser read under the same
 * key, or its own outline where the values will go, instead of keeping the
 * old page up until every read is done. A first load is read in full on the
 * server, so the page arrives complete.
 */
import { readSnapshot, retainSnapshot, type Snapshot } from './snapshots.ts';

export type Arriving<T> =
  | { state: 'ready'; snapshot: Snapshot<T> }
  | { state: 'pending'; key: string; snapshot: Promise<Snapshot<T>> };

/** A read that came back within its wait, or one still on its way. */
export type Within<T> = { state: 'in'; value: T } | { state: 'late' };

/** What a page shows of an arriving read: the snapshot, or its outline. */
export type Shown<T> = { state: 'ready'; snapshot: Snapshot<T> } | { state: 'pending' };

export class Arrival {
  /**
   * How long a navigation waits for a read before handing it over pending,
   * in ms. Below it, an outline would only flash by before the page; above
   * it, the outline shows the page is coming instead of the old one lingering.
   */
  static readonly GRACE_MS = 200;

  /** How many reads' newest values the browser keeps. */
  static readonly KEPT = 32;

  /** The newest value read under each key, in this browser; oldest first. */
  static #newest = new Map<string, Snapshot<unknown>>();

  /** Whether reads are handed over before they finish. */
  static #streaming = false;

  /**
   * From now on, reads in this browser are handed over pending. The root
   * layout calls this once it has hydrated: the loads that ran for the
   * server-rendered page must finish first, or hydration would meet an
   * outline where the server sent the data.
   */
  static stream(): void {
    Arrival.#streaming = true;
  }

  /** Keep `snapshot` as the newest read of its key, when it has a value.
   * Only the browser keeps any: the page's effects call this. */
  static keep<T>(snapshot: Snapshot<T>): void {
    if (snapshot.value === null) return;
    Arrival.#newest.delete(snapshot.key);
    Arrival.#newest.set(snapshot.key, snapshot);
    for (const key of Arrival.#newest.keys()) {
      if (Arrival.#newest.size <= Arrival.KEPT) break;
      Arrival.#newest.delete(key);
    }
  }

  /** The newest read of `key` this browser kept; null when none. */
  static newest<T>(key: string): Snapshot<T> | null {
    return (Arrival.#newest.get(key) as Snapshot<T> | undefined) ?? null;
  }

  /** `arriving`'s value now: read, or the newest kept under its key. */
  static value<T>(arriving: Arriving<T> | undefined): T | null {
    if (!arriving) return null;
    return arriving.state === 'ready'
      ? arriving.snapshot.value
      : (Arrival.newest<T>(arriving.key)?.value ?? null);
  }

  /** `read` under `key`, handed over pending when streaming and it is late. */
  static read<T>(key: string, read: () => Promise<T>): Promise<Arriving<T>> {
    return Arrival.over(key, readSnapshot(key, read), Arrival.#streaming);
  }

  /** A snapshot already on its way under `key`, handed over pending when
   * streaming and it is late. */
  static hand<T>(key: string, snapshot: Promise<Snapshot<T>>): Promise<Arriving<T>> {
    return Arrival.over(key, snapshot, Arrival.#streaming);
  }

  /**
   * A snapshot on its way: once it is read, or, when streaming, as soon as
   * it is in or the grace period is over, whichever comes first.
   */
  static async over<T>(
    key: string,
    snapshot: Promise<Snapshot<T>>,
    streaming: boolean,
    graceMs = Arrival.GRACE_MS,
  ): Promise<Arriving<T>> {
    if (!streaming) return { state: 'ready', snapshot: await snapshot };
    const within = await Arrival.within(snapshot, graceMs);
    switch (within.state) {
      case 'in':
        return { state: 'ready', snapshot: within.value };
      case 'late':
        return { state: 'pending', key, snapshot };
      default: {
        const never: never = within;
        return never;
      }
    }
  }

  /** `read`, if it is in within `ms`; else word that it is late. A server
   * load uses this too, for the reads it streams on a navigation. */
  static async within<T>(read: Promise<T>, ms: number): Promise<Within<T>> {
    let timer: ReturnType<typeof setTimeout> | undefined;
    const late = new Promise<Within<T>>((resolve) => {
      timer = setTimeout(() => resolve({ state: 'late' }), ms);
    });
    try {
      return await Promise.race([read.then((value): Within<T> => ({ state: 'in', value })), late]);
    } finally {
      clearTimeout(timer);
    }
  }

  /**
   * What to show of `arriving`, given `settled`, what this very read came
   * back with once it is in, and `previous`, the newest read that had a
   * value. A read that failed keeps the previous rows of the same query; while
   * one is on its way, the same query's previous rows stay (a refresh does not
   * blank the page), and anything else shows the outline. A read of another
   * query is never shown in its place.
   */
  static shown<T>(
    arriving: Arriving<T>,
    settled: Snapshot<T> | null,
    previous: Snapshot<T> | null,
  ): Shown<T> {
    if (arriving.state === 'ready')
      return { state: 'ready', snapshot: retainSnapshot(previous, arriving.snapshot) };
    if (settled) return { state: 'ready', snapshot: retainSnapshot(previous, settled) };
    if (previous?.key === arriving.key) return { state: 'ready', snapshot: previous };
    return { state: 'pending' };
  }
}
