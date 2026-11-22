import {
  CallId,
  ManagementApi,
  parseSearchField,
  type CallView,
  type ExposureView,
  type Health,
  type Metrics,
  type PageCursor,
  type RecordingId,
  type SearchField,
  type SearchPage,
  type Shelf,
  type RecordingReport,
  type RecordingView,
  type ReleaseView,
  type Storage,
  type TurnActivity,
  type TurnId,
  type TurnSummary,
  type UsageView,
  type Project,
} from './api.ts';
import {
  RunsClient,
  Sessions,
  type RunClient,
  type RunSnapshot,
  type Read,
  type SessionBody,
} from './runs.ts';
import { WorkApi, type WorkItem } from './work.ts';
import {
  Sparkline,
  localDayStart,
  parseRange,
  rangeStart,
  usagePoints,
  type UsageRange,
} from './usage.ts';
import { listingQuery } from './traffic.ts';
import { readSnapshot, snapshotError, type Snapshot } from './snapshots.ts';
import { Arrival, type Arriving } from './arrival.ts';
import { Band, Board, LivePlace, type LiveWindow } from './board.ts';
import { RecordingList, type RecordingDetail } from './recordings.ts';
import { changed, query } from './query.ts';
import type { RawCallQuery, RawSearchQuery } from './api.ts';

export interface GatewayData {
  health: Health;
  storage: Storage;
  /** Null until the factory has run, and from a gateway that predates `/api/release`. */
  release: ReleaseView | null;
}
/** A `?call=` deep link to a call that is not on the current page. */
export type OffPageSelection =
  { kind: 'none' } | { kind: 'loaded'; call: CallView } | { kind: 'failed'; message: string };
export interface TrafficData {
  calls: CallView[];
  next: PageCursor | null;
  recording: RecordingView | undefined;
  /** What the listed recordings are about: their newest prompt and folder. */
  details: Map<RecordingId, RecordingDetail>;
  /** Recordings for the picker; empty on a recording page. */
  recordings: RecordingView[];
  /** How many calls the default "model calls only" hides (at most 100), or null when not hiding. */
  hidden: number | null;
}

/** Listing filters `/api/calls` accepts, besides `limit`. */
const callFilters: readonly (keyof RawCallQuery & string)[] = [
  'q',
  'recording',
  'project',
  'client',
  'user',
  'provider',
  'model',
  'tool',
  'status',
  'has_tools',
  'from_ms',
  'until_ms',
  'after',
] as const;

export function gatewaySnapshot(fetcher: typeof fetch) {
  const api = new ManagementApi(fetcher);
  return Arrival.read('gateway', async (): Promise<GatewayData> => {
    // Health reads recorder counters, not storage, so the two run together.
    const [health, storage, release] = await Promise.all([
      api.health(),
      api.storage(),
      api.release().catch(() => null),
    ]);
    return { health, storage, release };
  });
}
export interface RecorderData {
  metrics: Metrics;
  /** The start of the hour of the oldest retained call; null when none, or unread. */
  oldest: number | null;
}

/** The Recorder page: its counters, and the oldest retained call for the history limit. */
export function recorderSnapshot(fetcher: typeof fetch) {
  const api = new ManagementApi(fetcher);
  return Arrival.read('recorder', async (): Promise<RecorderData> => {
    const [metrics, retained] = await Promise.all([
      api.metrics(),
      // Usage over every retained call lists its hours oldest first.
      api.usage({}).catch(() => null),
    ]);
    return { metrics, oldest: retained?.hours[0]?.start ?? null };
  });
}
export interface UsageData {
  range: UsageRange;
  /** When the window was computed, so the chart's last bar is "now". */
  now: number;
  usage: UsageView;
  /** The most-used recordings in the window's listing (recordings are not range-filtered). */
  recordings: RecordingView[];
  /** What the busiest recordings are about. */
  details: Map<RecordingId, RecordingDetail>;
  /**
   * Bookkeeping calls that failed in the range (a client's own checks, such
   * as Claude Code's quota probe), which the errors total leaves out; one
   * page is read, so 100 means that many or more. Null when not read.
   */
  bookkeepingErrors: number | null;
}

/** The Usage page: totals for the range, and the recordings list for its top recordings; handed
 * over pending on a navigation in the browser (`Arrival`). */
export function usageSnapshot(fetcher: typeof fetch, rawRange: string | null) {
  const api = new ManagementApi(fetcher);
  const range = parseRange(rawRange);
  return Arrival.read(`usage?${range}`, async (): Promise<UsageData> => {
    const now = Date.now();
    const from = rangeStart(range, now, localDayStart);
    const bookkeeping = query<RawCallQuery>({
      status: 'errors',
      kind: 'other',
      limit: 100,
      from_ms: from,
    });
    const [usage, recordings, bookkeepingErrors] = await Promise.all([
      api.usage({ from_ms: from }),
      api.recordings(100),
      api
        .calls(bookkeeping)
        .then((page) => page.calls.length)
        .catch(() => null),
    ]);
    const busiest = recordings.recordings
      .toSorted((a, b) => (b.tokens.input ?? 0) - (a.tokens.input ?? 0))
      .map((recording) => recording.id);
    const details = await RecordingDetails.shared.read(
      api,
      // The page shows five, after filtering by range: read a few more.
      RecordingDetails.pick(recordings.recordings, busiest, 10),
    );
    return { range, now, usage, recordings: recordings.recordings, details, bookkeepingErrors };
  });
}

export interface OverviewData {
  now: number;
  /** The last 24 hours. */
  usage: UsageView;
  /** The 24 hours before that, for the change in calls; null when it could not be read. */
  previous: UsageView | null;
  /** Model calls that failed in the last 24 hours. */
  failed: CallView[];
  /** Model calls cut off in the last 24 hours. */
  stopped: CallView[];
  /** Bookkeeping calls that failed in the last 24 hours: quota probes, token counts. */
  bookkeeping: CallView[];
  recordings: RecordingView[];
  /** Credentials found in the last 24 hours; null from a daemon without them. */
  exposures: ExposureView[] | null;
  /** The Live board's last 15 minutes, for the recordings at work; null when unread. */
  live: LiveData | null;
}

/**
 * The overview: everything read at once; each read is small. `live` keeps
 * the Live board's turn cache between refreshes in the browser.
 */
export function overviewSnapshot(
  fetcher: typeof fetch,
  live: LiveReader,
  project: string | null = null,
) {
  const api = new ManagementApi(fetcher);
  return Arrival.read(
    project ? `overview:${project}` : 'overview',
    async (): Promise<OverviewData> => {
      const now = Date.now();
      const day = 24 * 3_600_000;
      const since = now - day;
      const listing = (status: string, kind: string) =>
        api
          .calls(query<RawCallQuery>({ status, kind, limit: 100, from_ms: since, project }))
          .then((page) => page.calls);
      const [usage, previous, failed, stopped, bookkeeping, recordings, exposures, board] =
        await Promise.all([
          api.usage({ from_ms: rangeStart('24h', now, localDayStart), project }),
          api.usage({ from_ms: now - 2 * day, until_ms: now - day, project }).catch(() => null),
          listing('errors', 'model'),
          listing('interrupted', 'model'),
          listing('errors', 'other'),
          api.recordings(100, undefined, 'current', project),
          api.exposures(since, undefined, project).then((page) => page.exposures),
          live.read(fetcher, '15m', project),
        ]);
      return {
        now,
        usage,
        previous,
        failed,
        stopped,
        bookkeeping,
        recordings: recordings.recordings,
        exposures,
        live: board.value,
      };
    },
  );
}

export interface RecordingsData {
  recordings: RecordingView[];
  /** Model calls in flight, for the rows' state. */
  open: CallView[];
  /** Each detailed recording's newest turn: its prompt and folder. */
  details: Map<RecordingId, RecordingDetail>;
}

/**
 * Reads the Recordings page. A recording's newest turn says what it is about,
 * and costs two reads (its summary, then its activity for the folder), so
 * a recording is read again only when it has been seen since; recordings of a
 * single call with no turn are not read at all.
 */
export class RecordingsReader {
  /** The newest recordings read in detail; the rest show without a prompt. */
  static readonly detailed = 40;
  readonly #details: RecordingDetails;

  constructor(details: RecordingDetails = RecordingDetails.shared) {
    this.#details = details;
  }

  /** The recordings on `shelf`, current or archived; with `project`, only
   * its sessions' runs'. */
  read(
    fetcher: typeof fetch,
    shelf: Shelf = 'current',
    project: string | null = null,
  ): Promise<Arriving<RecordingsData>> {
    const api = new ManagementApi(fetcher);
    // Keyed by shelf and project: a refresh keeps its rows, but another
    // shelf or project is another query.
    return Arrival.read(
      `recordings:${shelf}:${project ?? ''}`,
      async (): Promise<RecordingsData> => {
        const [listing, open] = await Promise.all([
          api.recordings(100, undefined, shelf, project),
          api
            .calls(query<RawCallQuery>({ status: 'active', kind: 'model', limit: 100 }))
            .then((page) => page.calls),
        ]);
        const wanted = listing.recordings
          .filter((recording) => !RecordingList.trivial(recording))
          .slice(0, RecordingsReader.detailed);
        const details = await this.#details.read(api, wanted);
        return { recordings: listing.recordings, open, details };
      },
    );
  }
}

/**
 * What each recording is about, read from its newest turns: the newest
 * prompt, and the folder its newest turn's commands work in. A recording is
 * read again only once it has been seen since, so one instance kept in the
 * browser serves every page that names recordings.
 */
export class RecordingDetails {
  /** Recordings read at once. */
  static readonly parallel = 8;
  /** Turns read per recording, newest first, to find one with a prompt. */
  static readonly recentTurns = 5;
  /**
   * One cache for every page. It is keyed by when each recording was last seen,
   * so sharing it never shows a stale prompt, and the API it reads is the same
   * for every viewer.
   */
  static readonly shared = new RecordingDetails();
  readonly #cache = new Map<RecordingId, { lastSeen: number; detail: RecordingDetail }>();

  /** Details for these recordings; one whose turns cannot be read is left out. */
  async read(
    api: ManagementApi,
    recordings: RecordingView[],
  ): Promise<Map<RecordingId, RecordingDetail>> {
    const details = new Map<RecordingId, RecordingDetail>();
    const readOne = async (recording: RecordingView) => {
      const cached = this.#cache.get(recording.id);
      if (cached?.lastSeen === recording.last_seen) {
        details.set(recording.id, cached.detail);
        return;
      }
      try {
        // A turn can carry no prompt of its own (it continues the one before), so
        // the newest few are read and the newest with a prompt names the recording.
        const turns = (await api.turns(recording.id, RecordingDetails.recentTurns)).turns;
        const newest = turns[0];
        const activity = newest ? await api.turn(newest.turn) : null;
        const detail = {
          prompt: turns.find((turn) => turn.prompt?.text.trim())?.prompt ?? null,
          workdir: activity ? Board.workdir(activity) : null,
        };
        this.#cache.set(recording.id, { lastSeen: recording.last_seen, detail });
        details.set(recording.id, detail);
      } catch {
        // A recording whose turn cannot be read shows without its prompt, not the page empty.
        if (cached) details.set(recording.id, cached.detail);
      }
    };
    // A few at a time, so a first load does not send every recording's reads at once.
    for (let index = 0; index < recordings.length; index += RecordingDetails.parallel)
      await Promise.all(recordings.slice(index, index + RecordingDetails.parallel).map(readOne));
    return details;
  }

  /** The recordings among `listing` that these IDs name, at most `limit`, first named first. */
  static pick(listing: RecordingView[], ids: (RecordingId | null)[], limit = 40): RecordingView[] {
    const byId = new Map(listing.map((recording) => [recording.id, recording]));
    const picked = new Map<RecordingId, RecordingView>();
    for (const id of ids) {
      const recording = id ? byId.get(id) : undefined;
      if (recording && !RecordingList.trivial(recording)) picked.set(recording.id, recording);
      if (picked.size >= limit) break;
    }
    return [...picked.values()];
  }
}

/** A listing of calls, handed over pending on a navigation in the browser (`Arrival`). */
export async function trafficSnapshot(
  fetcher: typeof fetch,
  url: URL,
  recordingId: RecordingId | null,
) {
  const api = new ManagementApi(fetcher);
  // Read by name: Kit re-runs a load on any URL change once it reads the
  // whole URL (even `url.pathname`), but only on a named parameter's change
  // when it reads parameters by name. So the key comes from the route, and
  // the listing never reads `call`: selecting a call must not re-read it.
  const named = new URLSearchParams();
  for (const key of [...callFilters, 'kind', 'since']) {
    const value = url.searchParams.get(key);
    if (value) named.set(key, value);
  }
  const key = `${recordingId ? `/recordings/${recordingId}` : '/traffic'}?${named}`;
  return Arrival.read(key, async (): Promise<TrafficData> => {
    if (recordingId) {
      // A recording's own listing accepts only a cursor.
      const [recordings, page] = await Promise.all([
        api.recordings(100),
        api.recordingCalls(recordingId, named.get('after')),
      ]);
      const recording = recordings.recordings.find((item) => item.id === recordingId);
      const details = await RecordingDetails.shared.read(
        api,
        RecordingDetails.pick(recordings.recordings, [recordingId]),
      );
      return {
        calls: page.calls,
        next: page.next,
        recording,
        recordings: [],
        hidden: null,
        details,
      };
    }
    const params = listingQuery(named, callFilters, Date.now());
    const hiding = params.get('kind') === 'model';
    const hiddenQuery = changed(params, { kind: 'other', limit: 100, after: null });
    const [page, recordings, hidden] = await Promise.all([
      api.calls(params),
      // The picker offers the scope's recordings: another's would list none.
      api.recordings(100, undefined, 'current', named.get('project')),
      hiding ? api.calls(hiddenQuery).then((other) => other.calls.length) : Promise.resolve(null),
    ]);
    const details = await RecordingDetails.shared.read(
      api,
      RecordingDetails.pick(
        recordings.recordings,
        page.calls.map((call) => call.recording),
      ),
    );
    return {
      calls: page.calls,
      next: page.next,
      recording: undefined,
      recordings: recordings.recordings,
      hidden,
      details,
    };
  });
}

/**
 * A `?call=` link to a call that is not in `calls`: read that call alone.
 * The workbench calls this, not the page load, so changing the selection
 * never re-reads the listing.
 */
export async function offPageSelection(
  fetcher: typeof fetch,
  rawCall: string | null,
  calls: CallView[],
  signal?: AbortSignal,
): Promise<OffPageSelection> {
  if (rawCall === null) return { kind: 'none' };
  const callId = CallId.parse(rawCall);
  if (!callId) return { kind: 'failed', message: 'The linked call ID is invalid.' };
  if (calls.some((call) => call.id === callId)) return { kind: 'none' };
  try {
    return { kind: 'loaded', call: (await new ManagementApi(fetcher).call(callId, signal)).call };
  } catch (error) {
    return {
      kind: 'failed',
      message: snapshotError(error, 'Unable to load the selected call.').message,
    };
  }
}

export interface ReportData {
  report: RecordingReport;
  /** The turn listing, for prompts and error counts. */
  turns: TurnSummary[];
  recording: RecordingView | undefined;
  /** What the recording is about, for its header. */
  detail: RecordingDetail | null;
}

/** A recording's report, with its turns' prompts and its listing row. */
export function reportSnapshot(fetcher: typeof fetch, recordingId: RecordingId) {
  const api = new ManagementApi(fetcher);
  return Arrival.read(`/recordings/${recordingId}/report`, async (): Promise<ReportData> => {
    const [report, turns, recordings] = await Promise.all([
      api.report(recordingId),
      api.turns(recordingId, 100),
      api.recordings(100),
    ]);
    const recording = recordings.recordings.find((item) => item.id === recordingId);
    const details = await RecordingDetails.shared.read(
      api,
      RecordingDetails.pick(recordings.recordings, [recordingId]),
    );
    return { report, turns: turns.turns, recording, detail: details.get(recordingId) ?? null };
  });
}

export interface SearchData {
  /** The text searched for; empty before a search. */
  q: string;
  field: SearchField | null;
  /** Null before a search. */
  page: SearchPage | null;
  /** What the hits' recordings are about, and the recordings themselves. */
  details: Map<RecordingId, RecordingDetail>;
  recordings: RecordingView[];
}

/** A search from the page's URL: `?q=` and optionally `?field=`. */
export function searchSnapshot(fetcher: typeof fetch, url: URL) {
  const api = new ManagementApi(fetcher);
  const q = (url.searchParams.get('q') ?? '').trim();
  const field = parseSearchField(url.searchParams.get('field'));
  const project = url.searchParams.get('project') || null;
  const key = new URLSearchParams({ q, field: field ?? '' });
  if (project) key.set('project', project);
  return Arrival.read(`/search?${key}`, async (): Promise<SearchData> => {
    if (!q) return { q, field, page: null, details: new Map(), recordings: [] };
    const params = query<RawSearchQuery>({ q, limit: 50, field, project });
    // The recordings only name the hits: without them, the hits still show.
    const [page, listing] = await Promise.all([
      api.search(params),
      api.recordings(100, undefined, 'current', project).catch(() => ({ recordings: [] })),
    ]);
    const details = await RecordingDetails.shared.read(
      api,
      RecordingDetails.pick(
        listing.recordings,
        page.hits.map((hit) => hit.recording),
      ),
    );
    return { q, field, page, details, recordings: listing.recordings };
  });
}

export interface LiveData {
  /** When the snapshot was read: elapsed times count from the page's own clock. */
  read: number;
  recordings: RecordingView[];
  /** Model calls started in the board's window, and every call still open. */
  calls: CallView[];
  /** Model calls started in the band's span or the window, whichever is longer, and every call still open. */
  recent: CallView[];
  /** Every call started from this time on was read; before it, the reads came back full. */
  complete: number;
  /** The newest turn of each recording shown, where it was read. */
  turns: Map<TurnId, TurnActivity>;
  window: LiveWindow;
  /** A listing came back full: older calls or recordings in the window were not read. */
  truncated: { calls: boolean; recordings: boolean };
}

/**
 * Model calls started within a span back from now, kept between reads so a
 * read fetches only what can have changed: calls started since the previous
 * read, and every call that had not settled. A settled call (finished, with
 * its digest derived) does not change again.
 */
export class RecentCalls {
  static readonly pageLimit = 1000;
  static readonly maxPages = 5;
  /** Recording is asynchronous: each read looks this far behind the previous one. */
  static readonly overlapMs = 60_000;

  readonly #calls = new Map<CallId, CallView>();
  /** The project the held calls are narrowed to; another starts afresh. */
  #project: string | null = null;
  #last: number | null = null;
  #complete: number | null = null;
  /** The previous read came back full: what it could not hold is not read again. */
  #capped = false;

  static settled(call: CallView): boolean {
    return call.ended_at !== null && call.purpose !== null;
  }

  /** Where this read starts: the whole span when nothing usable is held. */
  cutoff(now: number, spanMs: number): number {
    const from = now - spanMs;
    if (this.#last === null || this.#complete === null) return from;
    // The span grew past what is held (a longer window): read it whole.
    if (this.#complete > from && !this.#capped) return from;
    let cutoff = this.#last - RecentCalls.overlapMs;
    for (const call of this.#calls.values())
      if (!RecentCalls.settled(call)) cutoff = Math.min(cutoff, call.started_at);
    return Math.max(from, cutoff);
  }

  async read(
    api: ManagementApi,
    now: number,
    spanMs: number,
    project: string | null = null,
  ): Promise<{ calls: CallView[]; complete: number }> {
    if (project !== this.#project) {
      this.#project = project;
      this.#calls.clear();
      this.#last = null;
      this.#complete = null;
    }
    const from = now - spanMs;
    const cutoff = this.cutoff(now, spanMs);
    const open = api.calls(
      query<RawCallQuery>({ kind: 'model', status: 'active', limit: 100, project }),
    );
    const fresh: CallView[] = [];
    let after: PageCursor | null = null;
    let full = false;
    for (let page = 0; page < RecentCalls.maxPages; page++) {
      const listing = await api.calls(
        query<RawCallQuery>({
          kind: 'model',
          from_ms: cutoff,
          limit: RecentCalls.pageLimit,
          after,
          project,
        }),
      );
      fresh.push(...listing.calls);
      after = listing.next;
      if (!after) break;
      full = page === RecentCalls.maxPages - 1;
    }
    const openCalls = (await open).calls;

    // Pages run newest first: a capped read holds nothing older than its last call.
    const oldest = fresh.at(-1)?.started_at ?? now;
    if (cutoff === from || full) this.#calls.clear();
    this.#complete = full ? oldest : cutoff === from ? from : (this.#complete ?? from);
    for (const [id, call] of this.#calls) {
      // Re-read now, or open earlier and since gone from the open listing.
      if (call.started_at >= cutoff || !RecentCalls.settled(call)) this.#calls.delete(id);
      else if (call.started_at < from) this.#calls.delete(id);
    }
    for (const call of [...fresh, ...openCalls]) this.#calls.set(call.id, call);
    this.#last = now;
    this.#capped = full;
    return { calls: [...this.#calls.values()], complete: Math.max(from, this.#complete) };
  }
}

/**
 * Reads the Live page. Turn activity is the costly read, and it changes
 * only when one of the turn's calls does, so a turn is read again only
 * then, or while some of its text is still being derived.
 */
export class LiveReader {
  readonly #calls = new RecentCalls();
  readonly #turns = new Map<TurnId, { signature: string; turn: TurnActivity }>();

  /** A board's snapshot key: its window, and its project when narrowed. */
  static key(window: LiveWindow, project: string | null): string {
    return project ? `live:${window}:${project}` : `live:${window}`;
  }

  /** The Live page's read, handed over pending on a navigation in the
   * browser (`Arrival`); with `project`, only that project's. */
  arrive(
    fetcher: typeof fetch,
    window: LiveWindow,
    project: string | null = null,
  ): Promise<Arriving<LiveData>> {
    return Arrival.hand(LiveReader.key(window, project), this.read(fetcher, window, project));
  }

  /** The board's snapshot; the overview reads it whole, inside its own. */
  read(
    fetcher: typeof fetch,
    window: LiveWindow,
    project: string | null = null,
  ): Promise<Snapshot<LiveData>> {
    const api = new ManagementApi(fetcher);
    const spec = LivePlace.windows[window];
    return readSnapshot(LiveReader.key(window, project), async (): Promise<LiveData> => {
      const read = Date.now();
      const [recordings, held] = await Promise.all([
        api.recordings(spec.recordings, undefined, 'current', project),
        this.#calls.read(api, read, Math.max(spec.ms, Band.spanMs), project),
      ]);
      const recent = held.calls;
      const calls = recent.filter(
        (call) => call.ended_at === null || call.started_at >= read - spec.ms,
      );
      const byRecording = Board.byRecording(calls);
      const shown = Board.shown(
        Board.activeRecordings(recordings.recordings, byRecording, read, spec.ms),
        byRecording,
      ).map((recording) => recording.id);
      const turns = new Map<TurnId, TurnActivity>();
      await Promise.all(
        Board.turnsToRead(shown, calls).map(async ({ turn, signature }) => {
          const cached = this.#turns.get(turn);
          const settled =
            cached?.signature === signature &&
            cached.turn.calls.every((call) => !call.limitations.includes('not_yet_derived'));
          if (cached && settled) {
            turns.set(turn, cached.turn);
            return;
          }
          try {
            const activity = await api.turn(turn);
            this.#turns.set(turn, { signature, turn: activity });
            turns.set(turn, activity);
          } catch {
            // A turn that cannot be read leaves its tile without text, not the page empty.
            if (cached) turns.set(turn, cached.turn);
          }
        }),
      );
      for (const turn of this.#turns.keys()) if (!turns.has(turn)) this.#turns.delete(turn);
      // Recordings come newest first: a full page whose oldest is still in the window may miss some.
      const oldest = recordings.recordings.at(-1);
      return {
        read,
        recordings: recordings.recordings,
        calls,
        recent,
        complete: held.complete,
        turns,
        window,
        truncated: {
          calls: held.complete > read - spec.ms,
          recordings:
            recordings.recordings.length >= spec.recordings &&
            oldest !== undefined &&
            read - oldest.last_seen <= spec.ms,
        },
      };
    });
  }
}

/** What every page reads for Pending, and the Projects view for its board.
 * Each list says whether it read: a list that failed is not an empty one. */
export interface ProjectsData {
  projects: Project[];
  /** Why the projects did not read, when they did not. */
  failed: string | null;
  /** Work items under way, across every project. */
  work: Read<WorkItem[]>;
  /** The runs the daemon holds, for the approvals they wait on; none when
   * this UI only reads. */
  live: Read<RunSnapshot[]>;
  /** The current sessions, for the project each run works for. */
  sessions: Read<SessionBody[]>;
  /** Each client and its sign-in on the daemon's host; none when this UI
   * only reads. */
  clients: Read<RunClient[]>;
}

export async function projectsData(
  fetcher: typeof fetch,
  control: 'drives' | 'reads',
): Promise<ProjectsData> {
  const api = new ManagementApi(fetcher);
  const runs = new RunsClient(fetcher);
  const settle = <T>(reading: Promise<T>): Promise<Read<T>> =>
    reading.then(
      (value): Read<T> => ({ state: 'read', value }),
      (failure: unknown): Read<T> => ({
        state: 'failed',
        reason: failure instanceof Error ? failure.message : 'The read failed.',
      }),
    );
  const none = <T>(): Promise<Read<T[]>> => Promise.resolve({ state: 'read', value: [] });
  const [listed, work, live, sessions, clients] = await Promise.all([
    api.projects().then(
      (projects) => ({ projects, failed: null }),
      (failure: unknown) => ({
        projects: [],
        failed: failure instanceof Error ? failure.message : 'The projects did not read.',
      }),
    ),
    settle(new WorkApi(fetcher).list('current')),
    control === 'drives' ? settle(runs.live()) : none<RunSnapshot>(),
    control === 'drives' ? settle(runs.sessions('current')) : none<SessionBody>(),
    control === 'drives' ? settle(runs.clients()) : none<RunClient>(),
  ]);
  return { ...listed, work, live, sessions, clients };
}

/** Which recordings are sessions Ren drives, for the links between Monitor
 * and their desks: none when it drives none here, or they did not read. It
 * arrives like any page's read, so the page never waits on the links. */
export function deskLinks(fetcher: typeof fetch): Promise<Arriving<Map<RecordingId, string>>> {
  return Arrival.read('desks', async () => {
    const [bodies, projects] = await Promise.all([
      new RunsClient(fetcher).sessions().catch(() => []),
      new ManagementApi(fetcher).projects().catch(() => []),
    ]);
    return Sessions.desks(bodies, projects);
  });
}

/** Each current project's calls by hour over the last day, as sparklines
 * 96 × 24, by project name. A project whose usage did not read has none. */
export function projectSparks(
  fetcher: typeof fetch,
  now: number,
): Promise<Arriving<Map<string, Sparkline>>> {
  const api = new ManagementApi(fetcher);
  return Arrival.read('projects:sparks', async () => {
    const projects = (await api.projects()).filter((project) => project.archived_at === null);
    const read = await Promise.all(
      projects.map(async (project) => {
        const usage = await api
          .usage({ from_ms: now - 86_400_000, project: project.name })
          .catch(() => null);
        return usage
          ? ([project.name, Sparkline.of(usagePoints(usage.hours, '24h', now), 96, 24)] as const)
          : null;
      }),
    );
    return new Map(read.filter((entry) => entry !== null));
  });
}
