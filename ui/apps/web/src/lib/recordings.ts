// The Recordings page's pure logic: what each row says about its recording,
// which rows fold away, and how the rest group by day. No browser, no
// fetching; tested in tests/recordings.test.ts.
import type { CallView, Excerpt, Harness, RecordingId, RecordingView } from './api.ts';
import { providerName } from './api.ts';
import { Board } from './board.ts';
import type { Mark } from './marks.ts';
import type { Stored } from './decode.ts';
import { date } from './display.ts';

/** What a recording's newest turn says it is about. */
export interface RecordingDetail {
  prompt: Excerpt | null;
  /** The directory the turn's commands change into, when they agree on one. */
  workdir: string | null;
}

/** A recording's state right now: a call in flight, recent activity, or neither. */
export type RecordingNow =
  | { kind: 'open'; label: string; mark: Mark }
  | { kind: 'recent'; at: number }
  | { kind: 'quiet'; at: number };

export interface RecordingRow {
  recording: RecordingView;
  harness: Stored<Harness> | null;
  client: string;
  /** Null until its newest turn is read, or when it has none. */
  detail: RecordingDetail | null;
  now: RecordingNow;
  /** Requests and input tokens against the list's largest, 0–1. */
  requests: number;
  tokens: number;
}

export interface DayGroup {
  key: string;
  label: string;
  rows: RecordingRow[];
}

export interface RecordingListInput {
  recordings: RecordingView[];
  details: ReadonlyMap<RecordingId, RecordingDetail>;
  /** Model calls in flight, any recording. */
  open: CallView[];
  now: number;
  query: string;
  /** The start of a time's local day; injected so tests do not depend on the zone. */
  dayStart: (ms: number) => number;
}

/**
 * The recording list: rows that matter grouped by the day each was last seen,
 * newest first, and recordings of a single call with no turn (a client's
 * own check, most often) folded into one line under them.
 */
export class RecordingList {
  /** Seen this recently, a recording counts as active. */
  static readonly recentMs = 15 * 60_000;

  readonly groups: DayGroup[];
  readonly folded: RecordingRow[];

  constructor(input: RecordingListInput) {
    const rows = input.recordings
      .map((recording) => RecordingList.row(recording, input))
      .filter((row) => RecordingList.matches(row, input.query));
    const most = (pick: (recording: RecordingView) => number) =>
      Math.max(1, ...rows.map((row) => pick(row.recording)));
    const requests = most((recording) => recording.call_count);
    const tokens = most((recording) => recording.tokens.input ?? 0);
    const scaled = rows
      .map((row) => ({
        ...row,
        requests: row.recording.call_count / requests,
        tokens: (row.recording.tokens.input ?? 0) / tokens,
      }))
      .sort((a, b) => b.recording.last_seen - a.recording.last_seen);
    this.folded = scaled.filter((row) => RecordingList.trivial(row.recording));
    const groups = new Map<number, DayGroup>();
    for (const row of scaled) {
      if (RecordingList.trivial(row.recording)) continue;
      const day = input.dayStart(row.recording.last_seen);
      const group = groups.get(day) ?? {
        key: String(day),
        label: RecordingList.dayLabel(day, input.now, input.dayStart),
        rows: [],
      };
      group.rows.push(row);
      groups.set(day, group);
    }
    this.groups = [...groups.values()];
  }

  /** One call and no turn: a check or a count, not a recording of work. */
  static trivial(recording: RecordingView): boolean {
    return recording.call_count <= 1 && recording.turn_count === 0;
  }

  static row(recording: RecordingView, input: RecordingListInput): RecordingRow {
    const harness = Board.harness(recording.source);
    const open = input.open
      .filter((call) => call.recording === recording.id)
      .sort((a, b) => a.started_at - b.started_at);
    const phase = open.length ? Board.phase(open, input.now) : null;
    const chip = phase ? Board.phaseChip(phase, input.now) : null;
    return {
      recording,
      harness,
      client: Board.clientName(harness),
      detail: input.details.get(recording.id) ?? null,
      now: chip
        ? { kind: 'open', label: chip.label, mark: chip.mark }
        : input.now - recording.last_seen <= RecordingList.recentMs
          ? { kind: 'recent', at: recording.last_seen }
          : { kind: 'quiet', at: recording.last_seen },
      requests: 0,
      tokens: 0,
    };
  }

  /** Whether a row matches the search: its client, provider, IDs, prompt, or folder. */
  static matches(row: RecordingRow, query: string): boolean {
    const needle = query.trim().toLowerCase();
    if (!needle) return true;
    return [
      row.client,
      providerName(row.recording.provider),
      row.recording.id,
      row.recording.native_id,
      row.detail?.prompt?.text ?? '',
      row.detail?.workdir ?? '',
    ].some((text) => text.toLowerCase().includes(needle));
  }

  /** `Today`, `Yesterday`, or the date. */
  static dayLabel(day: number, now: number, dayStart: (ms: number) => number): string {
    const today = dayStart(now);
    if (day === today) return 'Today';
    if (day === dayStart(today - 1)) return 'Yesterday';
    return date(day);
  }
}
