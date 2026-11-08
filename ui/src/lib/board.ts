// The Live page's pure logic: which sessions are on the board, what each is
// doing now, the recent calls drawn on its lane, and what needs a look.
// No browser, no fetching; tested in tests/board.test.ts.
import type {
  CallView,
  Provider,
  EffectKind,
  Excerpt,
  Finding,
  Harness,
  HintSource,
  SessionView,
  ToolCallActivity,
  TurnActivity,
  TurnId,
} from './api.ts';
import { HARNESSES, providerName, SessionId } from './api.ts';
import { findingLine, harnessLabel, resultBadge } from './activity.ts';
import { assertNever, unrecognised, type Stored } from './decode.ts';
import { callStatus, sessionTone, span, statusBadge, type Tone } from './display.ts';

/**
 * What a session is doing, as far as its recorded calls show. Only the
 * first two are observed on the wire right now; `client` is inferred from
 * the last call asking for tools, `yours` from the last call finishing its
 * turn without asking for any, and `idle` only means no call is open.
 */
export type SessionPhase =
  /** A call was sent and no response head has arrived. */
  | { kind: 'awaiting'; call: CallView; since: number }
  /** The response head arrived; the body is still streaming. */
  | { kind: 'responding'; call: CallView; since: number }
  /** The last call completed asking for tools: the client's turn. Inferred. */
  | { kind: 'client'; call: CallView; since: number }
  /** The last call completed without asking for tools, lately: the user's turn. Inferred. */
  | { kind: 'yours'; call: CallView; since: number }
  /** The last call did not end as a completed model call. */
  | {
      kind: 'stopped';
      call: CallView;
      since: number;
      label: string;
      tone: Tone;
      ending: 'failed' | 'warning' | 'unknown';
    }
  /** No call open, and the last one completed a while ago. */
  | { kind: 'idle'; since: number };

/**
 * A card's state at a glance: the colour of its edge and the glyph before
 * its state's words. Each one has its own shape, so colour is never the
 * only cue.
 */
export type StateMark =
  | 'responding'
  | 'waiting'
  | 'slow'
  | 'client'
  | 'yours'
  | 'idle'
  | 'failed'
  | 'warning'
  | 'unknown';

/** A tool call the model asked for: what it touched and what came back. */
export interface ToolLine {
  key: string;
  name: string;
  effect: Stored<EffectKind> | null;
  /** What it touched, with the session's directory taken off. */
  target: string | null;
  result: ResultMark;
  /** The first line of what the client reported back. */
  output: string | null;
  /** The tool's input and reported output, for the expanded card. */
  input: Excerpt;
  reported: Excerpt | null;
}

/** A model call of the current turn, for the expanded card. */
export interface TurnCall {
  key: string;
  /** Its place among the turn's calls on the board, from 1. */
  index: number | null;
  durationMs: number | null;
  output: number | null;
  reply: string | null;
  tools: ToolLine[];
}

/**
 * One line of a card, newest last: a reply, or a run of like tool calls
 * (same tool, same kind of effect, nothing said between them), drawn as its
 * newest call with a count.
 */
export type CardLine =
  | { kind: 'tools'; key: string; tool: ToolLine; count: number; result: ResultMark }
  | { kind: 'reply'; key: string; text: string };

/** What the turn has done so far, as the client reported it. */
export interface TurnTally {
  read: number;
  wrote: number;
  ran: number;
  failed: number;
}

/** A call of the turn not read yet: still open, or just finished. */
export interface UnreadCall {
  call: CallView;
  index: number;
}

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
  /** The turn's read calls, oldest first, for the expanded card. */
  calls: TurnCall[];
  /** The card's lines: the newest `maxLines`, like tool calls merged. */
  lines: CardLine[];
  /** Calls in the turn whose text is not derived yet (still open, or just finished). */
  unread: UnreadCall[];
  /** The directory the turn's commands change into, when they agree on one. */
  workdir: string | null;
  tally: TurnTally | null;
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
  /** Kept on the wall by the viewer. */
  pinned: boolean;
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
  /** How far back a session counts as active. */
  windowMs: number;
  /** The session expanded on the wall, if any. */
  focus: SessionId | null;
  /** Sessions kept on the wall while they are active, however quiet. */
  pins: SessionId[];
  /** Only sessions in these band series, when the viewer picked one. */
  only: { by: BandBy; keys: string[] } | null;
}

export class Board {
  /** The lanes show this much recent time: a few agent loops. */
  static readonly laneMs = 2 * 60_000;
  /** The wall is a fixed 4 × 2; more sessions than this go to an overflow count. */
  static readonly maxTiles = 8;
  /** Waiting this long for a response head is worth a look. */
  static readonly slowHeadMs = 30_000;
  /** An expanded card covers four slots, leaving this many beside it. */
  static readonly besideFocus = 4;
  /** Lines kept per card; the oldest fade out at the top. */
  static readonly maxLines = 10;

  readonly tiles: Tile[];
  /** The expanded session, when it is on the board; its tile comes first. */
  readonly focus: SessionId | null;
  /** Sessions active in the window that did not fit. */
  readonly overflow: number;
  readonly alerts: Alert[];
  /** The Now strip: what happened lately, newest first. */
  readonly feed: FeedItem[];
  readonly now: number;

  constructor(input: BoardInput) {
    this.now = input.now;
    const bySession = Board.bySession(input.calls);
    const only = input.only;
    const active = Board.activeSessions(
      input.sessions,
      bySession,
      input.now,
      input.windowMs,
    ).filter(
      (session) =>
        !only ||
        only.keys.includes(
          Band.sessionKey(
            only.by,
            session,
            (bySession.get(session.id) ?? []).findLast((call) => call.model !== null)?.model ??
              null,
          ),
        ),
    );
    this.focus = active.some((session) => session.id === input.focus) ? input.focus : null;
    const pins = new Set(input.pins);
    const shown = Board.shown(active, bySession, this.focus, pins);
    this.overflow = active.length - shown.length;
    this.tiles = shown.map((session) => ({
      ...Board.tile(session, bySession.get(session.id) ?? [], input.turns, input.now),
      pinned: pins.has(session.id),
    }));
    this.alerts = this.tiles
      .flatMap((tile) => Board.alerts(tile, bySession.get(tile.session.id) ?? [], input.now))
      .sort((a, b) => b.at - a.at);
    this.feed = new Feed(input, this.alerts).items;
  }

  /** Tiles the wall has room for: eight, or the expanded one and four beside it. */
  get capacity(): number {
    return this.focus ? 1 + Board.besideFocus : Board.maxTiles;
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
    windowMs: number,
  ): SessionView[] {
    return sessions
      .filter(
        (session) =>
          now - session.last_seen <= windowMs ||
          (calls.get(session.id) ?? []).some((call) => call.state === 'open'),
      )
      .sort((a, b) => a.first_seen - b.first_seen || a.id.localeCompare(b.id));
  }

  /**
   * The sessions that get a tile: pinned ones, then those with an open call,
   * then the most recently seen, up to `maxTiles`. They are drawn in the
   * order they first appeared, so a tile keeps its place while its session
   * stays on the board. An expanded session comes first, with `besideFocus`
   * others after it.
   */
  static shown(
    active: SessionView[],
    calls: Map<SessionId, CallView[]>,
    focus: SessionId | null = null,
    pins: ReadonlySet<SessionId> = new Set(),
  ): SessionView[] {
    const busy = (session: SessionView) =>
      (pins.has(session.id) ? 2 : 0) +
      ((calls.get(session.id) ?? []).some((call) => call.state === 'open') ? 1 : 0);
    const focused = active.find((session) => session.id === focus);
    const others = active.filter((session) => session !== focused);
    const chosen = new Set(
      others
        .toSorted((a, b) => busy(b) - busy(a) || b.last_seen - a.last_seen)
        .slice(0, focused ? Board.besideFocus : Board.maxTiles),
    );
    return [...(focused ? [focused] : []), ...others.filter((session) => chosen.has(session))];
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
      case 'opencode_headers':
        return 'opencode';
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
        : now - since < Board.laneMs
          ? { kind: 'yours', call: last, since }
          : { kind: 'idle', since };
    if (status.kind === 'http_complete') return { kind: 'idle', since };
    const badge = statusBadge(status);
    const label =
      last.status !== null && last.status >= 400 ? `${badge.label} ${last.status}` : badge.label;
    // A restart or an unreadable ending is unknown, never rounded to a failure or a success.
    const ending =
      status.kind === 'recovered' || status.kind === 'unknown'
        ? 'unknown'
        : badge.tone === 'error'
          ? 'failed'
          : 'warning';
    return { kind: 'stopped', call: last, since, label, tone: badge.tone, ending };
  }

  /** The phase as a short label, tone, and mark, for the tile's state chip and edge. */
  static phaseChip(
    phase: SessionPhase,
    now: number,
  ): { label: string; tone: Tone; title: string; mark: StateMark } {
    switch (phase.kind) {
      case 'awaiting': {
        const slow = now - phase.since >= Board.slowHeadMs;
        return {
          label: `Waiting for response · ${span(now - phase.since)}`,
          tone: slow ? 'warning' : 'live',
          title: 'Sent upstream; no response head yet',
          mark: slow ? 'slow' : 'waiting',
        };
      }
      case 'responding':
        return {
          label: `Responding · ${span(now - phase.since)}`,
          tone: 'live',
          title: 'Response head received; the body is still streaming',
          mark: 'responding',
        };
      case 'client':
        return {
          label: `Client's turn · ${span(now - phase.since)}`,
          tone: 'neutral',
          title: `The last call asked for ${phase.call.tool_names.join(', ')}; inferred, TGV does not see tools run`,
          mark: 'client',
        };
      case 'yours':
        return {
          label: `Your turn · ${span(now - phase.since)}`,
          tone: 'neutral',
          title:
            'The last call finished its turn without asking for tools; inferred, TGV does not see your terminal',
          mark: 'yours',
        };
      case 'stopped':
        return {
          label: phase.label,
          tone: phase.tone,
          title: 'How the last call ended',
          mark: phase.ending,
        };
      case 'idle':
        return {
          label: `Idle · ${span(now - phase.since)}`,
          tone: 'neutral',
          title: 'No call open; the last one completed',
          mark: 'idle',
        };
      default:
        return assertNever(phase);
    }
  }

  /**
   * The turn's read calls, oldest first. `order` numbers the turn's calls on
   * the board, so a call keeps the number its card header counts to.
   */
  static turnCalls(
    turn: TurnActivity,
    order: Map<string, number>,
    workdir: string | null,
  ): TurnCall[] {
    return turn.calls
      .filter((call) => !call.limitations.includes('not_yet_derived'))
      .map((call): TurnCall => {
        const callIndex = turn.calls.indexOf(call);
        return {
          key: call.call,
          index: order.get(call.call) ?? null,
          durationMs: call.ended_at === null ? null : call.ended_at - call.started_at,
          output: call.metrics.output_tokens,
          reply: call.reply && call.reply.text.trim() ? call.reply.text : null,
          tools: call.tool_calls.map((tool, index): ToolLine => {
            const effect = tool.effects[0] ?? null;
            return {
              key: `${call.call}:${tool.id ?? index}`,
              name: tool.name,
              effect: effect?.kind ?? null,
              target: effect ? Board.shorten(effect.target, workdir) : null,
              result: Board.result(turn, callIndex, tool),
              output: tool.result ? Board.firstLine(tool.result.output.text) : null,
              input: tool.input,
              reported: tool.result?.output ?? null,
            };
          }),
        };
      });
  }

  /**
   * A card's lines, newest last. Within a call the reply comes first (the
   * model writes, then asks for tools); like tool calls with nothing said
   * between them merge into one line, across calls.
   */
  static lines(calls: TurnCall[]): CardLine[] {
    const lines: CardLine[] = [];
    let run: ToolLine[] = [];
    const close = () => {
      const tool = run.at(-1);
      if (tool) lines.push(Board.run(run, tool));
      run = [];
    };
    for (const call of calls) {
      if (call.reply) {
        close();
        lines.push({ kind: 'reply', key: `${call.key}:reply`, text: call.reply });
      }
      for (const tool of call.tools) {
        const last = run.at(-1);
        if (last && (last.name !== tool.name || last.effect !== tool.effect)) close();
        run.push(tool);
      }
    }
    close();
    return lines.slice(-Board.maxLines);
  }

  /** A run of like tool calls as one line: the newest, a count, and a tally of results. */
  static run(run: ToolLine[], tool: ToolLine): CardLine {
    const failed = run.filter((line) => line.result.tone === 'error').length;
    const succeeded = run.filter((line) => line.result.label === 'Succeeded').length;
    const result: ResultMark =
      run.length === 1
        ? tool.result
        : failed
          ? { label: `${failed} failed`, tone: 'error' }
          : succeeded === run.length
            ? { label: `${run.length} ok`, tone: 'success' }
            : tool.result;
    return { kind: 'tools', key: tool.key, tool, count: run.length, result };
  }

  /** `cd <dir> && …` or `cd <dir>; …` at the start of a command: the directory and the rest. */
  static changeDirectory(command: string): { dir: string; rest: string } | null {
    const match = /^cd\s+(?:"([^"]+)"|'([^']+)'|(\S+))\s*(?:&&|;)\s*/.exec(command);
    const dir = match ? (match[1] ?? match[2] ?? match[3]) : undefined;
    return match && dir ? { dir, rest: command.slice(match[0].length) } : null;
  }

  /** The directory most of the turn's commands change into first. */
  static workdir(turn: TurnActivity): string | null {
    const counts = new Map<string, number>();
    for (const command of turn.commands) {
      const dir = Board.changeDirectory(command)?.dir;
      if (dir) counts.set(dir, (counts.get(dir) ?? 0) + 1);
    }
    let best: string | null = null;
    for (const [dir, count] of counts)
      if (best === null || count > (counts.get(best) ?? 0)) best = dir;
    return best;
  }

  /** A tool's target as the card shows it: no `cd` into the workdir, paths relative to it. */
  static shorten(target: string, workdir: string | null): string {
    if (!workdir) return target;
    const cd = Board.changeDirectory(target);
    const text = cd && cd.dir === workdir ? cd.rest : target;
    const root = workdir.replace(/\/+$/, '');
    return text.split(`${root}/`).join('').split(root).join('.');
  }

  /** The first line with text in it, clipped. */
  static firstLine(text: string): string | null {
    const line = text
      .split('\n')
      .map((part) => part.trim())
      .find((part) => part.length > 0);
    return line ? Feed.clip(line, 160) : null;
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
    const order = new Map(turnCalls.map((call, index) => [call.id as string, index + 1]));
    const workdir = turn ? Board.workdir(turn) : null;
    const readCalls = turn ? Board.turnCalls(turn, order, workdir) : [];
    const derived = new Set(readCalls.map((call) => call.key));
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
      calls: readCalls,
      lines: Board.lines(readCalls),
      unread: turnCalls.flatMap((call, index) =>
        derived.has(call.id) ? [] : [{ call, index: index + 1 }],
      ),
      workdir,
      tally: turn
        ? {
            read: turn.files_read.length,
            wrote: turn.files_written.length,
            ran: turn.commands.length,
            failed: turn.failed_tool_calls,
          }
        : null,
      context,
      cached: context ? Math.min(1, read / context) : null,
      lane,
      model: calls.findLast((call) => call.model !== null)?.model ?? null,
      output: usage?.output_tokens ?? null,
      turnCalls: turnCalls.length,
      quiet: !lane.length,
      pinned: false,
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

/** One line in the Now strip. */
export interface FeedItem {
  /** Stable while the item lasts, so an update never animates it in again. */
  key: string;
  session: SessionView | null;
  tone: 'neutral' | 'success' | 'warning' | 'error';
  text: string;
  detail: string | null;
  /** When it happened; for a pinned item, when it began. */
  at: number;
  /** Still going on (a call waiting for its response head): kept first, its age ticking. */
  pinned: boolean;
  href: string | null;
}

/**
 * What the gateway saw happen, newest first: sessions starting, prompts,
 * calls finishing, failed tools, bad endings, and rate limits. Derived again
 * from each snapshot, so an item is only as fresh as the call it comes from:
 * never per token. Calm items last `calmMs`; problems last `problemMs`.
 */
export class Feed {
  static readonly calmMs = 2 * 60_000;
  static readonly problemMs = 5 * 60_000;
  /** A call waiting longer than this for its response head is pinned. */
  static readonly waitMs = 5_000;
  /** Rate limits are counted per provider over this long. */
  static readonly limitMs = 60_000;
  static readonly maxItems = 24;

  readonly items: FeedItem[];

  constructor(input: BoardInput, alerts: Alert[]) {
    const { now } = input;
    const sessions = new Map(input.sessions.map((session) => [session.id, session]));
    const of = (id: SessionId | null) => (id ? (sessions.get(id) ?? null) : null);
    const callHref = (call: CallView) =>
      call.session
        ? `/sessions/${encodeURIComponent(call.session)}?call=${encodeURIComponent(call.id)}`
        : null;
    const calm = (at: number) => at <= now && now - at <= Feed.calmMs;
    const items: FeedItem[] = [];

    for (const session of input.sessions) {
      if (!calm(session.first_seen)) continue;
      const harness = Board.harness(session.source);
      items.push({
        key: `${session.id}:new`,
        session,
        tone: 'neutral',
        text: 'new session',
        detail: `${harness ? harnessLabel(harness) : 'Unknown client'} → ${providerName(session.provider)}`,
        at: session.first_seen,
        pinned: false,
        href: `/sessions/${encodeURIComponent(session.id)}`,
      });
    }

    for (const turn of input.turns.values()) {
      const href = `/turns/${encodeURIComponent(turn.turn)}`;
      if (turn.prompt && turn.started_at !== null && calm(turn.started_at)) {
        items.push({
          key: `${turn.turn}:prompt`,
          session: of(turn.session),
          tone: 'neutral',
          text: `you: “${Feed.clip(turn.prompt.text, 60)}”`,
          detail: null,
          at: turn.started_at,
          pinned: false,
          href,
        });
      }
      // A tool's result arrives with the next request, so it fails when that call is sent.
      for (const [index, call] of turn.calls.entries()) {
        const at = turn.calls[index + 1]?.started_at ?? call.ended_at;
        if (at === null || !calm(at)) continue;
        for (const [position, tool] of call.tool_calls.entries()) {
          if (Board.result(turn, index, tool).tone !== 'error') continue;
          const target = tool.effects[0]?.target ?? null;
          items.push({
            key: `${call.call}:${tool.id ?? position}:failed`,
            session: of(turn.session),
            tone: 'error',
            text: `${tool.name}${target ? ` ${Feed.clip(target, 40)}` : ''} failed`,
            detail: null,
            at,
            pinned: false,
            href,
          });
        }
      }
    }

    for (const call of input.calls) {
      if (call.state === 'open') {
        const waited = now - call.started_at;
        if (call.status !== null || waited < Feed.waitMs) continue;
        items.push({
          key: `${call.id}:waiting`,
          session: of(call.session),
          tone: waited >= Board.slowHeadMs ? 'warning' : 'neutral',
          text: `no response head after ${span(waited)}`,
          detail: call.model,
          at: call.started_at,
          pinned: true,
          href: callHref(call),
        });
        continue;
      }
      const at = call.ended_at ?? call.started_at;
      if (!calm(at) || callStatus(call).kind !== 'model_completed') continue;
      const out = call.usage?.output_tokens;
      const tools = [...new Set(call.tool_names)];
      items.push({
        key: `${call.id}:done`,
        session: of(call.session),
        tone: tools.length ? 'neutral' : 'success',
        text: tools.length
          ? `asked for ${tools.slice(0, 3).join(', ')}${tools.length > 3 ? ` +${tools.length - 3}` : ''}`
          : 'turn finished',
        detail: out === undefined || out === null ? null : `out ${out}`,
        at,
        pinned: false,
        href: callHref(call),
      });
    }

    // Bad endings and findings; slow heads and rate limits are told above and below.
    for (const alert of alerts) {
      if (alert.key.endsWith(':head') || alert.key.endsWith(':429')) continue;
      if (alert.at > now || now - alert.at > Feed.problemMs) continue;
      items.push({
        key: `alert:${alert.key}`,
        session: of(alert.session),
        tone: alert.tone === 'error' ? 'error' : 'warning',
        text: alert.label,
        detail: alert.detail,
        at: alert.at,
        pinned: false,
        href: alert.href,
      });
    }

    const limited = new Map<string, CallView[]>();
    for (const call of input.calls) {
      if (call.status !== 429 || now - call.started_at > Feed.limitMs) continue;
      limited.set(call.provider, [...(limited.get(call.provider) ?? []), call]);
    }
    for (const [provider, calls] of limited) {
      const newest = calls.reduce((a, b) => (b.started_at > a.started_at ? b : a));
      items.push({
        key: `429:${provider}`,
        session: null,
        tone: 'warning',
        text: `${providerName(provider)}: ${calls.length} rate limit${calls.length > 1 ? 's' : ''} (429)`,
        detail: `in the last ${span(Feed.limitMs)}`,
        at: newest.started_at,
        pinned: false,
        href: callHref(newest),
      });
    }

    this.items = items
      .sort((a, b) =>
        a.pinned === b.pinned ? (a.pinned ? a.at - b.at : b.at - a.at) : a.pinned ? -1 : 1,
      )
      .slice(0, Feed.maxItems);
  }

  /** Text cut to `length` characters on one line. */
  static clip(text: string, length: number): string {
    const line = text.replace(/\s+/g, ' ').trim();
    return line.length > length ? `${line.slice(0, length - 1)}…` : line;
  }
}

/**
 * The Live page's place, from its URL: how dense the board is. Parsed once;
 * the page never reads raw parameters.
 */
export interface LiveView {
  density: Density;
  band: BandStyle;
  window: LiveWindow;
  /** The session whose card is expanded. */
  focus: SessionId | null;
  /** Sessions kept on the wall, in the order they were pinned. */
  pins: SessionId[];
  /** What the band groups by. */
  by: BandBy;
  /** The band series the wall is narrowed to, by its key. */
  series: string | null;
}
export type Density = 'comfortable' | 'compact';
/** How the band draws its clients: a row each, or stacked into one ribbon. */
export type BandStyle = 'area' | 'ribbon';
/** How far back a session counts as active. */
export type LiveWindow = '5m' | '15m' | '1h' | '4h';

/** What each window means, and how much the page reads for it. */
export interface WindowSpec {
  ms: number;
  label: string;
  /** Calls read at most: the API's page limit caps it at 1000. */
  calls: number;
  sessions: number;
}

export class LivePlace {
  static readonly windows: Record<LiveWindow, WindowSpec> = {
    '5m': { ms: 5 * 60_000, label: 'last 5 min', calls: 500, sessions: 100 },
    '15m': { ms: 15 * 60_000, label: 'last 15 min', calls: 500, sessions: 100 },
    '1h': { ms: 60 * 60_000, label: 'last 1 h', calls: 1000, sessions: 200 },
    '4h': { ms: 4 * 60 * 60_000, label: 'last 4 h', calls: 1000, sessions: 300 },
  };
  static readonly defaultWindow: LiveWindow = '15m';

  static parse(params: {
    get(name: string): string | null;
    getAll(name: string): string[];
  }): LiveView {
    const window = params.get('window');
    return {
      density: params.get('density') === 'compact' ? 'compact' : 'comfortable',
      band: params.get('band') === 'area' ? 'area' : 'ribbon',
      window: LivePlace.isWindow(window) ? window : LivePlace.defaultWindow,
      focus: SessionId.parse(params.get('focus')),
      pins: [...new Set(params.getAll('pin').flatMap((raw) => SessionId.parse(raw) ?? []))].slice(
        0,
        Board.maxTiles,
      ),
      by: LivePlace.parseBy(params.get('by')),
      series: params.get('series') || null,
    };
  }

  static parseBy(text: string | null): BandBy {
    switch (text) {
      case 'provider':
        return 'provider';
      case 'model':
        return 'model';
      default:
        return 'client';
    }
  }

  static isWindow(text: string | null): text is LiveWindow {
    return text !== null && Object.hasOwn(LivePlace.windows, text);
  }

  static href(view: LiveView): string {
    const params = new URLSearchParams();
    if (view.window !== LivePlace.defaultWindow) params.set('window', view.window);
    if (view.density === 'compact') params.set('density', 'compact');
    if (view.band === 'area') params.set('band', 'area');
    if (view.by !== 'client') params.set('by', view.by);
    if (view.series) params.set('series', view.series);
    if (view.focus) params.set('focus', view.focus);
    for (const pin of view.pins) params.append('pin', pin);
    const query = params.toString();
    return query ? `/live?${query}` : '/live';
  }

  /** The view with this session pinned, or unpinned if it was. */
  static togglePin(view: LiveView, session: SessionId): LiveView {
    return {
      ...view,
      pins: view.pins.includes(session)
        ? view.pins.filter((pin) => pin !== session)
        : [...view.pins, session],
    };
  }

  /** The next longer window, to offer when the board is empty. */
  static longer(window: LiveWindow): LiveWindow | null {
    const order = Object.keys(LivePlace.windows) as LiveWindow[];
    return order[order.indexOf(window) + 1] ?? null;
  }
}

/** What the band groups output by. */
export type BandBy = 'client' | 'provider' | 'model';

/**
 * One series in the activity band: a client, a provider, or a model, or
 * every series past the biggest few, merged as `other`.
 */
export interface BandSeries {
  /** The client, provider, or model it stands for; `other` for the merged rest. */
  key: string;
  label: string;
  /** The keys it covers: its own, or every one merged into `other`. */
  members: string[];
  /** The CSS class that colours it. */
  swatch: string;
  /** Output tokens per second in each bin, oldest first. */
  bins: number[];
  /** Output tokens per second over the last `Band.rateMs`. */
  rate: number;
  /** Calls that ended badly, as fractions of the span. */
  marks: { key: string; at: number; tone: 'warning' | 'error' }[];
}

/**
 * Output tokens over recent minutes, by client, provider, or model. Only
 * series with output or a bad ending in the span are drawn, the biggest
 * `maxSeries` on their own and the rest as one `other`, so the band never
 * grows with the number of clients TGV knows. Usage arrives when a call
 * finishes, so each call's tokens are drawn at its end: the band trails the
 * stream, and tokens still streaming are not in it.
 */
export class Band {
  static readonly spanMs = 10 * 60_000;
  static readonly binMs = 15_000;
  static readonly rateMs = 60_000;
  static readonly maxSeries = 4;
  /** The merged series' key; no client, provider, or model is named this. */
  static readonly other = '__other';

  /** Biggest first: the ribbon stacks them from the bottom. */
  readonly series: BandSeries[];
  /** The scale shared by every series drawn alone, in tokens per second. */
  readonly max: number;
  /** The ribbon's scale: every series stacked, in tokens per second. */
  readonly ribbonMax: number;

  constructor(sessions: SessionView[], calls: CallView[], now: number, by: BandBy = 'client') {
    const bySession = new Map(sessions.map((session) => [session.id, session]));
    const count = Band.spanMs / Band.binMs;
    const from = now - Band.spanMs;
    const groups = new Map<
      string,
      { bins: number[]; recent: number; total: number; marks: BandSeries['marks'] }
    >();
    for (const call of calls) {
      if (call.ended_at === null || call.ended_at < from || call.ended_at > now) continue;
      const key = Band.callKey(by, call, bySession);
      const group = groups.get(key) ?? {
        bins: Array.from({ length: count }, () => 0),
        recent: 0,
        total: 0,
        marks: [],
      };
      groups.set(key, group);
      const out = call.usage?.output_tokens ?? 0;
      const index = Math.min(count - 1, Math.floor((call.ended_at - from) / Band.binMs));
      group.bins[index] = (group.bins[index] ?? 0) + out / (Band.binMs / 1000);
      group.total += out;
      if (now - call.ended_at <= Band.rateMs) group.recent += out;
      const tone = statusBadge(callStatus(call)).tone;
      if (tone === 'warning' || tone === 'error')
        group.marks.push({ key: call.id, at: (call.ended_at - from) / Band.spanMs, tone });
    }
    const ranked = [...groups]
      .filter(([, group]) => group.total > 0 || group.marks.length > 0)
      .sort(([a, x], [b, y]) => y.total - x.total || a.localeCompare(b));
    const own = ranked.length > Band.maxSeries + 1 ? ranked.slice(0, Band.maxSeries) : ranked;
    const rest = ranked.slice(own.length);
    const rate = (recent: number) => recent / (Band.rateMs / 1000);
    this.series = own.map(([key, group], slot) => ({
      key,
      label: Band.label(by, key),
      members: [key],
      swatch: Band.swatch(by, key, slot),
      bins: group.bins,
      rate: rate(group.recent),
      marks: group.marks,
    }));
    if (rest.length) {
      this.series.push({
        key: Band.other,
        label: `${rest.length} more`,
        members: rest.map(([key]) => key),
        swatch: 'series-other',
        bins: Array.from({ length: count }, (_, index) =>
          rest.reduce((sum, [, group]) => sum + (group.bins[index] ?? 0), 0),
        ),
        rate: rate(rest.reduce((sum, [, group]) => sum + group.recent, 0)),
        marks: rest.flatMap(([, group]) => group.marks),
      });
    }
    this.max = Band.scale(Math.max(0, ...this.series.flatMap((series) => series.bins)));
    const totals = Array.from({ length: count }, (_, index) =>
      this.series.reduce((sum, series) => sum + (series.bins[index] ?? 0), 0),
    );
    this.ribbonMax = Band.scale(Math.max(0, ...totals));
  }

  /** What a call counts toward, grouped `by`. */
  static callKey(by: BandBy, call: CallView, sessions: Map<SessionId, SessionView>): string {
    switch (by) {
      case 'client':
        return Band.harnessOf(call, sessions) ?? 'unknown';
      case 'provider':
        return call.provider;
      case 'model':
        return call.model ?? 'unknown';
      default:
        return assertNever(by);
    }
  }

  /** What a session counts toward, grouped `by`: its client, provider, or newest model. */
  static sessionKey(by: BandBy, session: SessionView, model: string | null): string {
    switch (by) {
      case 'client':
        return Board.harness(session.source) ?? 'unknown';
      case 'provider':
        return session.provider;
      case 'model':
        return model ?? 'unknown';
      default:
        return assertNever(by);
    }
  }

  static label(by: BandBy, key: string): string {
    switch (by) {
      case 'client':
        return (HARNESSES as readonly string[]).includes(key)
          ? harnessLabel(key as Harness)
          : 'Unknown client';
      case 'provider':
        return providerName(key as Stored<Provider>);
      case 'model':
        return key === 'unknown' ? 'Unknown model' : key;
      default:
        return assertNever(by);
    }
  }

  /** Clients keep the colour their cards use; providers and models take a slot's. */
  static swatch(by: BandBy, key: string, slot: number): string {
    return by === 'client' ? `harness-${key}` : `series-${slot}`;
  }

  /** The series a key is drawn in: its own, `other`, or none when it had no output. */
  seriesOf(key: string): BandSeries | null {
    return this.series.find((series) => series.members.includes(key)) ?? null;
  }

  /** A round scale above a peak, never below 50 tok/s, so a trickle does not look like a flood. */
  static scale(peak: number): number {
    return Math.max(50, Math.ceil((peak * 1.1) / 50) * 50);
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

  /**
   * The series at `index` drawn alone in its own slice of a 1000 × `height`
   * box: the slices split the height evenly, first at the top.
   */
  row(index: number, height: number): { line: string; area: string; top: number } {
    const slice = height / Math.max(1, this.series.length);
    const top = index * slice;
    const bins = this.series[index]?.bins ?? [];
    const points = Band.points(bins, this.max, slice, top);
    const base = (top + slice).toFixed(1);
    return {
      line: `M${points.join(' L')}`,
      area: `M${points.join(' L')} L1000,${base} L0,${base} Z`,
      top,
    };
  }

  /**
   * The ribbon layer of the series at `index`: series stack in order, the
   * first at the bottom, so the top edge of the last layer is all of them.
   */
  layer(index: number, height: number): { line: string; area: string } {
    const sum = (upTo: number) =>
      this.series[0]?.bins.map((_, bin) =>
        this.series.slice(0, upTo).reduce((total, series) => total + (series.bins[bin] ?? 0), 0),
      ) ?? [];
    const upper = Band.points(sum(index + 1), this.ribbonMax, height);
    const lower = Band.points(sum(index), this.ribbonMax, height).reverse();
    return { line: `M${upper.join(' L')}`, area: `M${upper.join(' L')} L${lower.join(' L')} Z` };
  }

  /** Bins as points across a 1000-wide box, held flat to both edges. */
  static points(bins: number[], max: number, height: number, top = 0): string[] {
    const step = 1000 / Math.max(1, bins.length);
    const y = (value: number) => (top + height - (value / max) * (height - 3)).toFixed(1);
    return [
      `0,${y(bins[0] ?? 0)}`,
      ...bins.map((value, index) => `${(index * step + step / 2).toFixed(1)},${y(value)}`),
      `1000,${y(bins.at(-1) ?? 0)}`,
    ];
  }
}
