import {
  CallId,
  ManagementApi,
  parseSearchField,
  type CallView,
  type ExposureView,
  type Health,
  type Metrics,
  type PageCursor,
  type SessionId,
  type SearchField,
  type SearchPage,
  type SessionReport,
  type SessionView,
  type Storage,
  type TurnActivity,
  type TurnId,
  type TurnSummary,
  type UsageView,
} from './api.ts';
import { localDayStart, parseRange, rangeStart, type UsageRange } from './usage.ts';
import { listingQuery } from './traffic.ts';
import { readSnapshot, snapshotError, type Snapshot } from './snapshots.ts';
import { Board, LivePlace, type LiveWindow } from './board.ts';
import { SessionList, type SessionDetail } from './sessions.ts';

export interface GatewayData {
  health: Health;
  storage: Storage;
}
/** A `?call=` deep link to a call that is not on the current page. */
export type OffPageSelection =
  { kind: 'none' } | { kind: 'loaded'; call: CallView } | { kind: 'failed'; message: string };
export interface TrafficData {
  calls: CallView[];
  next: PageCursor | null;
  session: SessionView | undefined;
  /** Sessions for the picker; empty on a session page. */
  sessions: SessionView[];
  /** How many calls the default "model calls only" hides (at most 100), or null when not hiding. */
  hidden: number | null;
}

/** Listing filters `/api/calls` accepts, besides `limit`. */
const callFilters = [
  'q',
  'session',
  'client',
  'provider',
  'model',
  'tool',
  'status',
  'has_tools',
  'from_ms',
  'until_ms',
  'after',
] as const;

export function gatewaySnapshot(fetcher: typeof fetch) {
  const api = new ManagementApi(fetcher);
  return readSnapshot('gateway', async (): Promise<GatewayData> => {
    // Health reads recorder counters, not storage, so the two run together.
    const [health, storage] = await Promise.all([api.health(), api.storage()]);
    return { health, storage };
  });
}
export interface RecorderData {
  metrics: Metrics;
  /** The start of the hour of the oldest retained call; null when none, or unread. */
  oldest: number | null;
}

/** The Recorder page: its counters, and the oldest retained call for the history limit. */
export function recorderSnapshot(fetcher: typeof fetch) {
  const api = new ManagementApi(fetcher);
  return readSnapshot('recorder', async (): Promise<RecorderData> => {
    const [metrics, retained] = await Promise.all([
      api.metrics(),
      // Usage over every retained call lists its hours oldest first.
      api.usage({}).catch(() => null),
    ]);
    return { metrics, oldest: retained?.hours[0]?.start ?? null };
  });
}
export interface UsageData {
  range: UsageRange;
  /** When the window was computed, so the chart's last bar is "now". */
  now: number;
  usage: UsageView;
  /** The most-used sessions in the window's listing (sessions are not range-filtered). */
  sessions: SessionView[];
}

/** The Usage page: totals for the range, and the sessions list for its top sessions. */
export function usageSnapshot(fetcher: typeof fetch, rawRange: string | null) {
  const api = new ManagementApi(fetcher);
  const range = parseRange(rawRange);
  return readSnapshot(`usage?${range}`, async (): Promise<UsageData> => {
    const now = Date.now();
    const [usage, sessions] = await Promise.all([
      api.usage({ from_ms: rangeStart(range, now, localDayStart) }),
      api.sessions(100),
    ]);
    return { range, now, usage, sessions: sessions.sessions };
  });
}

export interface OverviewData {
  now: number;
  /** The last 24 hours. */
  usage: UsageView;
  /** The 24 hours before that, for the change in calls; null when it could not be read. */
  previous: UsageView | null;
  /** Model calls that failed in the last 24 hours. */
  failed: CallView[];
  /** Model calls cut off in the last 24 hours. */
  stopped: CallView[];
  /** Bookkeeping calls that failed in the last 24 hours: quota probes, token counts. */
  bookkeeping: CallView[];
  sessions: SessionView[];
  /** Credentials found in the last 24 hours; null from a daemon without them. */
  exposures: ExposureView[] | null;
  /** The Live board's last 15 minutes, for the sessions at work; null when unread. */
  live: LiveData | null;
}

/**
 * The overview: everything read at once; each read is small. `live` keeps
 * the Live board's turn cache between refreshes in the browser.
 */
export function overviewSnapshot(fetcher: typeof fetch, live: LiveReader) {
  const api = new ManagementApi(fetcher);
  return readSnapshot('overview', async (): Promise<OverviewData> => {
    const now = Date.now();
    const day = 24 * 3_600_000;
    const since = String(now - day);
    const listing = (status: string, kind: string) =>
      api
        .calls(new URLSearchParams({ status, kind, limit: '100', from_ms: since }))
        .then((page) => page.calls);
    const [usage, previous, failed, stopped, bookkeeping, sessions, exposures, board] =
      await Promise.all([
        api.usage({ from_ms: rangeStart('24h', now, localDayStart) }),
        api.usage({ from_ms: now - 2 * day, until_ms: now - day }).catch(() => null),
        listing('errors', 'model'),
        listing('interrupted', 'model'),
        listing('errors', 'other'),
        api.sessions(100),
        api.exposures(Number(since)).then((page) => page.exposures),
        live.read(fetcher, '15m'),
      ]);
    return {
      now,
      usage,
      previous,
      failed,
      stopped,
      bookkeeping,
      sessions: sessions.sessions,
      exposures,
      live: board.value,
    };
  });
}

export interface SessionsData {
  sessions: SessionView[];
  /** Model calls in flight, for the rows' state. */
  open: CallView[];
  /** Each detailed session's newest turn: its prompt and folder. */
  details: Map<SessionId, SessionDetail>;
}

/**
 * Reads the Sessions page. A session's newest turn says what it is about,
 * and costs two reads (its summary, then its activity for the folder), so
 * a session is read again only when it has been seen since; sessions of a
 * single call with no turn are not read at all.
 */
export class SessionsReader {
  /** The newest sessions read in detail; the rest show without a prompt. */
  static readonly detailed = 40;
  /** Turns read per session, newest first, to find one with a prompt. */
  static readonly recentTurns = 5;
  readonly #details = new Map<SessionId, { lastSeen: number; detail: SessionDetail }>();

  read(fetcher: typeof fetch): Promise<Snapshot<SessionsData>> {
    const api = new ManagementApi(fetcher);
    return readSnapshot('sessions', async (): Promise<SessionsData> => {
      const [listing, open] = await Promise.all([
        api.sessions(100),
        api
          .calls(new URLSearchParams({ status: 'active', kind: 'model', limit: '100' }))
          .then((page) => page.calls),
      ]);
      const wanted = listing.sessions
        .filter((session) => !SessionList.trivial(session))
        .slice(0, SessionsReader.detailed);
      const details = new Map<SessionId, SessionDetail>();
      await Promise.all(
        wanted.map(async (session) => {
          const cached = this.#details.get(session.id);
          if (cached?.lastSeen === session.last_seen) {
            details.set(session.id, cached.detail);
            return;
          }
          try {
            // A turn can carry no prompt of its own (it continues the one before), so
            // the newest few are read and the newest with a prompt names the session.
            const turns = (await api.turns(session.id, SessionsReader.recentTurns)).turns;
            const newest = turns[0];
            const activity = newest ? await api.turn(newest.turn) : null;
            const detail = {
              prompt: turns.find((turn) => turn.prompt?.text.trim())?.prompt ?? null,
              workdir: activity ? Board.workdir(activity) : null,
            };
            this.#details.set(session.id, { lastSeen: session.last_seen, detail });
            details.set(session.id, detail);
          } catch {
            // A session whose turn cannot be read shows without its prompt, not the page empty.
            if (cached) details.set(session.id, cached.detail);
          }
        }),
      );
      return { sessions: listing.sessions, open, details };
    });
  }
}
export async function trafficSnapshot(
  fetcher: typeof fetch,
  url: URL,
  sessionId: SessionId | null,
) {
  const api = new ManagementApi(fetcher);
  // Read by name: Kit re-runs a load on any URL change once it reads the
  // whole URL (even `url.pathname`), but only on a named parameter's change
  // when it reads parameters by name. So the key comes from the route, and
  // the listing never reads `call`: selecting a call must not re-read it.
  const named = new URLSearchParams();
  for (const key of [...callFilters, 'kind', 'since']) {
    const value = url.searchParams.get(key);
    if (value) named.set(key, value);
  }
  const key = `${sessionId ? `/sessions/${sessionId}` : '/traffic'}?${named}`;
  return readSnapshot(key, async (): Promise<TrafficData> => {
    if (sessionId) {
      // A session's own listing accepts only a cursor.
      const [sessions, page] = await Promise.all([
        api.sessions(100),
        api.sessionCalls(sessionId, named.get('after')),
      ]);
      const session = sessions.sessions.find((item) => item.id === sessionId);
      return { calls: page.calls, next: page.next, session, sessions: [], hidden: null };
    }
    const params = listingQuery(named, callFilters, Date.now());
    const hiding = params.get('kind') === 'model';
    const hiddenQuery = new URLSearchParams(params);
    hiddenQuery.set('kind', 'other');
    hiddenQuery.set('limit', '100');
    hiddenQuery.delete('after');
    const [page, sessions, hidden] = await Promise.all([
      api.calls(params),
      api.sessions(100),
      hiding ? api.calls(hiddenQuery).then((other) => other.calls.length) : Promise.resolve(null),
    ]);
    return {
      calls: page.calls,
      next: page.next,
      session: undefined,
      sessions: sessions.sessions,
      hidden,
    };
  });
}

/**
 * A `?call=` link to a call that is not in `calls`: read that call alone.
 * The workbench calls this, not the page load, so changing the selection
 * never re-reads the listing.
 */
export async function offPageSelection(
  fetcher: typeof fetch,
  rawCall: string | null,
  calls: CallView[],
  signal?: AbortSignal,
): Promise<OffPageSelection> {
  if (rawCall === null) return { kind: 'none' };
  const callId = CallId.parse(rawCall);
  if (!callId) return { kind: 'failed', message: 'The linked call ID is invalid.' };
  if (calls.some((call) => call.id === callId)) return { kind: 'none' };
  try {
    return { kind: 'loaded', call: (await new ManagementApi(fetcher).call(callId, signal)).call };
  } catch (error) {
    return {
      kind: 'failed',
      message: snapshotError(error, 'Unable to load the selected call.').message,
    };
  }
}

export interface ReportData {
  report: SessionReport;
  /** The turn listing, for prompts and error counts. */
  turns: TurnSummary[];
  session: SessionView | undefined;
}

/** A session's report, with its turns' prompts and its listing row. */
export function reportSnapshot(fetcher: typeof fetch, sessionId: SessionId) {
  const api = new ManagementApi(fetcher);
  return readSnapshot(`/sessions/${sessionId}/report`, async (): Promise<ReportData> => {
    const [report, turns, sessions] = await Promise.all([
      api.report(sessionId),
      api.turns(sessionId, 100),
      api.sessions(100),
    ]);
    const session = sessions.sessions.find((item) => item.id === sessionId);
    return { report, turns: turns.turns, session };
  });
}

/** What one turn did, read from its calls' bodies. */
export function turnSnapshot(fetcher: typeof fetch, turnId: TurnId) {
  const api = new ManagementApi(fetcher);
  return readSnapshot(`/turns/${turnId}`, () => api.turn(turnId));
}

export interface SearchData {
  /** The text searched for; empty before a search. */
  q: string;
  field: SearchField | null;
  /** Null before a search. */
  page: SearchPage | null;
}

/** A search from the page's URL: `?q=` and optionally `?field=`. */
export function searchSnapshot(fetcher: typeof fetch, url: URL) {
  const api = new ManagementApi(fetcher);
  const q = (url.searchParams.get('q') ?? '').trim();
  const field = parseSearchField(url.searchParams.get('field'));
  const key = new URLSearchParams({ q, field: field ?? '' });
  return readSnapshot(`/search?${key}`, async (): Promise<SearchData> => {
    if (!q) return { q, field, page: null };
    const query = new URLSearchParams({ q, limit: '50' });
    if (field) query.set('field', field);
    return { q, field, page: await api.search(query) };
  });
}

export interface LiveData {
  /** When the snapshot was read: elapsed times count from the page's own clock. */
  read: number;
  sessions: SessionView[];
  /** Model calls started in the board's window, and every call still open. */
  calls: CallView[];
  /** The newest turn of each session shown, where it was read. */
  turns: Map<TurnId, TurnActivity>;
  window: LiveWindow;
  /** A listing came back full: older calls or sessions in the window were not read. */
  truncated: { calls: boolean; sessions: boolean };
}

/**
 * Reads the Live page. Turn activity is the costly read, and it changes
 * only when one of the turn's calls does, so a turn is read again only
 * then, or while some of its text is still being derived.
 */
export class LiveReader {
  readonly #turns = new Map<TurnId, { signature: string; turn: TurnActivity }>();

  read(fetcher: typeof fetch, window: LiveWindow): Promise<Snapshot<LiveData>> {
    const api = new ManagementApi(fetcher);
    const spec = LivePlace.windows[window];
    return readSnapshot(`live:${window}`, async (): Promise<LiveData> => {
      const read = Date.now();
      const recent = new URLSearchParams({
        kind: 'model',
        from_ms: String(read - spec.ms),
        limit: String(spec.calls),
      });
      const open = new URLSearchParams({ kind: 'model', status: 'active', limit: '100' });
      const [sessions, recentPage, openPage] = await Promise.all([
        api.sessions(spec.sessions),
        api.calls(recent),
        api.calls(open),
      ]);
      const calls = [
        ...new Map(
          [...recentPage.calls, ...openPage.calls].map((call) => [call.id, call]),
        ).values(),
      ];
      const bySession = Board.bySession(calls);
      const shown = Board.shown(
        Board.activeSessions(sessions.sessions, bySession, read, spec.ms),
        bySession,
      ).map((session) => session.id);
      const turns = new Map<TurnId, TurnActivity>();
      await Promise.all(
        Board.turnsToRead(shown, calls).map(async ({ turn, signature }) => {
          const cached = this.#turns.get(turn);
          const settled =
            cached?.signature === signature &&
            cached.turn.calls.every((call) => !call.limitations.includes('not_yet_derived'));
          if (cached && settled) {
            turns.set(turn, cached.turn);
            return;
          }
          try {
            const activity = await api.turn(turn);
            this.#turns.set(turn, { signature, turn: activity });
            turns.set(turn, activity);
          } catch {
            // A turn that cannot be read leaves its tile without text, not the page empty.
            if (cached) turns.set(turn, cached.turn);
          }
        }),
      );
      for (const turn of this.#turns.keys()) if (!turns.has(turn)) this.#turns.delete(turn);
      // Sessions come newest first: a full page whose oldest is still in the window may miss some.
      const oldest = sessions.sessions.at(-1);
      return {
        read,
        sessions: sessions.sessions,
        calls,
        turns,
        window,
        truncated: {
          calls: recentPage.calls.length >= spec.calls,
          sessions:
            sessions.sessions.length >= spec.sessions &&
            oldest !== undefined &&
            read - oldest.last_seen <= spec.ms,
        },
      };
    });
  }
}
