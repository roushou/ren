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

export interface Filters {
  q: string;
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
  for (const key of ['q', 'provider', 'model', 'tool', 'status'] as const) {
    const value = filters[key].trim();
    if (value) params.set(key, value);
  }
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
  for (const key of ['q', 'provider', 'model', 'tool', 'status'] as const) {
    filters[key] = params.get(key) ?? '';
  }
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
