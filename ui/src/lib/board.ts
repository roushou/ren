// The Live page's pure logic: which sessions are on the board, what each is
// doing now, the recent calls drawn on its lane, and what needs a look.
// No browser, no fetching; tested in tests/board.test.ts.
import type {
  CallView,
  Excerpt,
  Finding,
  Harness,
  HintSource,
  SessionId,
  SessionView,
  ToolCallActivity,
  TurnActivity,
  TurnId,
} from './api.ts';
import { HARNESSES } from './api.ts';
import { findingLine, resultBadge } from './activity.ts';
import { assertNever, unrecognised, type Stored } from './decode.ts';
import { callStatus, sessionTone, span, statusBadge, type Tone } from './display.ts';

/**
 * What a session is doing, as far as its recorded calls show. Only the
 * first two are observed on the wire right now; `client` is inferred from
 * the last call asking for tools, and `idle` only means no call is open.
 */
export type SessionPhase =
  /** A call was sent and no response head has arrived. */
  | { kind: 'awaiting'; call: CallView; since: number }
  /** The response head arrived; the body is still streaming. */
  | { kind: 'responding'; call: CallView; since: number }
  /** The last call completed asking for tools: the client's turn. Inferred. */
  | { kind: 'client'; call: CallView; since: number }
  /** The last call did not end as a completed model call. */
  | { kind: 'stopped'; call: CallView; since: number; label: string; tone: Tone }
  /** No call open, and the last one completed. */
  | { kind: 'idle'; since: number };

/** One line of what the session's current turn did, newest last. */
export type Step =
  | { kind: 'tool'; key: string; name: string; target: string | null; result: ResultMark }
  | { kind: 'reply'; key: string; text: string };
export interface ResultMark {
  label: string;
  tone: Exclude<Tone, 'live'>;
}

/** A call drawn on a session's lane, as fractions of the window. */
export interface LaneMark {
  call: CallView;
  start: number;
  end: number;
  open: boolean;
  tone: Tone;
}

export interface Tile {
  session: SessionView;
  harness: Stored<Harness> | null;
  tone: number;
  phase: SessionPhase;
  /** Model calls open at once in this session (subagents run in parallel). */
  open: number;
  /** The newest model call, for its model and usage. */
  latest: CallView | null;
  /** The newest turn's ID, from the newest call that carried one. */
  turnId: TurnId | null;
  /** That turn's activity, when it was read. */
  turn: TurnActivity | null;
  prompt: Excerpt | null;
  steps: Step[];
  /** Calls in the turn whose text is not derived yet (still open, or just finished). */
  pending: number;
  /** Context the newest call carried, in tokens: provider-reported. */
  context: number | null;
  /** The share of that context read from cache, 0–1. */
  cached: number | null;
  lane: LaneMark[];
  /** The newest model name a call reported. */
  model: string | null;
  /** Output tokens of the call that reported `context`. */
  output: number | null;
  /** Calls in the newest turn so far. */
  turnCalls: number;
  /** Nothing open and nothing on the lane: drawn dimmed until it leaves the board. */
  quiet: boolean;
}

export interface Alert {
  key: string;
  session: SessionId;
  tone: Exclude<Tone, 'live' | 'success'>;
  label: string;
  detail: string;
  at: number;
  /** Where to look: a call in its session, or a turn. */
  href: string;
}

/** What the board reads: the loader's snapshot, plus the clock. */
export interface BoardInput {
  sessions: SessionView[];
  /** Model calls in the window and every open one, any order. */
  calls: CallView[];
  turns: Map<TurnId, TurnActivity>;
  now: number;
}

export class Board {
  /** A session stays on the board this long after its last call. */
  static readonly windowMs = 15 * 60_000;
  /** The lanes show this much recent time: a few agent loops. */
  static readonly laneMs = 2 * 60_000;
  /** The wall is a fixed 4 × 2; more sessions than this go to an overflow count. */
  static readonly maxTiles = 8;
  /** Waiting this long for a response head is worth a look. */
  static readonly slowHeadMs = 30_000;
  /** Steps kept per tile; the oldest fade out at the top. */
  static readonly maxSteps = 10;

  readonly tiles: Tile[];
  /** Sessions active in the window that did not fit. */
  readonly overflow: number;
  readonly alerts: Alert[];
  readonly now: number;

  constructor(input: BoardInput) {
    this.now = input.now;
    const bySession = Board.bySession(input.calls);
    const active = Board.activeSessions(input.sessions, bySession, input.now);
    this.overflow = Math.max(0, active.length - Board.maxTiles);
    this.tiles = Board.shown(active, bySession).map((session) =>
      Board.tile(session, bySession.get(session.id) ?? [], input.turns, input.now),
    );
    this.alerts = this.tiles
      .flatMap((tile) => Board.alerts(tile, bySession.get(tile.session.id) ?? [], input.now))
      .sort((a, b) => b.at - a.at);
  }

  /** The worst thing to look at in a session, for its tile's top rule. */
  attention(session: SessionId): 'error' | 'warning' | null {
    const tones = this.alerts.filter((alert) => alert.session === session).map((a) => a.tone);
    return tones.includes('error') ? 'error' : tones.length ? 'warning' : null;
  }

  /** Model calls open now, across the board. */
  get inFlight(): number {
    return this.tiles.reduce((sum, tile) => sum + tile.open, 0);
  }

  /** Calls grouped by session, each list oldest first. Unattributed calls are left out. */
  static bySession(calls: CallView[]): Map<SessionId, CallView[]> {
    const groups = new Map<SessionId, CallView[]>();
    for (const call of calls) {
      if (!call.session) continue;
      const list = groups.get(call.session) ?? [];
      list.push(call);
      groups.set(call.session, list);
    }
    for (const list of groups.values()) list.sort((a, b) => a.started_at - b.started_at);
    return groups;
  }

  /** Turns worth reading for these sessions: each one's newest. */
  static turnsToRead(
    sessions: SessionId[],
    calls: CallView[],
  ): { turn: TurnId; signature: string }[] {
    return sessions.flatMap((session) => {
      const own = calls
        .filter((call) => call.session === session)
        .sort((a, b) => a.started_at - b.started_at);
      const turn = own.findLast((call) => call.turn !== null)?.turn ?? null;
      if (!turn) return [];
      // A turn's activity changes only when one of its calls does.
      const signature = own
        .filter((call) => call.turn === turn)
        .map((call) => `${call.id}:${call.state}`)
        .join(',');
      return [{ turn, signature }];
    });
  }

  /**
   * Sessions with an open call or a call in the window, in the order they
   * first appeared, so tiles never trade places as activity moves.
   */
  static activeSessions(
    sessions: SessionView[],
    calls: Map<SessionId, CallView[]>,
    now: number,
  ): SessionView[] {
    return sessions
      .filter(
        (session) =>
          now - session.last_seen <= Board.windowMs ||
          (calls.get(session.id) ?? []).some((call) => call.state === 'open'),
      )
      .sort((a, b) => a.first_seen - b.first_seen || a.id.localeCompare(b.id));
  }

  /**
   * The sessions that get a tile: those with an open call, then the most
   * recently seen, up to `maxTiles`. They are drawn in the order they first
   * appeared, so a tile keeps its place while its session stays on the board.
   */
  static shown(active: SessionView[], calls: Map<SessionId, CallView[]>): SessionView[] {
    const busy = (session: SessionView) =>
      (calls.get(session.id) ?? []).some((call) => call.state === 'open') ? 1 : 0;
    const chosen = new Set(
      active
        .toSorted((a, b) => busy(b) - busy(a) || b.last_seen - a.last_seen)
        .slice(0, Board.maxTiles),
    );
    return active.filter((session) => chosen.has(session));
  }

  static harness(source: Stored<HintSource>): Stored<Harness> | null {
    switch (source) {
      case 'claude_code_headers':
        return 'claude_code';
      case 'codex_headers':
      case 'codex_message_metadata':
        return 'codex';
      case 'pi_extension_headers':
        return 'pi';
      default:
        unrecognised(source);
        return null;
    }
  }

  static phase(calls: CallView[], now: number): SessionPhase {
    const open = calls.filter((call) => call.state === 'open');
    const current = open.at(-1);
    if (current) {
      return current.status === null
        ? { kind: 'awaiting', call: current, since: current.started_at }
        : { kind: 'responding', call: current, since: current.started_at };
    }
    const last = calls.at(-1);
    if (!last) return { kind: 'idle', since: now };
    const since = last.ended_at ?? last.started_at;
    const status = callStatus(last);
    if (status.kind === 'model_completed')
      return last.tool_names.length
        ? { kind: 'client', call: last, since }
        : { kind: 'idle', since };
    if (status.kind === 'http_complete') return { kind: 'idle', since };
    const badge = statusBadge(status);
    const label =
      last.status !== null && last.status >= 400 ? `${badge.label} ${last.status}` : badge.label;
    return { kind: 'stopped', call: last, since, label, tone: badge.tone };
  }

  /** The phase as a short label and tone, for the tile's state chip. */
  static phaseChip(phase: SessionPhase, now: number): { label: string; tone: Tone; title: string } {
    switch (phase.kind) {
      case 'awaiting':
        return {
          label: `Waiting for response · ${span(now - phase.since)}`,
          tone: now - phase.since >= Board.slowHeadMs ? 'warning' : 'live',
          title: 'Sent upstream; no response head yet',
        };
      case 'responding':
        return {
          label: `Responding · ${span(now - phase.since)}`,
          tone: 'live',
          title: 'Response head received; the body is still streaming',
        };
      case 'client':
        return {
          label: `Client's turn · ${span(now - phase.since)}`,
          tone: 'neutral',
          title: `The last call asked for ${phase.call.tool_names.join(', ')}; inferred, TGV does not see tools run`,
        };
      case 'stopped':
        return { label: phase.label, tone: phase.tone, title: 'How the last call ended' };
      case 'idle':
        return {
          label: `Idle · ${span(now - phase.since)}`,
          tone: 'neutral',
          title: 'No call open; the last one completed',
        };
      default:
        return assertNever(phase);
    }
  }

  static steps(turn: TurnActivity): Step[] {
    const steps: Step[] = turn.calls.flatMap((call, callIndex): Step[] => [
      ...call.tool_calls.map((tool, index): Step => {
        const effect = tool.effects[0];
        return {
          kind: 'tool',
          key: `${call.call}:${tool.id ?? index}`,
          name: tool.name,
          target: effect?.target ?? null,
          result: Board.result(turn, callIndex, tool),
        };
      }),
      ...(call.reply && call.reply.text.trim()
        ? [{ kind: 'reply' as const, key: `${call.call}:reply`, text: call.reply.text }]
        : []),
    ]);
    return steps.slice(-Board.maxSteps);
  }

  /**
   * A tool call's result. A result arrives with the client's next request,
   * so while no later call in the turn has been read, a missing one is
   * expected rather than missing.
   */
  static result(turn: TurnActivity, callIndex: number, tool: ToolCallActivity): ResultMark {
    const later = turn.calls.slice(callIndex + 1);
    if (tool.result === null && later.every((call) => call.limitations.includes('not_yet_derived')))
      return { label: 'No result yet', tone: 'neutral' };
    const badge = resultBadge(tool.result);
    return { label: badge.label, tone: badge.tone };
  }

  static lane(calls: CallView[], now: number): LaneMark[] {
    const from = now - Board.laneMs;
    const at = (time: number) => Math.min(1, Math.max(0, (time - from) / Board.laneMs));
    return calls
      .filter((call) => call.state === 'open' || (call.ended_at ?? call.started_at) >= from)
      .map((call) => {
        const open = call.state === 'open';
        const end = open ? now : (call.ended_at ?? call.started_at);
        return {
          call,
          start: at(call.started_at),
          end: at(end),
          open,
          tone: statusBadge(callStatus(call)).tone,
        };
      });
  }

  static tile(
    session: SessionView,
    calls: CallView[],
    turns: Map<TurnId, TurnActivity>,
    now: number,
  ): Tile {
    const latest = calls.at(-1) ?? null;
    const lane = Board.lane(calls, now);
    const turnId = calls.findLast((call) => call.turn !== null)?.turn ?? null;
    const turn = turnId ? (turns.get(turnId) ?? null) : null;
    // Context and cache come from the newest call that reported usage.
    const usage = calls.findLast((call) => call.usage !== null)?.usage ?? null;
    const read = usage?.cache_read_tokens ?? 0;
    const written = usage?.cache_creation_tokens ?? 0;
    const context =
      usage && usage.input_tokens !== null
        ? usage.accounting === 'excludes_cache'
          ? usage.input_tokens + read + written
          : usage.input_tokens
        : null;
    const turnCalls = turnId ? calls.filter((call) => call.turn === turnId) : [];
    const derived = new Set(
      (turn?.calls ?? [])
        .filter((call) => !call.limitations.includes('not_yet_derived'))
        .map((call) => call.call as string),
    );
    return {
      session,
      harness: Board.harness(session.source),
      tone: sessionTone(session.id),
      phase: Board.phase(calls, now),
      open: calls.filter((call) => call.state === 'open').length,
      latest,
      turnId,
      turn,
      prompt: turn?.prompt ?? null,
      steps: turn ? Board.steps(turn) : [],
      pending: turnCalls.filter((call) => !derived.has(call.id)).length,
      context,
      cached: context ? Math.min(1, read / context) : null,
      lane,
      model: calls.findLast((call) => call.model !== null)?.model ?? null,
      output: usage?.output_tokens ?? null,
      turnCalls: turnCalls.length,
      quiet: !lane.length,
    };
  }

  /** What in this session needs a look: slow heads, bad endings, findings. */
  static alerts(tile: Tile, calls: CallView[], now: number): Alert[] {
    const session = tile.session.id;
    const callHref = (call: CallView) =>
      `/sessions/${encodeURIComponent(session)}?call=${encodeURIComponent(call.id)}`;
    const alerts: Alert[] = [];
    if (tile.phase.kind === 'awaiting' && now - tile.phase.since >= Board.slowHeadMs) {
      alerts.push({
        key: `${tile.phase.call.id}:head`,
        session,
        tone: 'warning',
        label: 'No response head',
        detail: `waiting ${span(now - tile.phase.since)}`,
        at: tile.phase.since,
        href: callHref(tile.phase.call),
      });
    }
    // Rate limits come in bursts: one line per session, with the count.
    const limited = calls.filter((call) => call.status === 429);
    const newestLimited = limited.at(-1);
    if (newestLimited) {
      alerts.push({
        key: `${session}:429`,
        session,
        tone: 'warning',
        label: limited.length > 1 ? `${limited.length} × 429` : '429',
        detail: 'rate limited',
        at: newestLimited.started_at,
        href: callHref(newestLimited),
      });
    }
    for (const call of calls) {
      if (call.status === 429 || call.state === 'open') continue;
      const status = callStatus(call);
      switch (status.kind) {
        case 'model_completed':
        case 'http_complete':
        case 'in_progress':
          continue;
        case 'rejected':
        case 'error':
        case 'interrupted':
        case 'recovered':
        case 'incomplete':
        case 'unknown': {
          const badge = statusBadge(status);
          alerts.push({
            key: call.id,
            session,
            tone: badge.tone === 'error' ? 'error' : 'warning',
            label: call.status !== null && call.status >= 400 ? `${call.status}` : badge.label,
            detail: call.model ?? call.endpoint,
            at: call.ended_at ?? call.started_at,
            href: callHref(call),
          });
          continue;
        }
        default:
          assertNever(status);
      }
    }
    for (const [index, finding] of (tile.turn?.findings ?? []).entries()) {
      const alert = Board.findingAlert(tile, finding, index);
      if (alert) alerts.push(alert);
    }
    return alerts;
  }

  /** Findings that matter while a turn runs; growth and cache notes wait for the report. */
  static findingAlert(tile: Tile, finding: Finding, index: number): Alert | null {
    switch (finding.kind) {
      case 'secret_exposure':
      case 'repeated_failure': {
        const line = findingLine(finding);
        if (!tile.turn) return null;
        return {
          key: `${tile.turn.turn}:finding:${index}`,
          session: tile.session.id,
          tone: line.tone === 'error' ? 'error' : 'warning',
          label: line.title,
          detail: line.detail,
          at: tile.turn.ended_at ?? tile.turn.started_at ?? tile.session.last_seen,
          href: `/turns/${encodeURIComponent(tile.turn.turn)}`,
        };
      }
      case 'cache_rebuild':
      case 'context_growth':
      case 'large_tool_output':
      case 'unrecognized':
        return null;
      default:
        return assertNever(finding);
    }
  }
}

/**
 * The Live page's place, from its URL: how dense the board is. Parsed once;
 * the page never reads raw parameters.
 */
export interface LiveView {
  density: Density;
}
export type Density = 'comfortable' | 'compact';

export class LivePlace {
  static parse(params: { get(name: string): string | null }): LiveView {
    return { density: params.get('density') === 'compact' ? 'compact' : 'comfortable' };
  }

  static href(view: LiveView): string {
    return view.density === 'compact' ? '/live?density=compact' : '/live';
  }
}

/** One harness's row in the activity band. */
export interface BandRow {
  harness: Harness;
  /** Output tokens per second in each bin, oldest first. */
  bins: number[];
  /** Output tokens per second over the last `Band.rateMs`. */
  rate: number;
  /** Calls that ended badly, as fractions of the span. */
  marks: { key: string; at: number; tone: 'warning' | 'error' }[];
}

/**
 * Output tokens per harness over recent minutes. Usage arrives when a call
 * finishes, so each call's tokens are drawn at its end: the band trails the
 * stream, and tokens still streaming are not in it.
 */
export class Band {
  static readonly spanMs = 10 * 60_000;
  static readonly binMs = 15_000;
  static readonly rateMs = 60_000;

  readonly rows: BandRow[];
  /** The scale shared by every row, in tokens per second. */
  readonly max: number;

  constructor(sessions: SessionView[], calls: CallView[], now: number) {
    const bySession = new Map(sessions.map((session) => [session.id, session]));
    const count = Band.spanMs / Band.binMs;
    const from = now - Band.spanMs;
    this.rows = HARNESSES.map((harness): BandRow => {
      const own = calls.filter((call) => Band.harnessOf(call, bySession) === harness);
      const bins = Array.from({ length: count }, () => 0);
      let recent = 0;
      const marks: BandRow['marks'] = [];
      for (const call of own) {
        if (call.ended_at === null || call.ended_at < from || call.ended_at > now) continue;
        const out = call.usage?.output_tokens ?? 0;
        const index = Math.min(count - 1, Math.floor((call.ended_at - from) / Band.binMs));
        bins[index] = (bins[index] ?? 0) + out / (Band.binMs / 1000);
        if (now - call.ended_at <= Band.rateMs) recent += out;
        const tone = statusBadge(callStatus(call)).tone;
        if (tone === 'warning' || tone === 'error')
          marks.push({ key: call.id, at: (call.ended_at - from) / Band.spanMs, tone });
      }
      return { harness, bins, rate: recent / (Band.rateMs / 1000), marks };
    });
    const peak = Math.max(0, ...this.rows.flatMap((row) => row.bins));
    // A round scale, never below 50 tok/s, so a trickle does not look like a flood.
    this.max = Math.max(50, Math.ceil((peak * 1.1) / 50) * 50);
  }

  /** The harness a call came from: its own hint, else its session's. */
  static harnessOf(call: CallView, sessions: Map<SessionId, SessionView>): Harness | null {
    const source =
      call.attribution.hint?.source ??
      (call.session ? sessions.get(call.session)?.source : undefined);
    if (source === undefined) return null;
    const harness = Board.harness(source);
    return harness !== null && (HARNESSES as readonly string[]).includes(harness)
      ? (harness as Harness)
      : null;
  }

  /** A row's bins as an SVG line in a 1000 × `height` box. */
  line(row: BandRow, height: number): string {
    const step = 1000 / row.bins.length;
    const y = (value: number) => (height - (value / this.max) * (height - 3)).toFixed(1);
    const points = row.bins.map(
      (value, index) => `${(index * step + step / 2).toFixed(1)},${y(value)}`,
    );
    return `M0,${y(row.bins[0] ?? 0)} L${points.join(' L')} L1000,${y(row.bins.at(-1) ?? 0)}`;
  }

  /** The same line closed down to the baseline, for the fill under it. */
  area(row: BandRow, height: number): string {
    return `${this.line(row, height)} L1000,${height} L0,${height} Z`;
  }
}
