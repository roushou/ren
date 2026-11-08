// The overview page's pure logic: the health line, what needs attention,
// and which sessions are active. No browser needed.
import type { CallView, ExposureView, FileUsage, Health, Storage, UsageView } from './api.ts';
import { bytes, num, providerName } from './api.ts';
import { searchFieldLabel, secretKindLabel } from './activity.ts';
import { buildLabel, callStatus, compactOrDash, statusBadge, type Chip } from './display.ts';
import { usagePoints, type UsagePoint } from './usage.ts';

/** A health chip and where to look when it is not fine. */
export interface HealthItem extends Chip {
  href?: string;
}

const live = (file: FileUsage) =>
  Math.max(0, file.page_count - file.freelist_count) * file.page_size;

/**
 * One line that says whether everything is fine: the daemon is reachable,
 * recording works, calls are admitted, and storage has room. Each item is
 * green when fine and says what is wrong when not.
 */
export function healthLine(
  health: Health | null,
  storage: Storage | null,
  error: string | null,
): HealthItem[] {
  if (!health)
    return [
      {
        label: "Can't reach TGV",
        tone: 'error',
        title: error ?? 'The daemon did not answer.',
      },
    ];
  const items: HealthItem[] = [
    { label: 'Daemon reachable', tone: 'success', title: buildLabel(health.build) },
  ];
  const recorder = health.recorder;
  const lost =
    recorder.omissions.unrecorded_calls +
    recorder.omissions.queue_bytes_bytes +
    recorder.omissions.queue_slots_bytes +
    recorder.omissions.storage_failure_bytes;
  items.push(
    health.status !== 'ok' || recorder.storage_errors || recorder.retention_errors
      ? {
          label: 'Recording failing',
          tone: 'error',
          title: 'Storage or retention errors since the daemon started',
          href: '/recorder',
        }
      : lost
        ? {
            label: 'Capture loss',
            tone: 'warning',
            title: 'Some calls or body bytes were not recorded since the daemon started',
            href: '/recorder',
          }
        : {
            label: 'Recording',
            tone: 'success',
            title: 'No capture loss since the daemon started',
          },
  );
  const operator = health.operator;
  const pause = operator.pause;
  switch (pause.state) {
    case 'running':
      items.push({ label: 'Admitting calls', tone: 'success', title: 'Not paused' });
      break;
    case 'paused':
      items.push({
        label: `Paused · retry in ${pause.retry_after} s`,
        tone: 'warning',
        title: 'Every new call gets 503; `tgv control resume` admits them again',
      });
      break;
    case 'unknown':
      items.push({
        label: pause.text,
        tone: 'neutral',
        title: 'An admission state this UI does not know',
      });
      break;
  }
  if (operator.blocked_sessions)
    items.push({
      label: `${operator.blocked_sessions} session${operator.blocked_sessions === 1 ? '' : 's'} blocked`,
      tone: 'warning',
      title: 'Calls from blocked sessions get 403; `tgv control unblock` lifts a block',
    });
  if (storage) {
    const used = live(storage.usage.disk.metadata) + live(storage.usage.disk.blobs);
    const budget = storage.policy.max_storage_bytes;
    const share = storage.policy.enabled && budget ? used / budget : null;
    items.push(
      share === null
        ? {
            label: `${bytes(used)} stored`,
            tone: 'neutral',
            title: 'Retention is off: no storage limit',
            href: '/recorder',
          }
        : {
            label: `${bytes(used)} of ${bytes(budget)}`,
            tone: share >= 0.9 ? 'error' : share >= 0.7 ? 'warning' : 'success',
            title:
              'Live storage against the retention budget; the oldest calls are removed past it',
            href: '/recorder',
          },
    );
  }
  return items;
}

/** `just now`, `45 s ago`, `12 min ago`, `3 h ago`. */
export function ago(ms: number, now: number): string {
  const seconds = Math.max(0, Math.round((now - ms) / 1000));
  if (seconds < 5) return 'just now';
  if (seconds < 60) return `${seconds} s ago`;
  const minutes = Math.round(seconds / 60);
  if (minutes < 60) return `${minutes} min ago`;
  return `${Math.round(minutes / 60)} h ago`;
}

/** How much a line of the overview needs you. */
export type Severity = 'error' | 'warning' | 'neutral';

/**
 * One kind of trouble in the last day, with its repeats folded into a count
 * and their times kept as marks along the day.
 */
export interface AttentionGroup {
  key: string;
  severity: Severity;
  title: string;
  detail: string;
  /** When the newest one happened; null for counts since the daemon started. */
  at: number | null;
  /** Where each one happened along the day, 0 (a day ago) to 1 (now). */
  marks: number[];
  href: string;
}

/** What the overview reads, as the loader returns it. */
export interface OverviewInput {
  now: number;
  health: Health | null;
  /** Model calls that failed in the last day: an error status, transport, or outcome. */
  failed: CallView[];
  /** Model calls cut off in the last day: cancelled, interrupted, recovered. */
  stopped: CallView[];
  /** Bookkeeping calls that failed: quota probes, token counts, listings. */
  bookkeeping: CallView[];
  /** Credentials found in the last day; null from a daemon without them. */
  exposures: ExposureView[] | null;
}

/**
 * What needs attention, worst first: exposed secrets, recording trouble,
 * failed and cut-off work, then bookkeeping that failed, which is not the
 * agents' work and never outranks it. Repeats fold into one line each.
 */
export class Attention {
  static readonly dayMs = 24 * 3_600_000;
  /** Listings are read one page deep; a full page means there may be more. */
  static readonly page = 100;

  readonly groups: AttentionGroup[];

  constructor(input: OverviewInput) {
    const groups = [
      ...Attention.secrets(input),
      ...Attention.recording(input.health),
      ...Attention.calls(input, input.failed, 'error', 'failed'),
      ...Attention.calls(input, input.stopped, 'warning', 'stopped'),
      ...Attention.calls(input, input.bookkeeping, 'neutral', 'bookkeeping'),
    ];
    const rank: Record<Severity, number> = { error: 0, warning: 1, neutral: 2 };
    this.groups = groups.sort(
      (a, b) => rank[a.severity] - rank[b.severity] || (b.at ?? Infinity) - (a.at ?? Infinity),
    );
  }

  /** The worst severity present, or null when nothing needs a look. */
  get worst(): Severity | null {
    return this.groups[0]?.severity ?? null;
  }

  static count(items: unknown[]): string {
    return `${items.length}${items.length >= Attention.page ? '+' : ''}`;
  }

  static secrets(input: OverviewInput): AttentionGroup[] {
    const found = input.exposures ?? [];
    const newest = found[0];
    if (!newest) return [];
    const sessions = new Set(found.map((exposure) => exposure.session ?? exposure.call));
    const kinds = [...new Set(found.map((exposure) => secretKindLabel(exposure.kind)))];
    const fields = [...new Set(found.map((exposure) => searchFieldLabel(exposure.field)))];
    const only = sessions.size === 1 && newest.session;
    return [
      {
        key: 'secrets',
        severity: 'error',
        title: `${Attention.count(found)} secret${found.length === 1 ? '' : 's'} exposed`,
        detail: `${kinds.join(', ')} in ${fields.join(', ').toLowerCase()} · ${sessions.size} session${sessions.size === 1 ? '' : 's'} · rotate ${found.length === 1 ? 'it' : 'them'}`,
        at: newest.started_at,
        marks: found.map((exposure) => Attention.along(exposure.started_at, input.now)),
        href: only
          ? `/sessions/${encodeURIComponent(only)}/report`
          : `/traffic?call=${encodeURIComponent(newest.call)}`,
      },
    ];
  }

  /**
   * Recording trouble since the daemon started. Bytes left out because a
   * body passed the capture policy's limit are left out by design, so they
   * are noted, never raised.
   */
  static recording(health: Health | null): AttentionGroup[] {
    if (!health) return [];
    const recorder = health.recorder;
    const omissions = recorder.omissions;
    const groups: AttentionGroup[] = [];
    const base = { at: null, marks: [], href: '/recorder' };
    if (recorder.storage_errors || recorder.retention_errors)
      groups.push({
        ...base,
        key: 'recording',
        severity: 'error',
        title: 'Recording failing',
        detail: `${num(recorder.storage_errors)} storage and ${num(recorder.retention_errors)} retention errors since the daemon started`,
      });
    const lost =
      omissions.queue_bytes_bytes + omissions.queue_slots_bytes + omissions.storage_failure_bytes;
    if (lost || omissions.unrecorded_calls)
      groups.push({
        ...base,
        key: 'capture-loss',
        severity: 'warning',
        title: omissions.unrecorded_calls
          ? `${num(omissions.unrecorded_calls)} calls and ${bytes(lost)} not captured`
          : `${bytes(lost)} not captured`,
        detail:
          'The capture queue was full or storage failed, since the daemon started; forwarding was unaffected',
      });
    if (omissions.policy_limit_bytes)
      groups.push({
        ...base,
        key: 'capture-policy',
        severity: 'neutral',
        title: `${bytes(omissions.policy_limit_bytes)} over the capture limit`,
        detail: 'Body bytes past the capture policy, left out by design, since the daemon started',
      });
    return groups;
  }

  /** Calls folded by provider and how they ended, newest group first. */
  static calls(
    input: OverviewInput,
    calls: CallView[],
    severity: Severity,
    kind: 'failed' | 'stopped' | 'bookkeeping',
  ): AttentionGroup[] {
    const groups = new Map<string, CallView[]>();
    for (const call of calls) {
      const label = Attention.ending(call);
      const key = `${kind}:${call.provider}:${label}`;
      groups.set(key, [...(groups.get(key) ?? []), call]);
    }
    const status = kind === 'stopped' ? 'interrupted' : 'errors';
    const filter = kind === 'bookkeeping' ? '&kind=other' : '';
    return [...groups].map(([key, group]): AttentionGroup => {
      const first = group[0];
      const models = [...new Set(group.flatMap((call) => (call.model ? [call.model] : [])))];
      const what =
        kind === 'bookkeeping'
          ? "bookkeeping (quota probes, token counts), not your agents' work"
          : kind === 'stopped'
            ? 'model calls cut off before they finished'
            : 'model calls that failed';
      return {
        key,
        severity,
        title: `${first ? providerName(first.provider) : ''} ${first ? Attention.ending(first) : ''} ×${Attention.count(group)}`,
        detail: models.length ? `${what} · ${models.join(', ')}` : what,
        at: Math.max(...group.map((call) => call.started_at)),
        marks: group.map((call) => Attention.along(call.started_at, input.now)),
        href: `/traffic?status=${status}${filter}&since=24h`,
      };
    });
  }

  /** How a call ended, as its row badge says it: `Error 429`, `Interrupted`. */
  static ending(call: CallView): string {
    const badge = statusBadge(callStatus(call));
    return call.status !== null && call.status >= 400
      ? `${badge.label} ${call.status}`
      : badge.label;
  }

  /** A time as a fraction of the last day, clamped to it. */
  static along(at: number, now: number): number {
    return Math.min(1, Math.max(0, 1 - (now - at) / Attention.dayMs));
  }
}

/** A number tile: its value, a line under it, and the hours behind it. */
export interface DayTile {
  label: string;
  value: string;
  detail: string;
  /** The hourly series for its sparkline, oldest first; empty for none. */
  spark: number[];
  tone: 'error' | null;
}

/**
 * The last day in numbers and hours: calls, failed work, tokens in with
 * their cached share, tokens out. A day before it, when read, gives the
 * change in calls.
 */
export class Day {
  readonly points: UsagePoint[];
  readonly tiles: DayTile[];
  /** Input tokens read from cache and sent fresh; null when not reported. */
  readonly input: { cached: number; fresh: number } | null;

  constructor(
    usage: UsageView,
    previous: UsageView | null,
    now: number,
    counts: { bookkeeping: number; stopped: number },
  ) {
    this.points = usagePoints(usage.hours, '24h', now);
    const totals = usage.totals;
    const input = totals.tokens.input;
    this.input =
      input === null
        ? null
        : {
            cached: Math.min(input, totals.tokens.cache_read ?? 0),
            fresh: Math.max(0, input - (totals.tokens.cache_read ?? 0)),
          };
    const before = previous?.totals.calls ?? null;
    const peak = this.points.reduce<UsagePoint | null>(
      (best, point) => ((point.tokens.output ?? 0) > (best?.tokens.output ?? 0) ? point : best),
      null,
    );
    this.tiles = [
      {
        label: 'Calls',
        value: num(totals.calls),
        detail: Day.change(totals.calls, before),
        spark: this.points.map((point) => point.calls),
        tone: null,
      },
      {
        label: 'Failed work calls',
        value: num(totals.errors),
        detail: [
          `${num(counts.bookkeeping)} failed bookkeeping`,
          `${num(counts.stopped)} cut off`,
        ].join(' · '),
        spark: this.points.map((point) => point.errors),
        tone: totals.errors ? 'error' : null,
      },
      {
        label: 'Tokens in',
        value: compactOrDash(input),
        detail: this.input
          ? `${compactOrDash(this.input.cached)} cached · ${compactOrDash(this.input.fresh)} fresh`
          : 'not reported',
        spark: [],
        tone: null,
      },
      {
        label: 'Tokens out',
        value: compactOrDash(totals.tokens.output),
        detail: peak?.tokens.output
          ? `peak ${compactOrDash(peak.tokens.output)} at ${Day.clock(peak.start)}`
          : 'none reported',
        spark: this.points.map((point) => point.tokens.output ?? 0),
        tone: null,
      },
    ];
  }

  /** Below this many calls, the day before is shown as its count, not a percentage. */
  static readonly smallDay = 50;

  /** `+38% vs the day before`, or why there is no comparison. */
  static change(value: number, before: number | null): string {
    if (before === null) return 'no earlier day read';
    if (before === 0) return value ? 'none the day before' : 'none the day before either';
    // A percentage over a handful of calls says more about the handful than the change.
    if (before < Day.smallDay) return `${num(before)} the day before`;
    const change = Math.round(((value - before) / before) * 100);
    return `${change > 0 ? '+' : ''}${change}% vs the day before`;
  }

  static clock(ms: number): string {
    return new Date(ms).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
  }

  /** A sparkline's points in a 100 × `height` box, flat when nothing happened. */
  static spark(values: number[], height: number): string {
    const max = Math.max(0, ...values);
    const step = 100 / Math.max(1, values.length - 1);
    return values
      .map((value, index) => {
        const y = max ? height - 1 - (value / max) * (height - 2) : height - 1;
        return `${(index * step).toFixed(1)},${y.toFixed(1)}`;
      })
      .join(' ');
  }

  /** The chart's time span: the first hour's start to the end of the current one. */
  get span(): { from: number; until: number } {
    const from = this.points[0]?.start ?? 0;
    return { from, until: (this.points.at(-1)?.start ?? from) + 3_600_000 };
  }

  /** A time as x in a 1000-wide box over the chart's span, clamped to it. */
  x(at: number): number {
    const { from, until } = this.span;
    return Math.min(1000, Math.max(0, ((at - from) / Math.max(1, until - from)) * 1000));
  }

  /** Labels along the axis: every sixth hour's clock time, oldest first. */
  axis(): { at: number; label: string }[] {
    return this.points
      .filter((_, index) => index % 6 === 0)
      .map((point) => ({ at: point.start, label: Day.clock(point.start) }));
  }

  /**
   * The day's hours as stacked bars in a 1000 × `height` box: input read
   * from cache at the bottom, fresh input above it.
   */
  bars(
    height: number,
  ): { start: number; x: number; width: number; cached: number; fresh: number }[] {
    const max = Math.max(1, ...this.points.map((point) => point.tokens.input ?? 0));
    return this.points.map((point) => {
      const input = point.tokens.input ?? 0;
      const cached = Math.min(input, point.tokens.cache_read ?? 0);
      const x = this.x(point.start);
      return {
        start: point.start,
        x: x + 1,
        width: Math.max(1, this.x(point.start + 3_600_000) - x - 2),
        cached: (cached / max) * height,
        fresh: ((input - cached) / max) * height,
      };
    });
  }
}
