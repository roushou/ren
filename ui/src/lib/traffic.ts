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
