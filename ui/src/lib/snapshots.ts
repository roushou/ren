import { ApiVersionError } from './api.ts';

/** Why a snapshot could not be read. `incompatible` is a daemon/UI version mismatch. */
export interface SnapshotError {
  kind: 'unreachable' | 'incompatible' | 'failed';
  message: string;
}

export interface Snapshot<T> {
  key: string;
  value: T | null;
  error: SnapshotError | null;
  updated: number | null;
}

export function snapshotError(
  error: unknown,
  fallback = 'Unable to load this view.',
): SnapshotError {
  if (error instanceof ApiVersionError) return { kind: 'incompatible', message: error.message };
  if (error instanceof TypeError)
    return { kind: 'unreachable', message: 'Cannot reach TGV. Check that the daemon is running.' };
  return { kind: 'failed', message: error instanceof Error ? error.message : fallback };
}

export async function readSnapshot<T>(key: string, read: () => Promise<T>): Promise<Snapshot<T>> {
  try {
    return { key, value: await read(), error: null, updated: Date.now() };
  } catch (error) {
    return { key, value: null, error: snapshotError(error), updated: null };
  }
}

// Keep stale data only for the same query, never across filters or sessions.
export function retainSnapshot<T>(previous: Snapshot<T> | null, next: Snapshot<T>): Snapshot<T> {
  return next.value === null && previous?.key === next.key
    ? { ...previous, error: next.error }
    : next;
}
