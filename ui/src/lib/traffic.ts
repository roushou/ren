import type { CallId, CallView } from './api.ts';

/**
 * The call the inspector shows. A finished live observation of the same call
 * wins over the (possibly older) list row; an off-page deep link is used only
 * when the call is not on this page.
 */
export function selectedCall(
  id: CallId | null | undefined,
  calls: CallView[],
  observed?: CallView,
  offPage?: CallView,
): CallView | undefined {
  if (!id) return undefined;
  if (observed?.id === id && observed.state !== 'open') return observed;
  return calls.find((call) => call.id === id) ?? (offPage?.id === id ? offPage : undefined);
}

/** Which calls the listing shows; absent from the URL means model calls only. */
export type KindChoice = 'model' | 'all' | 'other';
/** A start relative to now, kept in the URL as `since=` so it stays relative. */
export type Since = '1h' | 'today' | '24h' | '7d';
export const sinceChoices: { since: Since; label: string }[] = [
  { since: '1h', label: 'Last hour' },
  { since: 'today', label: 'Today' },
  { since: '24h', label: '24 h' },
  { since: '7d', label: '7 days' },
];
export const clientChoices = [
  { client: 'claude_code', label: 'Claude Code' },
  { client: 'codex', label: 'Codex' },
  { client: 'pi', label: 'Pi' },
] as const;

export function parseKind(text: string | null): KindChoice {
  return text === 'all' || text === 'other' ? text : 'model';
}
export function parseSince(text: string | null): Since | null {
  return text === '1h' || text === 'today' || text === '24h' || text === '7d' ? text : null;
}
/** The start of a `since` choice, relative to `now`, in the browser's day. */
export function sinceStart(since: Since, now: number): number {
  switch (since) {
    case '1h':
      return now - 3_600_000;
    case 'today': {
      const date = new Date(now);
      date.setHours(0, 0, 0, 0);
      return date.getTime();
    }
    case '24h':
      return now - 24 * 3_600_000;
    case '7d':
      return now - 7 * 24 * 3_600_000;
  }
}

/**
 * The listing's API query from the page URL: filters pass through, `kind`
 * defaults to model calls, and `since` becomes `from_ms` as of `now`.
 */
export function listingQuery(url: URLSearchParams, keys: readonly string[], now: number) {
  const params = new URLSearchParams();
  for (const key of keys) {
    const value = url.get(key);
    if (value) params.set(key, value);
  }
  const kind = parseKind(url.get('kind'));
  if (kind === 'all') params.delete('kind');
  else params.set('kind', kind);
  const since = parseSince(url.get('since'));
  if (since) params.set('from_ms', String(sinceStart(since, now)));
  return params;
}

export interface Filters {
  q: string;
  session: string;
  client: string;
  kind: KindChoice;
  since: string;
  provider: string;
  model: string;
  tool: string;
  status: string;
  has_tools: boolean;
  from: string;
  until: string;
}
export const emptyFilters = (): Filters => ({
  q: '',
  session: '',
  client: '',
  kind: 'model',
  since: '',
  provider: '',
  model: '',
  tool: '',
  status: '',
  has_tools: false,
  from: '',
  until: '',
});
export function trafficParams(filters: Filters): string {
  const params = new URLSearchParams();
  for (const key of [
    'q',
    'session',
    'client',
    'since',
    'provider',
    'model',
    'tool',
    'status',
  ] as const) {
    const value = filters[key].trim();
    if (value) params.set(key, value);
  }
  if (filters.kind !== 'model') params.set('kind', filters.kind);
  if (filters.has_tools) params.set('has_tools', 'true');
  let from: number | undefined, until: number | undefined;
  for (const key of ['from', 'until'] as const) {
    if (!filters[key]) continue;
    const time = new Date(filters[key]).getTime();
    if (!Number.isFinite(time) || time < 0) throw new Error('Enter a valid time range.');
    params.set(`${key}_ms`, String(time));
    if (key === 'from') from = time;
    else until = time;
  }
  if (from !== undefined && until !== undefined && from > until)
    throw new Error('From must be before Until.');
  return params.toString();
}

export function filtersFromParams(params: URLSearchParams): Filters {
  const filters = emptyFilters();
  for (const key of ['q', 'session', 'client', 'provider', 'model', 'tool', 'status'] as const) {
    filters[key] = params.get(key) ?? '';
  }
  filters.kind = parseKind(params.get('kind'));
  filters.since = parseSince(params.get('since')) ?? '';
  filters.has_tools = params.get('has_tools') === 'true';
  for (const key of ['from', 'until'] as const) {
    const raw = params.get(`${key}_ms`);
    if (!raw) continue;
    const value = new Date(Number(raw));
    if (!Number.isFinite(value.getTime())) continue;
    // datetime-local needs wall time, while URLs retain absolute milliseconds.
    const local = new Date(value.getTime() - value.getTimezoneOffset() * 60000);
    filters[key] = local.toISOString().slice(0, 16);
  }
  return filters;
}

/** A run of calls shown under one heading on a session page. */
export interface TurnGroup {
  id: string;
  label: string;
  rows: CallView[];
}

/**
 * A session's calls grouped by turn, in order. Calls outside any turn that
 * follow one another share one group, so a run of them is one heading, not
 * one per call.
 */
export function turnGroups(calls: CallView[]): TurnGroup[] {
  const groups: TurnGroup[] = [];
  for (const call of calls) {
    const last = groups.at(-1);
    const key = call.turn ?? null;
    if (last && (last.rows[0].turn ?? null) === key) {
      last.rows.push(call);
      continue;
    }
    groups.push({
      // A turn can recur after calls outside it, so a group is keyed by its
      // first call, which is unique.
      id: `${key ?? 'outside'}-${call.id}`,
      label: key ? `Turn ${key.slice(0, 8)}` : 'Outside a turn',
      rows: [call],
    });
  }
  return groups;
}

/** The call `step` places after (1) or before (-1) `current`, staying in range. */
export function neighbour(calls: CallView[], current: CallId | null | undefined, step: 1 | -1) {
  if (!calls.length) return undefined;
  const index = calls.findIndex((call) => call.id === current);
  if (index === -1) return calls[0];
  return calls[Math.min(calls.length - 1, Math.max(0, index + step))];
}
