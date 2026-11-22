// A page of search hits as the Search view reads it: grouped by the
// recording each came from (newest group first), each group with where its
// hits fall in its span, and counts by day, field and recording for the
// side column. It reads the page it was given, not every match. Pure: no
// DOM, no clock.

import type { RecordingId, SearchField, SearchHit } from './api.ts';
import type { Stored } from './decode.ts';

/** One recording's hits on the page; `recording` null for hits outside any. */
export interface HitGroup {
  recording: RecordingId | null;
  hits: SearchHit[];
  /** Each hit's place in the group's span, oldest 0 to newest 1. */
  ticks: { at: number; field: Stored<SearchField> }[];
}

/** A count and its share of the largest of its kind (0–1). */
export interface Tally<K> {
  key: K;
  count: number;
  share: number;
}

export class SearchResults {
  readonly groups: HitGroup[];
  /** Hits by local day, oldest first. */
  readonly days: Tally<number>[];
  readonly fields: Tally<Stored<SearchField>>[];
  /** The recordings with the most hits, the most first. */
  readonly recordings: Tally<RecordingId | null>[];

  /** `hits` as the page lists them; `dayStart` gives a time's local day. */
  constructor(hits: readonly SearchHit[], dayStart: (ms: number) => number) {
    const byRecording = new Map<RecordingId | null, SearchHit[]>();
    for (const hit of hits) {
      const own = byRecording.get(hit.recording) ?? [];
      own.push(hit);
      byRecording.set(hit.recording, own);
    }
    const newest = (group: readonly SearchHit[]) => Math.max(...group.map((hit) => hit.started_at));
    this.groups = [...byRecording]
      .toSorted(([, a], [, b]) => newest(b) - newest(a))
      .map(([recording, own]) => {
        const from = Math.min(...own.map((hit) => hit.started_at));
        const span = newest(own) - from;
        return {
          recording,
          hits: own,
          ticks: own.map((hit) => ({
            at: span > 0 ? (hit.started_at - from) / span : 0.5,
            field: hit.field,
          })),
        };
      });
    this.days = SearchResults.#tally(hits.map((hit) => dayStart(hit.started_at))).toSorted(
      (a, b) => a.key - b.key,
    );
    this.fields = SearchResults.#tally(hits.map((hit) => hit.field));
    this.recordings = SearchResults.#tally(hits.map((hit) => hit.recording)).slice(0, 6);
  }

  /** How often each key comes, the most first, with its share of the most. */
  static #tally<K>(keys: readonly K[]): Tally<K>[] {
    const counts = new Map<K, number>();
    for (const key of keys) counts.set(key, (counts.get(key) ?? 0) + 1);
    const most = Math.max(1, ...counts.values());
    return [...counts]
      .map(([key, count]) => ({ key, count, share: count / most }))
      .toSorted((a, b) => b.count - a.count);
  }
}
