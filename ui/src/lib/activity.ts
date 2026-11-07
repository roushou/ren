// What turns did and where a session's tokens went, as the views show them.
// Pure: no browser, no fetching; tested in tests/activity.test.ts.
import type {
  CallId,
  EffectKind,
  Finding,
  ResultStatus,
  SessionReport,
  ToolResultView,
  TurnId,
  TurnMetrics,
  TurnSummary,
} from './api.ts';
import { assertNever, unrecognised, type Stored } from './decode.ts';
import { compact, span } from './display.ts';

export type Tone = 'success' | 'warning' | 'error' | 'neutral';

/** A finding as one line: what was measured, never a claimed cause. */
export interface FindingLine {
  tone: Tone;
  title: string;
  detail: string;
  /** The call it points to, if one. */
  call: CallId | null;
  turn: TurnId | null;
}

const tokens = (value: number) => `${compact(value)} tokens`;

export function findingLine(finding: Finding): FindingLine {
  switch (finding.kind) {
    case 'cache_rebuild':
      return {
        tone: 'warning',
        title: 'Cache rebuilt',
        detail:
          `Wrote ${tokens(finding.cache_written_tokens)} to the cache and read ` +
          `${compact(finding.cache_read_tokens)} of an earlier ${tokens(finding.earlier_peak_tokens)}` +
          (finding.idle_before_ms === null ? '.' : `, after ${span(finding.idle_before_ms)} idle.`),
        call: finding.call,
        turn: finding.turn,
      };
    case 'context_growth':
      return {
        tone: 'neutral',
        title: 'Context grew',
        detail: `From a peak of ${tokens(finding.earlier_peak_tokens)} to ${tokens(finding.context_tokens)}.`,
        call: finding.call,
        turn: finding.turn,
      };
    case 'large_tool_output':
      return {
        tone: 'warning',
        title: 'Large tool output',
        detail:
          `${finding.tool}${finding.target ? ` ${finding.target}` : ''} returned ` +
          `${finding.output_chars.toLocaleString()} characters, carried by every later call.`,
        call: finding.call,
        turn: null,
      };
    case 'repeated_failure':
      return {
        tone: 'error',
        title: 'Repeated failure',
        detail: `${finding.tool}${finding.target ? ` ${finding.target}` : ''} failed ${finding.failures} times.`,
        call: finding.calls[0] ?? null,
        turn: null,
      };
    case 'unrecognized':
      return {
        tone: 'neutral',
        title: finding.text.replaceAll('_', ' '),
        detail: 'A finding this UI does not know.',
        call: null,
        turn: null,
      };
    default:
      return assertNever(finding);
  }
}

export function effectLabel(kind: Stored<EffectKind>): string {
  switch (kind) {
    case 'read_file':
      return 'Read';
    case 'write_file':
      return 'Wrote';
    case 'run_command':
      return 'Ran';
    case 'search':
      return 'Searched';
    case 'delegate':
      return 'Delegated';
    case 'fetch':
      return 'Fetched';
    default:
      return unrecognised(kind).replaceAll('_', ' ');
  }
}

/** A tool call's result: what the client reported, or that none came. */
export function resultBadge(result: ToolResultView | null): { tone: Tone; label: string } {
  if (result === null) return { tone: 'warning', label: 'No result' };
  const status: Stored<ResultStatus> = result.status;
  switch (status) {
    case 'succeeded':
      return { tone: 'success', label: 'Succeeded' };
    case 'failed':
      return { tone: 'error', label: 'Failed' };
    case 'unreported':
      return { tone: 'neutral', label: 'Not reported' };
    default:
      return { tone: 'neutral', label: unrecognised(status).replaceAll('_', ' ') };
  }
}

/** One turn's row: the report's measurements and the listing's prompt. */
export interface TurnRow {
  metrics: TurnMetrics;
  summary: TurnSummary | null;
  /** The report's findings that point into this turn. */
  findings: FindingLine[];
}

/** Report rows, newest first, joined with the turn listing by turn ID. */
export function turnRows(report: SessionReport, summaries: TurnSummary[]): TurnRow[] {
  const byTurn = new Map(summaries.map((summary) => [summary.turn, summary]));
  const lines = report.findings.map(findingLine);
  return [...report.turns]
    .sort((a, b) => b.started_at - a.started_at)
    .map((metrics) => ({
      metrics,
      summary: byTurn.get(metrics.turn) ?? null,
      findings: lines.filter((line) => line.turn === metrics.turn),
    }));
}

/** Text with how much was left out, for an excerpt. */
export function omitted(chars: number): string {
  return chars ? `… ${chars.toLocaleString()} more characters not shown` : '';
}
