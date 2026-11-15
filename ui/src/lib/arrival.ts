/**
 * A view's data as its page receives it: read already, or still on its way.
 *
 * A page that takes its data this way renders at once on a navigation in
 * the browser, with its own outline where the values will go, instead of
 * keeping the old page up until every read is done. A first load is still
 * read in full on the server, so the page arrives complete.
 */
import { readSnapshot, retainSnapshot, type Snapshot } from './snapshots.ts';

export type Arriving<T> =
  | { state: 'ready'; snapshot: Snapshot<T> }
  | { state: 'pending'; key: string; snapshot: Promise<Snapshot<T>> };

/** What a page shows of an arriving read: the snapshot, or its outline. */
export type Shown<T> = { state: 'ready'; snapshot: Snapshot<T> } | { state: 'pending' };

export class Arrival {
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

  /** `read` under `key`, handed over pending when streaming. */
  static read<T>(key: string, read: () => Promise<T>): Promise<Arriving<T>> {
    return Arrival.over(key, readSnapshot(key, read), Arrival.#streaming);
  }

  /** A snapshot already on its way under `key`, handed over pending when streaming. */
  static hand<T>(key: string, snapshot: Promise<Snapshot<T>>): Promise<Arriving<T>> {
    return Arrival.over(key, snapshot, Arrival.#streaming);
  }

  /** A snapshot on its way, handed over pending, or once it is read. */
  static async over<T>(
    key: string,
    snapshot: Promise<Snapshot<T>>,
    streaming: boolean,
  ): Promise<Arriving<T>> {
    return streaming
      ? { state: 'pending', key, snapshot }
      : { state: 'ready', snapshot: await snapshot };
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
