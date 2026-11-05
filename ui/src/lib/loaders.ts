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
} from './api.ts';
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
  selection: OffPageSelection;
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
    const health = await api.health();
    const storage = await api.storage();
    return { health, storage };
  });
}
export function recorderSnapshot(fetcher: typeof fetch) {
  const api = new ManagementApi(fetcher);
  return readSnapshot('recorder', (): Promise<Metrics> => api.metrics());
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
  const rawCall = url.searchParams.get('call');
  const callId = CallId.parse(rawCall);
  const key = `${url.pathname}?${params}`;
  return readSnapshot(key, async (): Promise<TrafficData> => {
    // The bounded daemon reader is shared with body previews: query serially.
    let session: SessionView | undefined;
    if (sessionId) {
      session = (await api.sessions(100)).sessions.find((item) => item.id === sessionId);
    }
    const page = sessionId
      ? await api.sessionCalls(sessionId, params.get('after'))
      : await api.calls(params);
    let selection: OffPageSelection = { kind: 'none' };
    if (rawCall && !callId)
      selection = { kind: 'failed', message: 'The linked call ID is invalid.' };
    else if (callId && !page.calls.some((call) => call.id === callId)) {
      try {
        selection = { kind: 'loaded', call: (await api.call(callId)).call };
      } catch (error) {
        selection = {
          kind: 'failed',
          message: snapshotError(error, 'Unable to load the selected call.').message,
        };
      }
    }
    return { calls: page.calls, next: page.next, session, selection };
  });
}
