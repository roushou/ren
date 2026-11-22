import { untrack } from 'svelte';
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
 * in, the same query's previous rows while it refreshes (or the newest this
 * browser read of it, on a return to the page), or `pending`, for the page
 * to draw its own outline.
 */
export function createArrival<T>(read: () => Arriving<T>) {
  const first = untrack(read);
  // Raw, not proxied: which read a snapshot came from is checked by identity.
  /** The newest read that had a value. */
  let previous = $state.raw<Snapshot<T> | null>(
    Arrival.newest(first.state === 'ready' ? first.snapshot.key : first.key),
  );
  /** What a pending read came back with, and which read it was. */
  let settled = $state.raw<{ from: Arriving<T>; snapshot: Snapshot<T> } | null>(null);
  $effect(() => {
    const arriving = read();
    if (arriving.state === 'ready') {
      if (arriving.snapshot.value !== null) previous = arriving.snapshot;
      Arrival.keep(arriving.snapshot);
      return;
    }
    let current = true;
    void arriving.snapshot.then((snapshot) => {
      Arrival.keep(snapshot);
      if (!current) return;
      settled = { from: arriving, snapshot };
      if (snapshot.value !== null) previous = snapshot;
    });
    return () => {
      current = false;
    };
  });
  const current = $derived.by((): Shown<T> => {
    const arriving = read();
    return Arrival.shown(arriving, settled?.from === arriving ? settled.snapshot : null, previous);
  });
  return {
    get current(): Shown<T> {
      return current;
    },
    /** The snapshot shown; null while the page draws its outline. */
    get snapshot(): Snapshot<T> | null {
      return current.state === 'ready' ? current.snapshot : null;
    },
  };
}

/**
 * The snapshots of the reads `read` lists, as each comes in: for the layout,
 * which reports any page's failed read and counts from it, whether the page
 * received the read ready or pending.
 */
export function createSettled(read: () => readonly (Arriving<unknown> | undefined)[]) {
  // Raw, not proxied: reads are looked up by identity.
  let settled = $state.raw(new Map<Arriving<unknown>, Snapshot<unknown>>());
  $effect(() => {
    const pending = read().filter((arriving) => arriving?.state === 'pending');
    settled = new Map();
    let current = true;
    for (const arriving of pending) {
      if (arriving?.state !== 'pending') continue;
      void arriving.snapshot.then((snapshot) => {
        if (current) settled = new Map(settled).set(arriving, snapshot);
      });
    }
    return () => {
      current = false;
    };
  });
  return {
    /** `arriving`'s snapshot: at once when it came ready, else once it is in. */
    of<T>(arriving: Arriving<T> | undefined): Snapshot<T> | null {
      if (!arriving) return null;
      if (arriving.state === 'ready') return arriving.snapshot;
      return (settled.get(arriving) as Snapshot<T> | undefined) ?? null;
    },
  };
}
