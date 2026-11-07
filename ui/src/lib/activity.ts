// What turns did and where a session's tokens went, as the views show them.
// Pure: no browser, no fetching; tested in tests/activity.test.ts.
import type {
  CallId,
  EffectKind,
  Finding,
  Harness,
  ResultStatus,
  SearchField,
  SearchHit,
  SecretKind,
  SessionId,
  SessionReport,
  ToolResultView,
  TurnId,
  TurnMetrics,
  TurnSummary,
} from './api.ts';
import { title } from './api.ts';
import { assertNever, unrecognised, type Stored } from './decode.ts';
import { compact, span, type Tone as DisplayTone } from './display.ts';

/** Activity is read from finished records, so it has no live tone. */
export type Tone = Exclude<DisplayTone, 'live'>;

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
    case 'secret_exposure':
      return {
        tone: 'error',
        title: 'Secret exposed',
        detail:
          `${secretKindLabel(finding.secret)} ${finding.hint} ` +
          (finding.field === 'reply'
            ? "in the model's reply"
            : `in the ${searchFieldLabel(finding.field).toLowerCase()}, sent to the provider`) +
          (finding.count > 1 ? ` (${finding.count} times)` : '') +
          '. Rotate it: TGV cannot recall it.',
        call: finding.call,
        turn: finding.turn,
      };
    case 'unrecognized':
      return {
        tone: 'neutral',
        title: title(finding.text),
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
      return title(unrecognised(kind));
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
      return { tone: 'neutral', label: title(unrecognised(status)) };
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

/**
 * A row's prompt text. A turn outside the listing that was read had its
 * prompt not loaded, which is not the same as having none.
 */
export function rowPrompt(row: TurnRow): string {
  if (row.summary === null) return 'Prompt not loaded';
  return row.summary.prompt?.text ?? 'No prompt read';
}

/** Text with how much was left out, for an excerpt. */
export function omitted(chars: number): string {
  return chars ? `… ${chars.toLocaleString()} more characters not shown` : '';
}

/** A snippet's text in runs, `match` where the daemon marked «a match». */
export function snippetParts(snippet: string): { text: string; match: boolean }[] {
  const parts: { text: string; match: boolean }[] = [];
  let rest = snippet;
  while (rest) {
    const open = rest.indexOf('«');
    const close = open === -1 ? -1 : rest.indexOf('»', open + 1);
    if (open === -1 || close === -1) {
      parts.push({ text: rest, match: false });
      break;
    }
    if (open > 0) parts.push({ text: rest.slice(0, open), match: false });
    parts.push({ text: rest.slice(open + 1, close), match: true });
    rest = rest.slice(close + 1);
  }
  return parts;
}

export function searchFieldLabel(field: Stored<SearchField>): string {
  switch (field) {
    case 'prompt':
      return 'Prompt';
    case 'reply':
      return 'Reply';
    case 'tool_input':
      return 'Tool input';
    case 'tool_output':
      return 'Tool output';
    default:
      return title(unrecognised(field));
  }
}

export function secretKindLabel(kind: Stored<SecretKind>): string {
  switch (kind) {
    case 'anthropic_key':
      return 'Anthropic API key';
    case 'openai_key':
      return 'OpenAI API key';
    case 'github_token':
      return 'GitHub token';
    case 'aws_access_key':
      return 'AWS access key';
    case 'google_api_key':
      return 'Google API key';
    case 'slack_token':
      return 'Slack token';
    case 'stripe_live_key':
      return 'Stripe live key';
    case 'private_key':
      return 'Private key';
    case 'jwt':
      return 'JSON Web Token';
    default:
      return title(unrecognised(kind));
  }
}

/** A harness by the name its maker gives it. */
export function harnessLabel(harness: Stored<Harness>): string {
  switch (harness) {
    case 'claude_code':
      return 'Claude Code';
    case 'codex':
      return 'Codex';
    case 'pi':
      return 'Pi';
    default:
      return title(unrecognised(harness));
  }
}

/** A call, selected in its session's traffic. */
export function callHref(session: SessionId, call: CallId): string {
  return `/sessions/${encodeURIComponent(session)}?call=${encodeURIComponent(call)}`;
}

/** What one turn did. */
export function turnHref(turn: TurnId): string {
  return `/turns/${encodeURIComponent(turn)}`;
}

/** A search hit: its turn, else its call in its session, else the call in Traffic. */
export function searchHitHref(hit: SearchHit): string {
  if (hit.turn) return turnHref(hit.turn);
  if (hit.session) return callHref(hit.session, hit.call);
  return `/traffic?call=${encodeURIComponent(hit.call)}`;
}

/** Where a finding's link leads, and what it says. */
export interface FindingLink {
  href: string;
  label: 'Open turn' | 'Open call';
}

/**
 * Where a finding leads from a page. A session's report opens the finding's
 * turn, else its call; a turn's own page opens only the call, since the turn
 * is already shown.
 */
export function findingLink(
  line: FindingLine,
  session: SessionId,
  page: 'session' | 'turn',
): FindingLink | null {
  if (page === 'session' && line.turn) return { href: turnHref(line.turn), label: 'Open turn' };
  if (line.call) return { href: callHref(session, line.call), label: 'Open call' };
  return null;
}
