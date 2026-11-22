// The Overview page's pure logic: each provider's health over the last day,
// the day's failures folded by kind, and the day against the one before.
// No browser needed; tested in tests/overview.test.ts.
import type { CallId, CallView, ModelUsage, UsageView } from './api.ts';
import { num, providerName } from './api.ts';
import { Shares, callStatus, compactOrDash, statusBadge } from './display.ts';
import { Elapsed } from './time.ts';
import type { Stored } from './decode.ts';
import type { Mark } from './marks.ts';
import type { Severity } from './recorder.ts';

const { HOUR, DAY } = Elapsed;

/** One kind of failure in the last day, its repeats folded into a count. */
export interface FailureGroup {
  key: string;
  severity: Severity;
  /** The provider and how the calls ended: `Anthropic 500`, `Codex interrupted`. */
  name: string;
  /** How many, `100+` when the listing was full. */
  count: string;
  /** The models they were for, and `bookkeeping` for calls that are not agents' work. */
  models: string;
  /** When the newest one happened. */
  at: number;
  /** Where each one happened along the day, 0 (a day ago) to 1 (now). */
  marks: number[];
  href: string;
}

/** What the failures are folded from, as the loader read them. */
export interface FailureInput {
  now: number;
  /** Model calls that failed in the last day: an error status, transport, or outcome. */
  failed: CallView[];
  /** Model calls cut off in the last day: cancelled, interrupted, recovered. */
  stopped: CallView[];
  /** Bookkeeping calls that failed: quota probes, token counts, listings. */
  bookkeeping: CallView[];
}

/**
 * The day's failures, worst first: failed work, then cut-off work, then
 * failed bookkeeping, which is not the agents' work and never outranks it;
 * newest first within each.
 */
export class Failures {
  /** Listings are read one page deep; a full page means there may be more. */
  static readonly page = 100;

  static of(input: FailureInput): FailureGroup[] {
    const rank: Record<Severity, number> = { error: 0, warning: 1, neutral: 2 };
    return [
      ...Failures.#fold(input, input.failed, 'error', 'failed'),
      ...Failures.#fold(input, input.stopped, 'warning', 'stopped'),
      ...Failures.#fold(input, input.bookkeeping, 'neutral', 'bookkeeping'),
    ].sort((a, b) => rank[a.severity] - rank[b.severity] || b.at - a.at);
  }

  static count(items: readonly unknown[]): string {
    return `${items.length}${items.length >= Failures.page ? '+' : ''}`;
  }

  static #fold(
    input: FailureInput,
    calls: readonly CallView[],
    severity: Severity,
    kind: 'failed' | 'stopped' | 'bookkeeping',
  ): FailureGroup[] {
    const groups = new Map<string, CallView[]>();
    for (const call of calls) {
      const key = `${kind}:${call.provider}:${Failures.ending(call)}`;
      groups.set(key, [...(groups.get(key) ?? []), call]);
    }
    const status = kind === 'stopped' ? 'interrupted' : 'errors';
    const filter = kind === 'bookkeeping' ? '&kind=other' : '';
    return [...groups].map(([key, group]): FailureGroup => {
      const first = group[0];
      const models = [...new Set(group.flatMap((call) => (call.model ? [call.model] : [])))];
      return {
        key,
        severity,
        name: `${providerName(first.provider)} ${Failures.ending(first)}`,
        count: Failures.count(group),
        models: [...(kind === 'bookkeeping' ? ['bookkeeping'] : []), ...models].join(' · '),
        at: Math.max(...group.map((call) => call.started_at)),
        marks: group.map((call) => Failures.along(call.started_at, input.now)),
        href: `/monitor/traffic?status=${status}${filter}&provider=${encodeURIComponent(first.provider)}&since=24h`,
      };
    });
  }

  /** How a call ended, short: its HTTP status when it has one, else `interrupted`. */
  static ending(call: CallView): string {
    return call.status !== null && call.status >= 400
      ? String(call.status)
      : statusBadge(callStatus(call)).label.toLowerCase();
  }

  /** A time as a fraction of the last day, clamped to it. */
  static along(at: number, now: number): number {
    return Math.min(1, Math.max(0, 1 - (now - at) / DAY));
  }
}

/** How a provider stands: failing now, failing at times, or fine. */
export type ProviderState = 'failing' | 'failures' | 'healthy';

export interface ProviderRow {
  provider: Stored<string>;
  name: string;
  /** Its models, the busiest first. */
  models: string[];
  calls: number;
  failed: number;
  /** Failed against calls, 0–1; null without calls. */
  rate: number | null;
  /** Failed model calls per hour, oldest first, the last ending now. */
  hours: number[];
  /** The newest failure: how it ended and when. */
  last: { ending: string; at: number } | null;
  state: ProviderState;
  /** Its newest finished call, whose response says how much allowance is left. */
  newest: CallId | null;
  href: string;
}

export interface ProviderInput {
  now: number;
  models: readonly ModelUsage[];
  failed: readonly CallView[];
  /** Recent calls of any outcome, for each provider's newest. */
  recent: readonly CallView[];
}

/** Each provider's last day: its calls and failures, and how it stands now. */
export class Providers {
  /** Failures in the last hour that make a provider failing, not just failing at times. */
  static readonly FAILING = 10;

  static rows(input: ProviderInput): ProviderRow[] {
    const byProvider = new Map<string, ModelUsage[]>();
    for (const model of input.models)
      byProvider.set(model.provider, [...(byProvider.get(model.provider) ?? []), model]);
    for (const call of input.failed)
      if (!byProvider.has(call.provider)) byProvider.set(call.provider, []);
    const rows = [...byProvider].map(([provider, models]): ProviderRow => {
      const failures = input.failed.filter((call) => call.provider === provider);
      const calls = models.reduce((sum, model) => sum + model.calls, 0);
      const failed = models.reduce((sum, model) => sum + model.errors, 0);
      const hours = Array.from({ length: 24 }, () => 0);
      for (const call of failures) {
        const back = Math.floor((input.now - call.started_at) / HOUR);
        if (back >= 0 && back < 24) hours[23 - back] += 1;
      }
      const newestFailure = failures.reduce<CallView | null>(
        (best, call) => (!best || call.started_at > best.started_at ? call : best),
        null,
      );
      const newest = input.recent
        .filter((call) => call.provider === provider && call.state !== 'open')
        .reduce<CallView | null>(
          (best, call) => (!best || call.started_at > best.started_at ? call : best),
          newestFailure,
        );
      return {
        provider,
        name: providerName(provider),
        models: models
          .toSorted((a, b) => b.calls - a.calls)
          .flatMap((model) => (model.model ? [model.model] : [])),
        calls,
        failed,
        rate: calls ? failed / calls : null,
        hours,
        last: newestFailure
          ? { ending: Failures.ending(newestFailure), at: newestFailure.started_at }
          : null,
        state: Providers.state(hours),
        newest: newest?.id ?? null,
        href: `/monitor/traffic?provider=${encodeURIComponent(provider)}&since=24h`,
      };
    });
    const rank: Record<ProviderState, number> = { failing: 0, failures: 1, healthy: 2 };
    return rows.sort((a, b) => rank[a.state] - rank[b.state] || b.calls - a.calls);
  }

  /** Failing with many failures in the last hour; failing at times with any. */
  static state(hours: readonly number[]): ProviderState {
    const lastHour = hours.at(-1) ?? 0;
    return lastHour >= Providers.FAILING ? 'failing' : lastHour > 0 ? 'failures' : 'healthy';
  }

  static mark(state: ProviderState): Mark {
    switch (state) {
      case 'failing':
        return 'failed';
      case 'failures':
        return 'warning';
      case 'healthy':
        return 'passed';
      default: {
        const never: never = state;
        return never;
      }
    }
  }
}

/** One of the day's figures against the day before. */
export interface Change {
  label: string;
  value: string;
  /** `+18%`, `+2.4 pt`; null when the day before was not read or had nothing. */
  delta: string | null;
  /** The change is for the worse. */
  worse: boolean;
}

/** The last day against the day before: calls, failure rate, input, cache. */
export class DayChange {
  static rows(day: UsageView, before: UsageView | null): Change[] {
    const share = (part: number | null, whole: number | null) =>
      whole ? (part ?? 0) / whole : null;
    const was = before?.totals ?? null;
    const now = day.totals;
    const rateNow = share(now.errors, now.calls);
    const rateWas = was ? share(was.errors, was.calls) : null;
    const cacheNow = share(now.tokens.cache_read, now.tokens.input);
    const cacheWas = was ? share(was.tokens.cache_read, was.tokens.input) : null;
    return [
      {
        label: 'Calls',
        value: num(now.calls),
        delta: DayChange.percent(now.calls, was?.calls ?? null),
        worse: false,
      },
      {
        label: 'Failure rate',
        value: Shares.percent(rateNow),
        delta: DayChange.points(rateNow, rateWas),
        worse: rateNow !== null && rateWas !== null && rateNow > rateWas,
      },
      {
        label: 'Tokens in',
        value: compactOrDash(now.tokens.input),
        delta: DayChange.percent(now.tokens.input, was?.tokens.input ?? null),
        worse: false,
      },
      {
        label: 'Read from cache',
        value: Shares.percent(cacheNow),
        delta: DayChange.points(cacheNow, cacheWas),
        worse: cacheNow !== null && cacheWas !== null && cacheNow < cacheWas,
      },
    ];
  }

  /** The change as a percentage of the day before; null without one to compare. */
  static percent(value: number | null, before: number | null): string | null {
    if (value === null || !before) return null;
    const change = Math.round(((value - before) / before) * 100);
    return `${change > 0 ? '+' : change < 0 ? '−' : '±'}${Math.abs(change)}%`;
  }

  /** The change between two shares in percentage points. */
  static points(value: number | null, before: number | null): string | null {
    if (value === null || before === null) return null;
    const change = Math.round((value - before) * 1000) / 10;
    return `${change > 0 ? '+' : change < 0 ? '−' : '±'}${Math.abs(change)} pt`;
  }
}
