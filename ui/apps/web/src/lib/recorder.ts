// The Recorder page's pure logic: whether recording is healthy, how close
// retention is to pruning, and which counters need a look. The layout's tab
// marker and footer, and the overview, read capture health through it too.
// No browser, no fetching; tested in tests/recorder.test.ts.
import { Elapsed } from './time.ts';
import type { FileUsage, RecorderMetrics, RetentionPolicy, StorageView } from './api.ts';
import { bytes, num } from './api.ts';
import { Shares } from './display.ts';
import { assertNever } from './decode.ts';

/**
 * Recording as its counters since startup show it. Bytes past the capture
 * policy, and WebSocket bytes between calls, are left out by design: they
 * are never loss.
 */
export type CaptureHealth =
  | { kind: 'failing'; storageErrors: number; retentionErrors: number }
  | { kind: 'lost'; calls: number; bytes: number }
  | { kind: 'healthy' };

/** How much a counter, or a line of the overview, needs a look. */
export type Severity = 'error' | 'warning' | 'neutral';

/** How a verdict line reads: something is wrong, needs a look, or is fine. */
export type VerdictTone = 'error' | 'warning' | 'success';

/** A counter since startup that is not zero, and what it means. */
export interface CounterNote {
  label: string;
  value: string;
  severity: Severity;
  detail: string;
}

/** One figure of how recording stands, toned by its own limits. */
export interface Indicator {
  key: 'lost' | 'unrecorded' | 'errors' | 'queue' | 'commit' | 'lag';
  label: string;
  value: string;
  tone: VerdictTone;
  /** How full it is, 0–1, for a figure with a capacity; else null. */
  share: number | null;
}

/** One retention limit, and how much of it is used. */
export interface Gauge {
  key: 'storage' | 'calls' | 'history';
  label: string;
  used: string;
  limit: string;
  /** Used over limit, 0–1 and past it; null when unknown or unlimited. */
  share: number | null;
  /** The limit closest to its end: retention prunes for it first. */
  binding: boolean;
}

/** One step of what a request or response body becomes on disk, with the
 * ratio it saves on the step before it. */
export interface BodyStep {
  label: string;
  bytes: number;
  note: { label: string; ratio: string } | null;
  tone: 'referenced' | 'unique' | 'compressed';
}

/** One part of the database files' bytes on disk. */
export interface DiskPart {
  key: 'meta-live' | 'meta-free' | 'blob-live' | 'blob-free';
  label: string;
  bytes: number;
}

/** One of the recorder queue's two limits, and how much of it is used. */
export interface QueueMeter {
  label: string;
  /** Used over capacity, 0–1 and past it; 0 for no capacity. */
  share: number;
  text: string;
}

export class RecorderReport {
  readonly health: CaptureHealth;
  /** Non-zero loss, error, and by-design counters, worst first. */
  readonly notes: CounterNote[];
  /** Loss and error counters at zero: folded into one line. */
  readonly quiet: string[];
  readonly gauges: Gauge[];

  readonly recorder: RecorderMetrics;

  constructor(
    recorder: RecorderMetrics,
    storage: StorageView | null,
    policy: RetentionPolicy | null,
    /** When the oldest retained call started; null when none is retained or unread. */
    oldest: number | null,
    now: number,
  ) {
    this.recorder = recorder;
    this.health = RecorderReport.capture(recorder);
    const counters = RecorderReport.counters(recorder);
    const rank: Record<Severity, number> = { error: 0, warning: 1, neutral: 2 };
    this.notes = counters
      .filter((counter) => counter.amount > 0)
      .sort((a, b) => rank[a.note.severity] - rank[b.note.severity])
      .map((counter) => counter.note);
    this.quiet = counters
      .filter((counter) => counter.amount === 0 && counter.note.severity !== 'neutral')
      .map((counter) => counter.note.label);
    this.gauges = RecorderReport.gauges(storage, policy, oldest, now);
  }

  /** Storage used past this share of its limit needs a look. */
  static readonly CROWDED = 0.8;

  /** Storage, when it is close enough to its limit to need a look. */
  get crowded(): Gauge | null {
    const storage = this.gauges.find((gauge) => gauge.key === 'storage');
    return storage?.share != null && storage.share > RecorderReport.CROWDED ? storage : null;
  }

  /** Bytes the recorder could not keep: the queue was full or storage failed. */
  static lostBytes(recorder: RecorderMetrics): number {
    const omissions = recorder.omissions;
    return (
      omissions.queue_bytes_bytes + omissions.queue_slots_bytes + omissions.storage_failure_bytes
    );
  }

  static capture(recorder: RecorderMetrics): CaptureHealth {
    if (recorder.storage_errors || recorder.retention_errors)
      return {
        kind: 'failing',
        storageErrors: recorder.storage_errors,
        retentionErrors: recorder.retention_errors,
      };
    const lost = RecorderReport.lostBytes(recorder);
    const calls = recorder.omissions.unrecorded_calls;
    return lost || calls ? { kind: 'lost', calls, bytes: lost } : { kind: 'healthy' };
  }

  /** What went wrong, in a few words: `2 calls and 4.0 KiB not captured`. */
  static lossText(health: Extract<CaptureHealth, { kind: 'lost' }>): string {
    const calls = `${num(health.calls)} call${health.calls === 1 ? '' : 's'}`;
    if (health.calls && health.bytes) return `${calls} and ${bytes(health.bytes)} not captured`;
    return health.calls ? `${calls} not recorded` : `${bytes(health.bytes)} not captured`;
  }

  /** Queue use past these shares needs a look, then is trouble. */
  static readonly QUEUE_WARN = 0.7;
  static readonly QUEUE_ERROR = 0.9;

  /** How full something with a capacity is: fine, past the queue's warning
   * share, or past its error share. Retention limits read the same way. */
  static fullness(share: number): 'fine' | 'warning' | 'error' {
    if (share >= RecorderReport.QUEUE_ERROR) return 'error';
    if (share >= RecorderReport.QUEUE_WARN) return 'warning';
    return 'fine';
  }
  /** A commit slower than these, in ms, needs a look, then is trouble. */
  static readonly COMMIT_WARN_MS = 250;
  static readonly COMMIT_ERROR_MS = 1000;
  /** Recording lag past these, in ms, needs a look, then is trouble. */
  static readonly LAG_WARN_MS = 1000;
  static readonly LAG_ERROR_MS = 5000;

  /** How recording stands: its tone and the state it names. */
  get verdict(): { tone: VerdictTone; headline: string } {
    const health = this.health;
    switch (health.kind) {
      case 'failing':
        return { tone: 'error', headline: 'Recording failing' };
      case 'lost':
        return { tone: 'warning', headline: 'Capture loss since startup' };
      case 'healthy':
        return { tone: 'success', headline: 'Recording healthy' };
      default:
        return assertNever(health);
    }
  }

  /** The figures behind the verdict, each toned by its own limits. */
  get indicators(): Indicator[] {
    const recorder = this.recorder;
    const queue = Math.max(
      recorder.queue.capacity ? recorder.queue.items / recorder.queue.capacity : 0,
      recorder.queue.byte_capacity ? recorder.queue.bytes / recorder.queue.byte_capacity : 0,
    );
    const lost = RecorderReport.lostBytes(recorder);
    const unrecorded = recorder.omissions.unrecorded_calls;
    const errors = recorder.storage_errors + recorder.retention_errors;
    const above = (value: number, warn: number, error: number): VerdictTone =>
      value >= error ? 'error' : value >= warn ? 'warning' : 'success';
    return [
      {
        key: 'lost',
        label: 'Lost',
        value: bytes(lost),
        tone: lost ? 'warning' : 'success',
        share: null,
      },
      {
        key: 'unrecorded',
        label: 'Unrecorded',
        value: num(unrecorded),
        tone: unrecorded ? 'warning' : 'success',
        share: null,
      },
      {
        key: 'errors',
        label: 'Errors',
        value: num(errors),
        tone: errors ? 'error' : 'success',
        share: null,
      },
      {
        key: 'queue',
        label: 'Queue',
        value: Shares.percent(queue),
        tone: above(queue, RecorderReport.QUEUE_WARN, RecorderReport.QUEUE_ERROR),
        share: Math.min(1, queue),
      },
      {
        key: 'commit',
        label: 'Commit',
        value: `${num(recorder.last_commit_ms)} ms`,
        tone: above(
          recorder.max_commit_ms,
          RecorderReport.COMMIT_WARN_MS,
          RecorderReport.COMMIT_ERROR_MS,
        ),
        share: null,
      },
      {
        key: 'lag',
        label: 'Lag',
        value: `${num(recorder.lag_ms)} ms`,
        tone: above(recorder.lag_ms, RecorderReport.LAG_WARN_MS, RecorderReport.LAG_ERROR_MS),
        share: null,
      },
    ];
  }

  static counters(recorder: RecorderMetrics): { amount: number; note: CounterNote }[] {
    const omissions = recorder.omissions;
    const lost = 'Not captured: forwarding went on, the recording did not';
    return [
      {
        amount: recorder.storage_errors,
        note: {
          label: 'Storage errors',
          value: num(recorder.storage_errors),
          severity: 'error',
          detail: 'Writes to the database failed; see the daemon log',
        },
      },
      {
        amount: recorder.retention_errors,
        note: {
          label: 'Retention errors',
          value: num(recorder.retention_errors),
          severity: 'error',
          detail: 'Pruning failed, so storage may grow past its limits',
        },
      },
      {
        amount: omissions.unrecorded_calls,
        note: {
          label: 'Unrecorded calls',
          value: num(omissions.unrecorded_calls),
          severity: 'warning',
          detail: 'Calls forwarded with no record at all: the queue had no room',
        },
      },
      {
        amount: omissions.queue_bytes_bytes,
        note: {
          label: 'Omitted · queue byte limit',
          value: bytes(omissions.queue_bytes_bytes),
          severity: 'warning',
          detail: lost,
        },
      },
      {
        amount: omissions.queue_slots_bytes,
        note: {
          label: 'Omitted · queue slot limit',
          value: bytes(omissions.queue_slots_bytes),
          severity: 'warning',
          detail: lost,
        },
      },
      {
        amount: omissions.storage_failure_bytes,
        note: {
          label: 'Omitted · storage failure',
          value: bytes(omissions.storage_failure_bytes),
          severity: 'warning',
          detail: lost,
        },
      },
      {
        amount: omissions.policy_limit_bytes,
        note: {
          label: 'Over the capture limit',
          value: bytes(omissions.policy_limit_bytes),
          severity: 'neutral',
          detail: 'Body bytes past the capture policy, left out by design',
        },
      },
      {
        amount: omissions.unattributed_bytes,
        note: {
          label: 'WebSocket bytes between calls',
          value: bytes(omissions.unattributed_bytes),
          severity: 'neutral',
          detail: 'Frames that belong to no call, not stored by design',
        },
      },
    ];
  }

  /**
   * Storage, calls, and history against their limits. The one nearest its
   * end is marked binding: retention prunes the oldest calls for it first.
   */
  static gauges(
    storage: StorageView | null,
    policy: RetentionPolicy | null,
    oldest: number | null,
    now: number,
  ): Gauge[] {
    const on = !!policy?.enabled;
    const used = storage
      ? RecorderReport.live(storage.disk.metadata) + RecorderReport.live(storage.disk.blobs)
      : null;
    const age = oldest === null ? null : Math.max(0, now - oldest);
    const share = (value: number | null, limit: number | undefined) =>
      on && value !== null && limit ? value / limit : null;
    const gauges: Gauge[] = [
      {
        key: 'storage',
        label: 'Storage',
        used: used === null ? '—' : bytes(used),
        limit: on && policy ? bytes(policy.max_storage_bytes) : 'no limit',
        share: share(used, policy?.max_storage_bytes),
        binding: false,
      },
      {
        key: 'calls',
        label: 'Calls',
        used: storage ? num(storage.calls) : '—',
        limit: on && policy ? num(policy.max_calls) : 'no limit',
        share: share(storage?.calls ?? null, policy?.max_calls),
        binding: false,
      },
      {
        key: 'history',
        label: 'History',
        used: age === null ? '—' : Elapsed.span(age),
        limit: on && policy ? Elapsed.span(policy.max_age_seconds * 1000) : 'no limit',
        share: share(age, policy ? policy.max_age_seconds * 1000 : undefined),
        binding: false,
      },
    ];
    const nearest = gauges.reduce<Gauge | null>(
      (best, gauge) =>
        gauge.share !== null && (best?.share === null || best === null || gauge.share > best.share)
          ? gauge
          : best,
      null,
    );
    return gauges.map((gauge) => ({ ...gauge, binding: gauge === nearest }));
  }

  /** A capacity's meter: its own colour while fine, else how it stands. */
  static meterTone(share: number): 'chart' | 'warning' | 'error' {
    const fullness = RecorderReport.fullness(share);
    switch (fullness) {
      case 'fine':
        return 'chart';
      case 'warning':
      case 'error':
        return fullness;
      default:
        return assertNever(fullness);
    }
  }

  /** A database file's in-use bytes: its pages less the free ones. */
  static live(file: FileUsage): number {
    return Math.max(0, file.page_count - file.freelist_count) * file.page_size;
  }

  /** A database file's free pages, in bytes: room it holds and does not use. */
  static free(file: FileUsage): number {
    return file.freelist_count * file.page_size;
  }

  /** How many times `numerator` is `denominator` (`3.20×`); `—` over nothing. */
  static ratio(numerator: number, denominator: number): string {
    return denominator > 0 ? `${(numerator / denominator).toFixed(2)}×` : '—';
  }

  /** What bodies become on disk: the bytes calls reference, the distinct
   * chunks they dedupe to, and those chunks compressed. */
  static bodySteps(storage: StorageView): BodyStep[] {
    return [
      {
        label: 'Referenced body bytes',
        bytes: storage.referenced_bytes,
        note: null,
        tone: 'referenced',
      },
      {
        label: 'Unique chunk bytes',
        bytes: storage.unique_bytes,
        note: {
          label: 'Deduplication',
          ratio: RecorderReport.ratio(storage.referenced_bytes, storage.unique_bytes),
        },
        tone: 'unique',
      },
      {
        label: 'Compressed chunk bytes',
        bytes: storage.compressed_bytes,
        note: {
          label: 'Compression',
          ratio: RecorderReport.ratio(storage.unique_bytes, storage.compressed_bytes),
        },
        tone: 'compressed',
      },
    ];
  }

  /** The database files' bytes on disk: each file's live and free pages. */
  static diskParts(storage: StorageView): DiskPart[] {
    return [
      {
        key: 'meta-live',
        label: 'Metadata · live',
        bytes: RecorderReport.live(storage.disk.metadata),
      },
      {
        key: 'meta-free',
        label: 'Metadata · free pages',
        bytes: RecorderReport.free(storage.disk.metadata),
      },
      { key: 'blob-live', label: 'Blobs · live', bytes: RecorderReport.live(storage.disk.blobs) },
      {
        key: 'blob-free',
        label: 'Blobs · free pages',
        bytes: RecorderReport.free(storage.disk.blobs),
      },
    ];
  }

  /** The recorder queue against its record and byte limits. */
  static queueMeters(recorder: RecorderMetrics): QueueMeter[] {
    const share = (used: number, capacity: number) => (capacity > 0 ? used / capacity : 0);
    const queue = recorder.queue;
    return [
      {
        label: 'Records',
        share: share(queue.items, queue.capacity),
        text: `${num(queue.items)} / ${num(queue.capacity)}`,
      },
      {
        label: 'Bytes',
        share: share(queue.bytes, queue.byte_capacity),
        text: `${bytes(queue.bytes)} / ${bytes(queue.byte_capacity)}`,
      },
    ];
  }
}
