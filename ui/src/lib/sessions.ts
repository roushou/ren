// The Sessions page's pure logic: what each row says about its session,
// which rows fold away, and how the rest group by day. No browser, no
// fetching; tested in tests/sessions.test.ts.
import type { CallView, Excerpt, Harness, SessionId, SessionView } from './api.ts';
import { providerName } from './api.ts';
import { harnessLabel } from './activity.ts';
import { Board, type StateMark } from './board.ts';
import type { Stored } from './decode.ts';
import { date } from './display.ts';

/** What a session's newest turn says it is about. */
export interface SessionDetail {
  prompt: Excerpt | null;
  /** The directory the turn's commands change into, when they agree on one. */
  workdir: string | null;
}

/** A session's state right now: a call in flight, recent activity, or neither. */
export type SessionNow =
  | { kind: 'open'; label: string; mark: StateMark }
  | { kind: 'recent'; at: number }
  | { kind: 'quiet'; at: number };

export interface SessionRow {
  session: SessionView;
  harness: Stored<Harness> | null;
  client: string;
  /** Null until its newest turn is read, or when it has none. */
  detail: SessionDetail | null;
  now: SessionNow;
  /** Requests and input tokens against the list's largest, 0–1. */
  requests: number;
  tokens: number;
}

export interface DayGroup {
  key: string;
  label: string;
  rows: SessionRow[];
}

export interface SessionListInput {
  sessions: SessionView[];
  details: ReadonlyMap<SessionId, SessionDetail>;
  /** Model calls in flight, any session. */
  open: CallView[];
  now: number;
  query: string;
  /** The start of a time's local day; injected so tests do not depend on the zone. */
  dayStart: (ms: number) => number;
}

/**
 * The session list: rows that matter grouped by the day each was last seen,
 * newest first, and sessions of a single call with no turn (a client's
 * own check, most often) folded into one line under them.
 */
export class SessionList {
  /** Seen this recently, a session counts as active. */
  static readonly recentMs = 15 * 60_000;

  readonly groups: DayGroup[];
  readonly folded: SessionRow[];

  constructor(input: SessionListInput) {
    const rows = input.sessions
      .map((session) => SessionList.row(session, input))
      .filter((row) => SessionList.matches(row, input.query));
    const most = (pick: (session: SessionView) => number) =>
      Math.max(1, ...rows.map((row) => pick(row.session)));
    const requests = most((session) => session.call_count);
    const tokens = most((session) => session.tokens.input ?? 0);
    const scaled = rows
      .map((row) => ({
        ...row,
        requests: row.session.call_count / requests,
        tokens: (row.session.tokens.input ?? 0) / tokens,
      }))
      .sort((a, b) => b.session.last_seen - a.session.last_seen);
    this.folded = scaled.filter((row) => SessionList.trivial(row.session));
    const groups = new Map<number, DayGroup>();
    for (const row of scaled) {
      if (SessionList.trivial(row.session)) continue;
      const day = input.dayStart(row.session.last_seen);
      const group = groups.get(day) ?? {
        key: String(day),
        label: SessionList.dayLabel(day, input.now, input.dayStart),
        rows: [],
      };
      group.rows.push(row);
      groups.set(day, group);
    }
    this.groups = [...groups.values()];
  }

  /** One call and no turn: a check or a count, not a session of work. */
  static trivial(session: SessionView): boolean {
    return session.call_count <= 1 && session.turn_count === 0;
  }

  static row(session: SessionView, input: SessionListInput): SessionRow {
    const harness = Board.harness(session.source);
    const open = input.open
      .filter((call) => call.session === session.id)
      .sort((a, b) => a.started_at - b.started_at);
    const phase = open.length ? Board.phase(open, input.now) : null;
    const chip = phase ? Board.phaseChip(phase, input.now) : null;
    return {
      session,
      harness,
      client: harness ? harnessLabel(harness) : 'Unknown client',
      detail: input.details.get(session.id) ?? null,
      now: chip
        ? { kind: 'open', label: chip.label, mark: chip.mark }
        : input.now - session.last_seen <= SessionList.recentMs
          ? { kind: 'recent', at: session.last_seen }
          : { kind: 'quiet', at: session.last_seen },
      requests: 0,
      tokens: 0,
    };
  }

  /** Whether a row matches the search: its client, provider, IDs, prompt, or folder. */
  static matches(row: SessionRow, query: string): boolean {
    const needle = query.trim().toLowerCase();
    if (!needle) return true;
    return [
      row.client,
      providerName(row.session.provider),
      row.session.id,
      row.session.native_id,
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

  /** The last part of a directory, as a row names it. */
  static folder(dir: string): string {
    return dir.replace(/\/+$/, '').split('/').at(-1) || dir;
  }
}
