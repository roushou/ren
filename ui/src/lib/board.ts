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
import { HARNESSES, providerName } from './api.ts';
import { findingLine, harnessLabel, resultBadge } from './activity.ts';
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
  /** How far back a session counts as active. */
  windowMs: number;
}

export class Board {
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
  /** The Now strip: what happened lately, newest first. */
  readonly feed: FeedItem[];
  readonly now: number;

  constructor(input: BoardInput) {
    this.now = input.now;
    const bySession = Board.bySession(input.calls);
    const active = Board.activeSessions(input.sessions, bySession, input.now, input.windowMs);
    this.overflow = Math.max(0, active.length - Board.maxTiles);
    this.tiles = Board.shown(active, bySession).map((session) =>
      Board.tile(session, bySession.get(session.id) ?? [], input.turns, input.now),
    );
    this.alerts = this.tiles
      .flatMap((tile) => Board.alerts(tile, bySession.get(tile.session.id) ?? [], input.now))
      .sort((a, b) => b.at - a.at);
    this.feed = new Feed(input, this.alerts).items;
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

  static parse(params: { get(name: string): string | null }): LiveView {
    const window = params.get('window');
    return {
      density: params.get('density') === 'compact' ? 'compact' : 'comfortable',
      band: params.get('band') === 'ribbon' ? 'ribbon' : 'area',
      window: LivePlace.isWindow(window) ? window : LivePlace.defaultWindow,
    };
  }

  static isWindow(text: string | null): text is LiveWindow {
    return text !== null && Object.hasOwn(LivePlace.windows, text);
  }

  static href(view: LiveView): string {
    const params = new URLSearchParams();
    if (view.window !== LivePlace.defaultWindow) params.set('window', view.window);
    if (view.density === 'compact') params.set('density', 'compact');
    if (view.band === 'ribbon') params.set('band', 'ribbon');
    const query = params.toString();
    return query ? `/live?${query}` : '/live';
  }

  /** The next longer window, to offer when the board is empty. */
  static longer(window: LiveWindow): LiveWindow | null {
    const order = Object.keys(LivePlace.windows) as LiveWindow[];
    return order[order.indexOf(window) + 1] ?? null;
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
  /** The ribbon's scale: every client stacked, in tokens per second. */
  readonly ribbonMax: number;

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
    this.max = Band.scale(Math.max(0, ...this.rows.flatMap((row) => row.bins)));
    const totals = Array.from({ length: count }, (_, index) =>
      this.rows.reduce((sum, row) => sum + (row.bins[index] ?? 0), 0),
    );
    this.ribbonMax = Band.scale(Math.max(0, ...totals));
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

  /** A row's bins as an SVG line in a 1000 × `height` box. */
  line(row: BandRow, height: number): string {
    return `M${Band.points(row.bins, this.max, height).join(' L')}`;
  }

  /** The same line closed down to the baseline, for the fill under it. */
  area(row: BandRow, height: number): string {
    return `${this.line(row, height)} L1000,${height} L0,${height} Z`;
  }

  /**
   * The ribbon layer of the row at `index`: rows stack in order, the first at
   * the bottom, so the top edge of the last layer is every client together.
   */
  layer(index: number, height: number): { line: string; area: string } {
    const sum = (upTo: number) =>
      this.rows[0]?.bins.map((_, bin) =>
        this.rows.slice(0, upTo).reduce((total, row) => total + (row.bins[bin] ?? 0), 0),
      ) ?? [];
    const upper = Band.points(sum(index + 1), this.ribbonMax, height);
    const lower = Band.points(sum(index), this.ribbonMax, height).reverse();
    return { line: `M${upper.join(' L')}`, area: `M${upper.join(' L')} L${lower.join(' L')} Z` };
  }

  /** Bins as points across a 1000-wide box, held flat to both edges. */
  static points(bins: number[], max: number, height: number): string[] {
    const step = 1000 / bins.length;
    const y = (value: number) => (height - (value / max) * (height - 3)).toFixed(1);
    return [
      `0,${y(bins[0] ?? 0)}`,
      ...bins.map((value, index) => `${(index * step + step / 2).toFixed(1)},${y(value)}`),
      `1000,${y(bins.at(-1) ?? 0)}`,
    ];
  }
}
