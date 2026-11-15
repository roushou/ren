import { retainSnapshot, type Snapshot } from './snapshots.ts';
import { Arrival, type Arriving, type Shown } from './arrival.ts';

export function createSnapshot<T>(read: () => Snapshot<T>) {
  let previous = $state<Snapshot<T> | null>(null);
  $effect(() => {
    const next = read();
    if (next.value !== null) previous = next;
  });
  return {
    get current() {
      return retainSnapshot(previous, read());
    },
  };
}

/**
 * A page's view of an arriving read (`Arrival`): the snapshot once it is
 * in, the same query's previous rows while it refreshes, or `pending`, for
 * the page to draw its own outline.
 */
export function createArrival<T>(read: () => Arriving<T>) {
  // Raw, not proxied: which read a snapshot came from is checked by identity.
  /** The newest read that had a value. */
  let previous = $state.raw<Snapshot<T> | null>(null);
  /** What a pending read came back with, and which read it was. */
  let settled = $state.raw<{ from: Arriving<T>; snapshot: Snapshot<T> } | null>(null);
  $effect(() => {
    const arriving = read();
    if (arriving.state === 'ready') {
      if (arriving.snapshot.value !== null) previous = arriving.snapshot;
      return;
    }
    let current = true;
    void arriving.snapshot.then((snapshot) => {
      if (!current) return;
      settled = { from: arriving, snapshot };
      if (snapshot.value !== null) previous = snapshot;
    });
    return () => {
      current = false;
    };
  });
  return {
    get current(): Shown<T> {
      const arriving = read();
      return Arrival.shown(
        arriving,
        settled?.from === arriving ? settled.snapshot : null,
        previous,
      );
    },
  };
}
