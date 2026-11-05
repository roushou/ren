import { retainSnapshot, type Snapshot } from './snapshots.ts';

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
