import {
  CallId,
  ManagementApi,
  type CallView,
  type Health,
  type Metrics,
  type PageCursor,
  type SessionId,
  type SessionView,
  type Storage,
  type UsageView,
} from './api.ts';
import { localDayStart, parseRange, rangeStart, type UsageRange } from './usage.ts';
import { readSnapshot, snapshotError } from './snapshots.ts';

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
}

/** Listing filters `/api/calls` accepts, besides `limit`. */
const callFilters = [
  'q',
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
export function recorderSnapshot(fetcher: typeof fetch) {
  const api = new ManagementApi(fetcher);
  return readSnapshot('recorder', (): Promise<Metrics> => api.metrics());
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
  /** Calls still open, newest first. */
  active: CallView[];
  /** Errors and stopped calls in the last 24 hours. */
  errors: CallView[];
  stopped: CallView[];
  sessions: SessionView[];
}

/** The overview: everything read at once; each read is small. */
export function overviewSnapshot(fetcher: typeof fetch) {
  const api = new ManagementApi(fetcher);
  return readSnapshot('overview', async (): Promise<OverviewData> => {
    const now = Date.now();
    const since = String(now - 24 * 3_600_000);
    const listing = (status: string, limit: number, from?: string) => {
      const query = new URLSearchParams({ status, limit: String(limit) });
      if (from) query.set('from_ms', from);
      return api.calls(query).then((page) => page.calls);
    };
    const [usage, active, errors, stopped, sessions] = await Promise.all([
      api.usage({ from_ms: rangeStart('24h', now, localDayStart) }),
      listing('active', 50),
      listing('errors', 10, since),
      listing('interrupted', 10, since),
      api.sessions(100),
    ]);
    return { now, usage, active, errors, stopped, sessions: sessions.sessions };
  });
}

export function sessionsSnapshot(fetcher: typeof fetch) {
  const api = new ManagementApi(fetcher);
  return readSnapshot('sessions', async () => (await api.sessions(100)).sessions);
}
export async function trafficSnapshot(
  fetcher: typeof fetch,
  url: URL,
  sessionId: SessionId | null,
) {
  const api = new ManagementApi(fetcher);
  const params = new URLSearchParams();
  // Session listings accept only a cursor; the global listing accepts filters.
  for (const key of sessionId ? (['after'] as const) : callFilters) {
    const value = url.searchParams.get(key);
    if (value) params.set(key, value);
  }
  // Kit re-runs a load on any URL change once it reads the whole URL (even
  // `url.pathname`), but only on a named parameter's change when it reads
  // parameters by name. So the key comes from the route, and the listing
  // never reads `call`: selecting a call must not re-read the listing.
  const key = `${sessionId ? `/sessions/${sessionId}` : '/traffic'}?${params}`;
  return readSnapshot(key, async (): Promise<TrafficData> => {
    // The bounded daemon reader is shared with body previews: query serially.
    let session: SessionView | undefined;
    if (sessionId) {
      session = (await api.sessions(100)).sessions.find((item) => item.id === sessionId);
    }
    const page = sessionId
      ? await api.sessionCalls(sessionId, params.get('after'))
      : await api.calls(params);
    return { calls: page.calls, next: page.next, session };
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
