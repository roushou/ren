// The Usage page's pure logic: the range, the time window it reads, the
// contiguous buckets a chart draws, and clean axis ticks. No browser needed.
import type { UsageBucket, UsageCounts } from './api.ts';

/** What the page shows: the last day by hour, the last week by day, or all retained calls by day. */
export type UsageRange = '24h' | '7d' | 'all';
export const usageRanges: { range: UsageRange; label: string }[] = [
  { range: '24h', label: '24 hours' },
  { range: '7d', label: '7 days' },
  { range: 'all', label: 'All retained' },
];

/** `?range=`; anything else is the default, a week. */
export function parseRange(text: string | null): UsageRange {
  switch (text) {
    case '24h':
    case '7d':
    case 'all':
      return text;
    default:
      return '7d';
  }
}

const HOUR = 3_600_000;
const DAY = 24 * HOUR;

/** The start of a range, relative to `now`; none for everything retained. */
export function rangeStart(range: UsageRange, now: number, dayStart: (ms: number) => number) {
  switch (range) {
    case '24h':
      return Math.floor(now / HOUR) * HOUR - 23 * HOUR;
    case '7d':
      return dayStart(now - 6 * DAY);
    case 'all':
      return undefined;
  }
}

/** Midnight before `ms`, in the browser's time zone. */
export function localDayStart(ms: number): number {
  const date = new Date(ms);
  date.setHours(0, 0, 0, 0);
  return date.getTime();
}

/** One bar: its period and what happened in it. */
export interface UsagePoint extends UsageCounts {
  start: number;
}

const empty = (start: number): UsagePoint => ({
  start,
  calls: 0,
  errors: 0,
  tokens: { input: null, output: null, cache_read: null },
});

const add = (a: number | null, b: number | null) =>
  a === null && b === null ? null : (a ?? 0) + (b ?? 0);

/**
 * The bars for a range: every hour of the last day, or every day of the
 * range, including empty ones, so time reads evenly. `hours` comes from the
 * API by UTC hour; days are local, so hours are folded into the local day
 * they start in.
 */
export function usagePoints(
  hours: UsageBucket[],
  range: UsageRange,
  now: number,
  dayStart: (ms: number) => number = localDayStart,
): UsagePoint[] {
  const byHour = range === '24h';
  const key = (ms: number) => (byHour ? Math.floor(ms / HOUR) * HOUR : dayStart(ms));
  const first = rangeStart(range, now, dayStart) ?? (hours.length ? key(hours[0].start) : key(now));
  const last = key(now);
  const points = new Map<number, UsagePoint>();
  // Step by a day and re-anchor, so a daylight-saving change still lands on midnight.
  for (let at = first; at <= last; at = byHour ? at + HOUR : dayStart(at + DAY + HOUR)) {
    points.set(at, empty(at));
    if (points.size > 400) break;
  }
  for (const hour of hours) {
    const point = points.get(key(hour.start));
    if (!point) continue;
    point.calls += hour.calls;
    point.errors += hour.errors;
    point.tokens = {
      input: add(point.tokens.input, hour.tokens.input),
      output: add(point.tokens.output, hour.tokens.output),
      cache_read: add(point.tokens.cache_read, hour.tokens.cache_read),
    };
  }
  return [...points.values()];
}

/** What the chart plots. */
export type UsageMeasure = 'input' | 'output' | 'calls' | 'errors';
export const usageMeasures: { measure: UsageMeasure; label: string }[] = [
  { measure: 'input', label: 'Tokens in' },
  { measure: 'output', label: 'Tokens out' },
  { measure: 'calls', label: 'Calls' },
  { measure: 'errors', label: 'Errors' },
];

export function measureValue(counts: UsageCounts, measure: UsageMeasure): number {
  switch (measure) {
    case 'input':
      return counts.tokens.input ?? 0;
    case 'output':
      return counts.tokens.output ?? 0;
    case 'calls':
      return counts.calls;
    case 'errors':
      return counts.errors;
  }
}

/** Gridline values from 0 to a round top at or above `max`: 1, 2, 2.5, or 5 times a power of ten. */
export function axisTicks(max: number, count = 4): number[] {
  if (max <= 0) return [0, 1];
  const rough = max / count;
  const power = 10 ** Math.floor(Math.log10(rough));
  const step = ([1, 2, 2.5, 5, 10].find((m) => m * power >= rough) ?? 10) * power;
  const ticks: number[] = [];
  for (let value = 0; value < max + step; value += step) {
    ticks.push(value);
    if (value >= max) break;
  }
  return ticks;
}

/** The share of input read from cache, 0..1; null when unknown. */
export function cacheShare(counts: UsageCounts): number | null {
  const { input, cache_read } = counts.tokens;
  return input && cache_read !== null ? cache_read / input : null;
}
