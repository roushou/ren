// What the session desk's inspector shows beside a conversation: its context
// turn by turn, what it spent, where its time went, and the files it touched.
// Pure: it reads a recording's report and the conversation's steps, and is
// tested in tests/inspector.test.ts.
import type { RecordingReport } from './api.ts';
import { compact } from './display.ts';
import type { Act, Step } from './steps.ts';
import { assertNever } from './decode.ts';
import { TurnContexts } from './recordings.ts';

/** One turn's context: the part read from cache and the rest, against the
 * tallest turn shown (0–1). */
export interface TurnContextBar {
  cached: number;
  fresh: number;
  title: string;
}

export interface ContextView {
  bars: TurnContextBar[];
  /** The newest turn's context at its last call, in tokens. */
  last: number | null;
  /** The share of the recording's context read from cache. */
  cacheHit: number | null;
}

/** What the session's calls add up to. */
export interface Totals {
  input: string;
  output: string;
  calls: string;
}

/** Where time goes: the model, or one kind of step. */
export type TimeKind = 'model' | 'run' | 'read' | 'edit' | 'tool';

export interface TimeShare {
  kind: TimeKind;
  label: string;
  ms: number;
  /** Of the whole, 0–1. */
  share: number;
}

export interface TimeView {
  total: number;
  parts: TimeShare[];
}

export interface FileRow {
  path: string;
  reads: number;
  edits: number;
}

export class SessionInspector {
  /** Turns drawn at most: the newest ones. */
  static readonly TURNS = 16;

  static context(report: RecordingReport): ContextView {
    const turns = TurnContexts.of(report.turns.slice(-SessionInspector.TURNS));
    return {
      bars: turns.map((turn) => ({
        cached: turn.end * (turn.hit ?? 0),
        fresh: turn.end * (1 - (turn.hit ?? 0)),
        title: turn.title,
      })),
      last: turns.at(-1)?.endTokens ?? null,
      cacheHit: report.totals.cache_hit_percent,
    };
  }

  static totals(report: RecordingReport): Totals {
    return {
      input: compact(report.totals.context_tokens),
      output: compact(report.totals.output_tokens),
      calls: report.totals.calls.toLocaleString(),
    };
  }

  /** The kind of time a step's act spends. */
  static kindOf(act: Act): Exclude<TimeKind, 'model'> {
    switch (act) {
      case 'run':
        return 'run';
      case 'read':
      case 'search':
      case 'list':
      case 'fetch':
        return 'read';
      case 'edit':
        return 'edit';
      case 'tool':
      case 'delegate':
        return 'tool';
      default:
        return assertNever(act);
    }
  }

  static label(kind: TimeKind): string {
    switch (kind) {
      case 'model':
        return 'Model';
      case 'run':
        return 'Run';
      case 'read':
        return 'Read · Search';
      case 'edit':
        return 'Edit';
      case 'tool':
        return 'Tools';
      default:
        return assertNever(kind);
    }
  }

  /**
   * Where the time went: each kind of step by how long its steps took, and
   * the model by what is left of the recorded turns' time once the steps are
   * taken out. Without a report, the steps alone. Largest first; kinds that
   * took no time are left out.
   */
  static time(steps: readonly Step[], report: RecordingReport | null): TimeView {
    const by = new Map<TimeKind, number>();
    for (const step of steps) {
      if (step.took === null) continue;
      const kind = SessionInspector.kindOf(step.act);
      by.set(kind, (by.get(kind) ?? 0) + step.took);
    }
    const stepped = [...by.values()].reduce((sum, ms) => sum + ms, 0);
    if (report) {
      const turned = report.turns.reduce(
        (sum, turn) => sum + (turn.ended_at === null ? 0 : turn.ended_at - turn.started_at),
        0,
      );
      if (turned > stepped) by.set('model', turned - stepped);
    }
    const total = [...by.values()].reduce((sum, ms) => sum + ms, 0);
    return {
      total,
      parts: [...by]
        .filter(([, ms]) => ms > 0)
        .map(([kind, ms]) => ({
          kind,
          label: SessionInspector.label(kind),
          ms,
          share: total ? ms / total : 0,
        }))
        .toSorted((a, b) => b.ms - a.ms),
    };
  }

  /** The files the steps read and edited, the most touched first. */
  static files(steps: readonly Step[]): FileRow[] {
    const rows = new Map<string, FileRow>();
    for (const step of steps) {
      const reading = step.act === 'read';
      if (!reading && step.act !== 'edit') continue;
      for (const path of step.targets) {
        const row = rows.get(path) ?? { path, reads: 0, edits: 0 };
        if (reading) row.reads += 1;
        else row.edits += 1;
        rows.set(path, row);
      }
    }
    return [...rows.values()].toSorted(
      (a, b) => b.edits - a.edits || b.reads - a.reads || a.path.localeCompare(b.path),
    );
  }
}
