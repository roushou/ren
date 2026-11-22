import { Clock } from './time.ts';
import { harnesses, title, type CallId, type CallView, type Provider } from './api.ts';
import { harnessLabel } from './activity.ts';
import { changed, picked, type Query } from './query.ts';
import type { RawCallQuery } from './wire/RawCallQuery.ts';

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
export const sinceChoices: { since: Since; label: string; title: string }[] = [
  { since: '1h', label: '1h', title: 'Last hour' },
  { since: 'today', label: 'Today', title: 'Since midnight' },
  { since: '24h', label: '24h', title: 'Last 24 hours' },
  { since: '7d', label: '7d', title: 'Last 7 days' },
];
export const clientChoices = harnesses.map((client) => ({ client, label: harnessLabel(client) }));

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
export function listingQuery(
  url: URLSearchParams,
  keys: readonly (keyof RawCallQuery & string)[],
  now: number,
): Query<RawCallQuery> {
  const kind = parseKind(url.get('kind'));
  const since = parseSince(url.get('since'));
  return changed(picked<RawCallQuery>(url, keys), {
    kind: kind === 'all' ? null : kind,
    ...(since ? { from_ms: sinceStart(since, now) } : {}),
  });
}

export interface Filters {
  q: string;
  recording: string;
  client: string;
  /** A named user of the gateway, not a harness. */
  user: string;
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
export const Filters = {
  /** `filters` without the narrowing `filter` applies; an unknown one changes nothing. */
  without(filters: Filters, filter: AppliedFilter): Filters {
    if (filter.type === 'unknown') return filters;
    const key = filter.key;
    switch (key) {
      case 'from_ms':
        return { ...filters, from: '' };
      case 'until_ms':
        return { ...filters, until: '' };
      case 'has_tools':
        return { ...filters, has_tools: false };
      case 'kind':
        return { ...filters, kind: 'model' };
      case 'q':
      case 'recording':
      case 'client':
      case 'user':
      case 'since':
      case 'provider':
      case 'model':
      case 'tool':
      case 'status':
        return { ...filters, [key]: '' };
      default: {
        const never: never = key;
        return never;
      }
    }
  },
};
export const emptyFilters = (): Filters => ({
  q: '',
  recording: '',
  client: '',
  user: '',
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
/** The filters written as text, each its own URL parameter. */
export const TEXT_FILTERS = [
  'q',
  'recording',
  'client',
  'user',
  'since',
  'provider',
  'model',
  'tool',
  'status',
] as const;
export type TextFilter = (typeof TEXT_FILTERS)[number];

/** A call's state, as the listing's `status=` filter names it. */
export type StatusChoice = 'active' | 'finished' | 'errors' | 'interrupted';
/** The provider picker's choices; the empty value is every provider. */
export const providerChoices: readonly { value: Provider | ''; label: string }[] = [
  { value: '', label: 'All providers' },
  { value: 'anthropic', label: 'Anthropic' },
  { value: 'openai', label: 'OpenAI' },
  { value: 'codex', label: 'Codex' },
  { value: 'deepseek', label: 'DeepSeek' },
];
/** The status picker's choices; the empty value is every status. */
export const statusChoices: readonly { value: StatusChoice | ''; label: string }[] = [
  { value: '', label: 'All statuses' },
  { value: 'active', label: 'In progress' },
  { value: 'finished', label: 'Finished' },
  { value: 'errors', label: 'Errors' },
  { value: 'interrupted', label: 'Interrupted / cancelled' },
];

/** A narrowing the listing's URL applies, named by its query parameter. */
export type FilterKey = TextFilter | 'kind' | 'has_tools' | 'from_ms' | 'until_ms';
const FILTER_KEYS: readonly FilterKey[] = [
  ...TEXT_FILTERS,
  'kind',
  'has_tools',
  'from_ms',
  'until_ms',
];
export const FilterKey = {
  /** The filter a query parameter names, or null for one this view does not know. */
  parse(raw: string): FilterKey | null {
    return FILTER_KEYS.find((key) => key === raw) ?? null;
  },
};

/** What naming an applied filter needs from the page. */
export interface FilterContext {
  /** A session as the operator recognises it, from its recording ID. */
  session(id: string): string;
}

/** One parameter of the applied query: a filter this view knows, or one kept as it came. */
export type AppliedFilter =
  | { type: 'filter'; key: FilterKey; value: string }
  | { type: 'unknown'; key: string; value: string };
export const AppliedFilter = {
  /** Every parameter of `params`, in its order. */
  list(params: URLSearchParams): AppliedFilter[] {
    return [...params.entries()].map(([raw, value]): AppliedFilter => {
      const key = FilterKey.parse(raw);
      return key ? { type: 'filter', key, value } : { type: 'unknown', key: raw, value };
    });
  },
  /** How a chip names the filter. */
  label(filter: AppliedFilter, context: FilterContext): string {
    if (filter.type === 'unknown') return `${title(filter.key)}: ${filter.value}`;
    const { key, value } = filter;
    switch (key) {
      case 'q':
        return `Search: ${value}`;
      case 'recording':
        return `Session: ${context.session(value)}`;
      case 'user':
        return `User: ${value}`;
      case 'client':
        return `Client: ${clientChoices.find((item) => item.client === value)?.label ?? value}`;
      case 'since': {
        const since = parseSince(value);
        return sinceChoices.find((item) => item.since === since)?.title ?? `Since: ${value}`;
      }
      case 'kind':
        return parseKind(value) === 'other' ? 'Bookkeeping calls only' : 'All call kinds';
      case 'from_ms':
      case 'until_ms':
        return `${key === 'from_ms' ? 'From' : 'Until'}: ${new Date(Number(value)).toLocaleString([], { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' })}`;
      case 'has_tools':
        return 'Has tool calls';
      case 'provider':
      case 'model':
      case 'tool':
      case 'status':
        return `${title(key)}: ${value}`;
      default: {
        const never: never = key;
        return never;
      }
    }
  },
};

/** The quick narrowings above the listing: by status, or to calls that asked for tools. */
export type QuickFilter = 'all' | 'active' | 'errors' | 'tools';
export const QuickFilter = {
  /** Whether the applied `filters` are what `quick` narrows to. */
  chosen(quick: QuickFilter, filters: Filters): boolean {
    switch (quick) {
      case 'all':
        return !filters.status && !filters.has_tools;
      case 'active':
      case 'errors':
        return filters.status === quick;
      case 'tools':
        return filters.has_tools;
      default: {
        const never: never = quick;
        return never;
      }
    }
  },
  /** `filters` narrowed to `quick`: its status and tool narrowing replaced. */
  apply(quick: QuickFilter, filters: Filters): Filters {
    switch (quick) {
      case 'all':
        return { ...filters, status: '', has_tools: false };
      case 'active':
      case 'errors':
        return { ...filters, status: quick, has_tools: false };
      case 'tools':
        return { ...filters, status: '', has_tools: true };
      default: {
        const never: never = quick;
        return never;
      }
    }
  },
};

export function trafficParams(filters: Filters): string {
  const params = new URLSearchParams();
  for (const key of TEXT_FILTERS) {
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
  for (const key of TEXT_FILTERS) {
    // `since` is parsed below.
    if (key !== 'since') filters[key] = params.get(key) ?? '';
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

/** A run of calls shown under one heading on a recording page. */
export interface TurnGroup {
  id: string;
  label: string;
  /** The turn the group's calls belong to; null outside a turn. */
  turn: CallView['turn'];
  rows: CallView[];
}

/**
 * A recording's calls grouped by turn, in order. Calls outside any turn that
 * follow one another share one group, so a run of them is one heading, not
 * one per call.
 */
export function turnGroups(calls: CallView[]): TurnGroup[] {
  const groups: TurnGroup[] = [];
  // Turns are numbered in the order they first appear on the page.
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
      // A turn is named by when it started: its first call here.
      label: key ? `Turn · ${Clock.second(call.started_at)}` : 'Outside a turn',
      turn: key,
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
