// Agent runs: what the daemon's control API says about them (through this
// UI server's `/control/runs` proxy), what the recorded history keeps, and
// the Runs view's pure logic: the live conversation built from run events,
// what each run can be asked, and how the sidebar groups runs by folder.
// No browser APIs beyond the `fetch` handed in; tested in tests/runs.test.ts.
import {
  API_VERSION,
  SessionId,
  versioned,
  type Excerpt,
  type Harness,
  type NativeThreadId,
  type NativeTurnId,
  type UnixMillis,
} from './api.ts';
import {
  Uuid,
  array,
  assertNever,
  branded,
  brandedCount,
  isRecord,
  label,
  nullable,
  object,
  string,
  unrecognised,
  ShapeError,
  type Brand,
  type Decoder,
  type Stored,
} from './decode.ts';

export type RunId = Brand<string, 'RunId'>;
export const RunId = new Uuid<'RunId'>();
/** The client's own id for a request waiting on the operator. */
export type ApprovalId = Brand<string, 'ApprovalId'>;
/** A turn item, named as the client named it. */
export type ItemId = Brand<string, 'ItemId'>;

const harnesses = ['claude_code', 'codex', 'pi', 'opencode'] as const;
const runEnds = [
  'spawn_failed',
  'open_refused',
  'exited',
  'stopped',
  'drained',
  'recovered',
] as const;
export type RunEnd = (typeof runEnds)[number];
const stopCauses = ['operator', 'shutdown', 'open_refused'] as const;
export type StopCause = (typeof stopCauses)[number];
const decisions = ['accept', 'accept_for_session', 'decline', 'cancel'] as const;
export type Decision = (typeof decisions)[number];

/** How a run's process exited. */
export type ProcessExit =
  { kind: 'code'; value: number } | { kind: 'signal'; value: number } | { kind: 'unknown' };

/** What an approval asks the operator to allow. */
export type ApprovalAsk =
  | { kind: 'command'; command: string | null; cwd: string | null; reason: string | null }
  | { kind: 'file_change'; reason: string | null }
  | { kind: 'tool'; tool: string; detail: string | null }
  | { kind: 'unrecognized'; text: string };

export interface Waiting {
  approval: ApprovalId;
  ask: ApprovalAsk;
}

/** A run's lifecycle as the daemon holds it now. */
export type RunState =
  | { phase: 'starting' }
  | { phase: 'idle'; thread: NativeThreadId }
  | {
      phase: 'working';
      thread: NativeThreadId;
      turn: NativeTurnId | null;
      approvals: Waiting[];
    }
  | { phase: 'stopping'; cause: Stored<StopCause> }
  | { phase: 'ended'; end: Stored<RunEnd>; exit: ProcessExit | null }
  /** A phase a newer daemon reports. */
  | { phase: 'unrecognized'; text: string };

export interface RunSnapshot {
  run: RunId;
  harness: Stored<Harness>;
  workdir: string;
  state: RunState;
}

/** Where an item's action stands; a status a newer daemon names is kept as text. */
export type ItemStatus = 'in_progress' | 'completed' | 'failed' | 'declined' | (string & {});

/** What the daemon read from an item, the same for every client. */
export type ItemRead =
  | { as: 'prompt'; text: string }
  | { as: 'reply'; text: string }
  | { as: 'reasoning' }
  | { as: 'plan'; text: string }
  | {
      as: 'command';
      command: string;
      status: ItemStatus;
      output: string | null;
      exit_code: number | null;
    }
  | { as: 'file_change'; paths: string[]; status: ItemStatus }
  | { as: 'tool'; name: string; status: ItemStatus }
  /** Nothing read, or a reading a newer daemon gives: shown by the item's kind. */
  | { as: 'unread' };

/** One item of a turn: its kind and what the daemon read of it. */
export interface DriveItem {
  id: ItemId;
  kind: string;
  read: ItemRead;
}

export type TurnEnd =
  | { kind: 'completed' }
  | { kind: 'interrupted' }
  | { kind: 'failed'; message: string | null }
  | { kind: 'unknown'; status: string };

/** What the agent reported, as the daemon relays it. Not stored. */
export type DriveEvent =
  | { kind: 'ready'; thread: NativeThreadId }
  | { kind: 'turn_started'; turn: NativeTurnId }
  | { kind: 'delta'; item: ItemId; text: string }
  | { kind: 'item'; phase: 'started' | 'completed'; item: DriveItem }
  | { kind: 'approval'; approval: ApprovalId; ask: ApprovalAsk }
  | { kind: 'approval_resolved'; approval: ApprovalId }
  | { kind: 'turn_ended'; turn: NativeTurnId; end: TurnEnd }
  | { kind: 'error'; message: string; retry: 'will_retry' | 'final' }
  | { kind: 'refused'; request: string; message: string }
  | { kind: 'unsupported'; method: string }
  /** Unknown, unreadable, or newer events: shown by their kind only. */
  | { kind: 'other'; name: string };

export type RunUpdate =
  { kind: 'state'; run: RunId; state: RunState } | { kind: 'event'; run: RunId; event: DriveEvent };

/** A run as recorded: its start, opening, and end. */
export interface RecordedRun {
  id: RunId;
  harness: Stored<Harness>;
  workdir: string;
  started_at: UnixMillis;
  thread: NativeThreadId | null;
  /** The session its model traffic was recorded under, once a call was. */
  session: SessionId | null;
  ended_at: UnixMillis | null;
  end: Stored<RunEnd> | null;
  exit: ProcessExit | null;
  /** When the operator archived it; null when it is not. */
  archived_at: UnixMillis | null;
}

/** Which part of a listing: what is current, or what was archived. */
export type Shelf = 'current' | 'archived';

/** What an archive or delete applies to. */
export type HistorySubject = { kind: 'session'; id: SessionId } | { kind: 'run'; id: RunId };

/** What a history edit does. */
export type HistoryChange = 'archive' | 'unarchive' | 'delete';

const optionalString: Decoder<string | null> = (value, path) =>
  value === undefined || value === null ? null : string(value, path);

const integer: Decoder<number> = (value, path) => {
  if (typeof value !== 'number' || !Number.isInteger(value))
    throw new ShapeError(path, 'an integer');
  return value;
};

const processExit: Decoder<ProcessExit> = (value, path) => {
  if (!isRecord(value)) throw new ShapeError(path, 'an object');
  switch (value.kind) {
    case 'code':
    case 'signal':
      return { kind: value.kind, value: integer(value.value, `${path}.value`) };
    default:
      return { kind: 'unknown' };
  }
};

const approvalAsk: Decoder<ApprovalAsk> = (value, path) => {
  if (!isRecord(value)) throw new ShapeError(path, 'an object');
  switch (value.kind) {
    case 'command':
      return {
        kind: 'command',
        command: optionalString(value.command, `${path}.command`),
        cwd: optionalString(value.cwd, `${path}.cwd`),
        reason: optionalString(value.reason, `${path}.reason`),
      };
    case 'file_change':
      return { kind: 'file_change', reason: optionalString(value.reason, `${path}.reason`) };
    case 'tool':
      return {
        kind: 'tool',
        tool: string(value.tool, `${path}.tool`),
        detail: optionalString(value.detail, `${path}.detail`),
      };
    default:
      return { kind: 'unrecognized', text: String(value.kind) };
  }
};

const waiting = object<Waiting>({ approval: branded<'ApprovalId'>(), ask: approvalAsk });

const runState: Decoder<RunState> = (value, path) => {
  if (!isRecord(value)) throw new ShapeError(path, 'an object');
  const thread = () => branded<'NativeThreadId'>()(value.thread, `${path}.thread`);
  switch (value.phase) {
    case 'starting':
      return { phase: 'starting' };
    case 'idle':
      return { phase: 'idle', thread: thread() };
    case 'working':
      return {
        phase: 'working',
        thread: thread(),
        turn: nullable(branded<'NativeTurnId'>())(value.turn ?? null, `${path}.turn`),
        approvals: array(waiting)(value.approvals ?? [], `${path}.approvals`),
      };
    case 'stopping':
      return { phase: 'stopping', cause: label(stopCauses)(value.cause, `${path}.cause`) };
    case 'ended':
      return {
        phase: 'ended',
        end: label(runEnds)(value.end, `${path}.end`),
        exit: value.exit == null ? null : processExit(value.exit, `${path}.exit`),
      };
    default:
      return { phase: 'unrecognized', text: String(value.phase) };
  }
};

const itemRead: Decoder<ItemRead> = (value, path) => {
  if (!isRecord(value)) return { as: 'unread' };
  const at = (key: string) => `${path}.${key}`;
  const status = () => string(value.status, at('status'));
  switch (value.as) {
    case 'prompt':
    case 'reply':
    case 'plan':
      return { as: value.as, text: string(value.text, at('text')) };
    case 'reasoning':
      return { as: 'reasoning' };
    case 'command':
      return {
        as: 'command',
        command: string(value.command, at('command')),
        status: status(),
        output: optionalString(value.output, at('output')),
        exit_code: value.exit_code == null ? null : integer(value.exit_code, at('exit_code')),
      };
    case 'file_change':
      return {
        as: 'file_change',
        paths: array(string)(value.paths, at('paths')),
        status: status(),
      };
    case 'tool':
      return { as: 'tool', name: string(value.name, at('name')), status: status() };
    default:
      return { as: 'unread' };
  }
};

const runSnapshot = object<RunSnapshot>({
  run: RunId.decode,
  harness: label(harnesses),
  workdir: string,
  state: runState,
});

const turnEnd: Decoder<TurnEnd> = (value, path) => {
  if (!isRecord(value)) throw new ShapeError(path, 'an object');
  switch (value.kind) {
    case 'completed':
    case 'interrupted':
      return { kind: value.kind };
    case 'failed':
      return { kind: 'failed', message: optionalString(value.message, `${path}.message`) };
    default:
      return { kind: 'unknown', status: String(value.status ?? value.kind) };
  }
};

const driveEvent: Decoder<DriveEvent> = (value, path) => {
  if (!isRecord(value)) throw new ShapeError(path, 'an object');
  const at = (key: string) => `${path}.${key}`;
  switch (value.kind) {
    case 'ready':
      return { kind: 'ready', thread: branded<'NativeThreadId'>()(value.thread, at('thread')) };
    case 'turn_started':
      return { kind: 'turn_started', turn: branded<'NativeTurnId'>()(value.turn, at('turn')) };
    case 'delta':
      return {
        kind: 'delta',
        item: branded<'ItemId'>()(value.item, at('item')),
        text: string(value.text, at('text')),
      };
    case 'item': {
      const item = value.item;
      if (!isRecord(item)) throw new ShapeError(at('item'), 'an object');
      const phase = value.phase === 'completed' ? 'completed' : 'started';
      return {
        kind: 'item',
        phase,
        item: {
          id: branded<'ItemId'>()(item.id, at('item.id')),
          kind: string(item.kind, at('item.kind')),
          read: itemRead(item.read, at('item.read')),
        },
      };
    }
    case 'approval':
      return {
        kind: 'approval',
        approval: branded<'ApprovalId'>()(value.approval, at('approval')),
        ask: approvalAsk(value.ask, at('ask')),
      };
    case 'approval_resolved':
      return {
        kind: 'approval_resolved',
        approval: branded<'ApprovalId'>()(value.approval, at('approval')),
      };
    case 'turn_ended':
      return {
        kind: 'turn_ended',
        turn: branded<'NativeTurnId'>()(value.turn, at('turn')),
        end: turnEnd(value.end, at('end')),
      };
    case 'error':
      return {
        kind: 'error',
        message: string(value.message, at('message')),
        retry: value.retry === 'will_retry' ? 'will_retry' : 'final',
      };
    case 'refused':
      return {
        kind: 'refused',
        request: String(value.request),
        message: string(value.message, at('message')),
      };
    case 'unsupported':
      return { kind: 'unsupported', method: string(value.method, at('method')) };
    default:
      return { kind: 'other', name: String(value.kind) };
  }
};

/** One `run` event of the update feed. */
export const runUpdate: Decoder<RunUpdate> = (value, path) => {
  if (!isRecord(value)) throw new ShapeError(path, 'an object');
  const run = RunId.decode(value.run, `${path}.run`);
  switch (value.kind) {
    case 'state':
      return { kind: 'state', run, state: runState(value.state, `${path}.state`) };
    case 'event':
      return { kind: 'event', run, event: driveEvent(value.event, `${path}.event`) };
    default:
      throw new ShapeError(`${path}.kind`, 'state or event');
  }
};

const recordedRun = object<RecordedRun>({
  id: RunId.decode,
  harness: label(harnesses),
  workdir: string,
  started_at: brandedCount<'UnixMillis'>(),
  thread: nullable(branded<'NativeThreadId'>()),
  session: nullable(SessionId.decode),
  ended_at: nullable(brandedCount<'UnixMillis'>()),
  end: nullable(label(runEnds)),
  exit: nullable(processExit),
  archived_at: nullable(brandedCount<'UnixMillis'>()),
});

const liveRuns = versioned(object<{ runs: RunSnapshot[] }>({ runs: array(runSnapshot) }));
/** A project's repository, as its `HEAD` reads. */
export type ProjectGit =
  | { kind: 'branch'; name: string }
  | { kind: 'detached'; commit: string }
  | { kind: 'unreadable' }
  | { kind: 'none' };

/** A place a run can start in: a git repository under a run root, or a root's folder. */
export interface RunProject {
  path: string;
  name: string;
  git: ProjectGit;
}

const projectGit: Decoder<ProjectGit> = (value, path) => {
  if (!isRecord(value)) throw new ShapeError(path, 'an object');
  switch (value.kind) {
    case 'branch':
      return { kind: 'branch', name: string(value.name, `${path}.name`) };
    case 'detached':
      return { kind: 'detached', commit: string(value.commit, `${path}.commit`) };
    case 'none':
      return { kind: 'none' };
    default:
      return { kind: 'unreadable' };
  }
};
const runProjects = versioned(
  object<{ projects: RunProject[] }>({
    projects: array(object<RunProject>({ path: string, name: string, git: projectGit })),
  }),
);
const recordedRuns = versioned(object<{ runs: RecordedRun[] }>({ runs: array(recordedRun) }));

const clientStatuses = ['ready', 'not_installed', 'not_drivable'] as const;
/** Whether a run can start with a client on the daemon's host. */
export type ClientStatus = (typeof clientStatuses)[number];

/** A client, and whether a run can start with it on the daemon's host. */
export interface RunClient {
  harness: Stored<Harness>;
  status: Stored<ClientStatus>;
}

const runClients = versioned(
  object<{ clients: RunClient[] }>({
    clients: array(object<RunClient>({ harness: label(harnesses), status: label(clientStatuses) })),
  }),
);
const started = versioned(object<{ run: RunId }>({ run: RunId.decode }));
const versionedRecordedRun = versioned(recordedRun);

/** Why the daemon or this UI server refused a run command, by its error code. */
export class RunCommandError extends Error {
  readonly status: number;
  readonly code: string | null;
  constructor(status: number, code: string | null) {
    super(RunCommandError.describe(status, code));
    this.status = status;
    this.code = code;
  }

  static describe(status: number, code: string | null): string {
    switch (code) {
      case 'runs_off':
        return 'This UI server does not drive runs: set REN_CONTROL_TOKEN_FILE.';
      case 'runs_disabled':
        return 'Runs are off on the daemon: add a folder to [runs] roots.';
      case 'invalid_directory':
        return 'That folder is not under one of the daemon’s run roots.';
      case 'directory_not_found':
        return 'That folder does not exist on the daemon’s host.';
      case 'not_drivable':
        return 'Ren cannot drive that client yet.';
      case 'spawn_failed':
        return 'The client could not be started; is it installed on the daemon’s host?';
      case 'unrecorded':
        return 'The recorder has no room to record a new run right now.';
      case 'busy':
        return 'A turn is already running.';
      case 'starting':
        return 'The run is still opening its conversation.';
      case 'not_working':
      case 'turn_not_started':
        return 'No turn is running.';
      case 'no_such_approval':
        return 'That approval no longer waits.';
      case 'stopping':
      case 'ended':
        return 'The run is stopping or has ended.';
      case 'not_found':
        return 'The daemon does not know that run.';
      case 'call_open':
        return 'A call of it is still open; delete it once the call ends.';
      case 'run_running':
        return 'A run of it is still running; stop it before deleting.';
      case 'cross_origin':
      case 'login_not_allowed':
        return 'This UI does not let you drive runs.';
      case 'no_control_token':
        return 'The UI server cannot read the daemon’s control token. Is the daemon running?';
      case 'daemon_unreachable':
        return 'Cannot reach the Ren daemon.';
      case 'unauthorized':
      case 'forbidden':
      case 'cross_site':
        return 'The daemon refused the UI server’s control token.';
      default:
        return `The command was refused (${status}${code ? ` ${code}` : ''}).`;
    }
  }
}

/** The clients the daemon can drive, as the new-run form offers them. */
export const DRIVABLE = ['codex', 'claude_code', 'pi'] as const satisfies readonly Harness[];
export type Drivable = (typeof DRIVABLE)[number];

/** Run commands and reads through a capability-supplied `fetch`. */
export class RunsClient {
  readonly #fetch: typeof fetch;
  constructor(fetcher: typeof fetch = fetch) {
    this.#fetch = fetcher;
  }

  async live(signal?: AbortSignal): Promise<RunSnapshot[]> {
    return (await this.#send('GET', '/control/runs', undefined, liveRuns, signal)).runs;
  }
  async projects(signal?: AbortSignal): Promise<RunProject[]> {
    return (await this.#send('GET', '/control/runs/projects', undefined, runProjects, signal))
      .projects;
  }
  /** Every client, and whether a run can start with it on the daemon's host. */
  async clients(signal?: AbortSignal): Promise<RunClient[]> {
    return (await this.#send('GET', '/control/runs/clients', undefined, runClients, signal))
      .clients;
  }
  async start(
    harness: Drivable,
    dir: string,
    resume: NativeThreadId | null = null,
    prompt: string | null = null,
  ): Promise<RunId> {
    const body = { harness, dir, ...(resume ? { resume } : {}), ...(prompt ? { prompt } : {}) };
    return (await this.#send('POST', '/control/runs', body, started)).run;
  }
  async prompt(run: RunId, text: string): Promise<void> {
    await this.#send('POST', `/control/runs/${run}/prompt`, { text }, null);
  }
  async answer(run: RunId, approval: ApprovalId, decision: Decision): Promise<void> {
    const path = `/control/runs/${run}/approvals/${encodeURIComponent(approval)}`;
    await this.#send('POST', path, { decision }, null);
  }
  async interrupt(run: RunId): Promise<void> {
    await this.#send('POST', `/control/runs/${run}/interrupt`, {}, null);
  }
  async stop(run: RunId): Promise<void> {
    await this.#send('POST', `/control/runs/${run}/stop`, {}, null);
  }
  /**
   * Archive, unarchive, or delete what was recorded of a session or a run.
   * The daemon applies it after what it is recording; the live feed says
   * when.
   */
  async edit(subject: HistorySubject, change: HistoryChange): Promise<void> {
    const kind = subject.kind === 'session' ? 'sessions' : 'runs';
    await this.#send('POST', `/control/history/${kind}/${subject.id}/${change}`, {}, null);
  }
  /** Recorded runs on `shelf`, newest first, from the read-only management API. */
  async recorded(
    limit = 100,
    shelf: Shelf = 'current',
    signal?: AbortSignal,
  ): Promise<RecordedRun[]> {
    const query = shelf === 'archived' ? `&shelf=archived` : '';
    return (
      await this.#send('GET', `/api/runs?limit=${limit}${query}`, undefined, recordedRuns, signal)
    ).runs;
  }
  async recordedRun(run: RunId, signal?: AbortSignal): Promise<RecordedRun> {
    return this.#send('GET', `/api/runs/${run}`, undefined, versionedRecordedRun, signal);
  }

  async #send<T>(
    method: 'GET' | 'POST',
    path: string,
    body: unknown,
    decode: Decoder<T> | null,
    signal?: AbortSignal,
  ): Promise<T> {
    const timeout = AbortSignal.timeout(15_000);
    const response = await this.#fetch(path, {
      method,
      cache: 'no-store',
      headers:
        body === undefined
          ? { accept: 'application/json' }
          : { accept: 'application/json', 'content-type': 'application/json' },
      body: body === undefined ? undefined : JSON.stringify(body),
      signal: signal ? AbortSignal.any([signal, timeout]) : timeout,
    });
    let parsed: unknown = null;
    try {
      parsed = await response.json();
    } catch {
      parsed = null;
    }
    if (!response.ok) {
      const code = isRecord(parsed) && typeof parsed.error === 'string' ? parsed.error : null;
      throw new RunCommandError(response.status, code);
    }
    if (decode === null) {
      if (!isRecord(parsed) || parsed.api_version !== API_VERSION) {
        throw new ShapeError('api_version', `${API_VERSION}`);
      }
      return undefined as T;
    }
    return decode(parsed, '');
  }
}

/** One entry of a run's live conversation, built from its events. */
export type Entry =
  | { kind: 'prompt'; text: string }
  | { kind: 'message'; item: ItemId; text: string; done: boolean }
  | { kind: 'reasoning'; item: ItemId; done: boolean }
  | {
      kind: 'command';
      item: ItemId;
      command: string;
      status: string;
      output: string | null;
      exit: number | null;
    }
  | { kind: 'file_change'; item: ItemId; paths: string[]; status: string }
  | { kind: 'tool'; item: ItemId; name: string; done: boolean }
  | { kind: 'turn_end'; end: TurnEnd }
  | { kind: 'notice'; tone: 'warn' | 'danger'; text: string };

/** The newest entries a conversation keeps; older ones are in the recorded history. */
const KEPT_ENTRIES = 400;

/**
 * A run's conversation as it happens, since this page started following it.
 * The recorded turns are the durable record; this is what streams in between.
 */
export class Transcript {
  readonly entries: Entry[];
  /** The turn the newest entries belong to, once the agent named it. */
  readonly turn: NativeTurnId | null;
  /**
   * Recorded turns already read when this turn started, by native id; null
   * when the page has not read them.
   */
  readonly known: readonly string[] | null;

  constructor(
    entries: Entry[] = [],
    turn: NativeTurnId | null = null,
    known: readonly string[] | null = null,
  ) {
    this.entries = entries;
    this.turn = turn;
    this.known = known;
  }

  /** The recorded turns read when the current turn started. */
  knowing(known: readonly string[]): Transcript {
    return new Transcript(this.entries, this.turn, known);
  }

  /**
   * Whether the turn that just ended is now among `turns`, the recorded
   * ones: by its id where the client names its turns the same live and on
   * the wire (Codex), or else (Claude Code) as a recorded turn that was not
   * there when it started and carries its prompt, or any such turn when this
   * page joined after the prompt was sent.
   */
  recordedIn(turns: readonly { native_id: string; prompt: Excerpt | null }[]): boolean {
    const turn = this.turn;
    if (turn === null || !this.ended(turn)) return false;
    if (turns.some((recorded) => recorded.native_id === turn)) return true;
    const known = this.known;
    if (known === null) return false;
    const prompt = this.#endedPrompt();
    return turns.some(
      (recorded) =>
        !known.includes(recorded.native_id) &&
        (prompt === null ||
          (recorded.prompt !== null &&
            (recorded.prompt.omitted_chars > 0
              ? prompt.startsWith(recorded.prompt.text)
              : prompt === recorded.prompt.text))),
    );
  }

  /** The prompt of the turn the transcript ends with. */
  #endedPrompt(): string | null {
    const last = this.entries.length - 1;
    const previous = this.entries.findLastIndex(
      (entry, index) => index < last && entry.kind === 'turn_end',
    );
    const prompt = this.entries
      .slice(previous + 1, last)
      .findLast((entry) => entry.kind === 'prompt');
    return prompt?.kind === 'prompt' ? prompt.text : null;
  }

  /**
   * The prompt the operator just sent. The agent's echo of it may arrive
   * first, over the update feed, so the current turn shows it once.
   */
  prompted(text: string): Transcript {
    if (this.#prompts().includes(text)) return this;
    return this.#with([...this.entries, { kind: 'prompt', text }], this.turn);
  }

  /** The prompts shown since the last turn ended. */
  #prompts(): string[] {
    const start = this.entries.findLastIndex((entry) => entry.kind === 'turn_end') + 1;
    return this.entries
      .slice(start)
      .flatMap((entry) => (entry.kind === 'prompt' ? [entry.text] : []));
  }

  /** Whether `turn` ended in this transcript. */
  ended(turn: NativeTurnId): boolean {
    return this.turn === turn && this.entries.at(-1)?.kind === 'turn_end';
  }

  apply(event: DriveEvent): Transcript {
    switch (event.kind) {
      case 'turn_started':
        return new Transcript(this.entries, event.turn, this.known);
      case 'delta': {
        const index = this.#find(event.item);
        const entry = index === -1 ? undefined : this.entries[index];
        if (entry?.kind === 'message') {
          return this.#replace(index, { ...entry, text: entry.text + event.text });
        }
        return this.#with(
          [...this.entries, { kind: 'message', item: event.item, text: event.text, done: false }],
          this.turn,
        );
      }
      case 'item':
        return this.#item(event.item, event.phase === 'completed');
      case 'turn_ended':
        // A page opened mid-turn learns the turn here.
        return this.#with([...this.entries, { kind: 'turn_end', end: event.end }], event.turn);
      case 'error':
        return this.#with(
          [
            ...this.entries,
            {
              kind: 'notice',
              tone: event.retry === 'will_retry' ? 'warn' : 'danger',
              text: event.retry === 'will_retry' ? `${event.message} (retrying)` : event.message,
            },
          ],
          this.turn,
        );
      case 'refused':
        return this.#with(
          [...this.entries, { kind: 'notice', tone: 'danger', text: event.message }],
          this.turn,
        );
      case 'ready':
      case 'approval':
      case 'approval_resolved':
      case 'unsupported':
      case 'other':
        // Approvals live in the run's state; the rest says nothing to show.
        return this;
      default:
        return assertNever(event);
    }
  }

  #item(item: DriveItem, done: boolean): Transcript {
    const read = item.read;
    let entry: Entry | null;
    switch (read.as) {
      case 'prompt':
        // The operator's prompt, which `prompted` usually showed already.
        entry =
          read.text === '' || this.#prompts().includes(read.text)
            ? null
            : { kind: 'prompt', text: read.text };
        break;
      case 'reply': {
        const index = this.#find(item.id);
        const before = index === -1 ? undefined : this.entries[index];
        const streamed = before?.kind === 'message' ? before.text : '';
        entry = { kind: 'message', item: item.id, text: read.text || streamed, done };
        break;
      }
      case 'reasoning':
        entry = { kind: 'reasoning', item: item.id, done };
        break;
      case 'command':
        entry = {
          kind: 'command',
          item: item.id,
          command: read.command,
          status: read.status,
          output: read.output,
          exit: read.exit_code,
        };
        break;
      case 'file_change':
        entry = { kind: 'file_change', item: item.id, paths: read.paths, status: read.status };
        break;
      case 'plan':
        entry = read.text ? { kind: 'message', item: item.id, text: read.text, done } : null;
        break;
      case 'tool':
        entry = { kind: 'tool', item: item.id, name: read.name, done };
        break;
      case 'unread':
        entry = { kind: 'tool', item: item.id, name: item.kind, done };
        break;
      default:
        return assertNever(read);
    }
    if (entry === null) return this;
    const index = this.#find(item.id);
    return index === -1
      ? this.#with([...this.entries, entry], this.turn)
      : this.#replace(index, entry);
  }

  #find(item: ItemId): number {
    return this.entries.findLastIndex((entry) => 'item' in entry && entry.item === item);
  }

  #replace(index: number, entry: Entry): Transcript {
    const entries = this.entries.slice();
    entries[index] = entry;
    return new Transcript(entries, this.turn, this.known);
  }

  #with(entries: Entry[], turn: NativeTurnId | null): Transcript {
    return new Transcript(entries.slice(-KEPT_ENTRIES), turn, this.known);
  }
}

/** What the operator can ask of a run in its current state. */
export interface RunControls {
  prompt: boolean;
  interrupt: boolean;
  stop: boolean;
  /** The approvals waiting for an answer, oldest first. */
  approvals: Waiting[];
}

/** How a run's state reads in the sidebar and header. */
export interface PhaseMark {
  label: string;
  tone: 'live' | 'warn' | 'neutral' | 'danger' | 'ok';
}

export class Runs {
  static controls(state: RunState): RunControls {
    switch (state.phase) {
      case 'idle':
        return { prompt: true, interrupt: false, stop: true, approvals: [] };
      case 'working':
        return {
          prompt: false,
          interrupt: state.turn !== null,
          stop: true,
          approvals: state.approvals,
        };
      case 'starting':
        return { prompt: false, interrupt: false, stop: true, approvals: [] };
      case 'stopping':
      case 'ended':
      case 'unrecognized':
        return { prompt: false, interrupt: false, stop: false, approvals: [] };
      default:
        return assertNever(state);
    }
  }

  static mark(state: RunState): PhaseMark {
    switch (state.phase) {
      case 'starting':
        return { label: 'starting', tone: 'neutral' };
      case 'idle':
        return { label: 'your turn', tone: 'ok' };
      case 'working':
        return state.approvals.length
          ? { label: 'needs you', tone: 'warn' }
          : { label: 'working', tone: 'live' };
      case 'stopping':
        return { label: 'stopping', tone: 'neutral' };
      case 'ended':
        return { label: Runs.endLabel(state.end), tone: Runs.endTone(state.end) };
      case 'unrecognized':
        return { label: state.text, tone: 'neutral' };
      default:
        return assertNever(state);
    }
  }

  static endLabel(end: Stored<RunEnd>): string {
    switch (end) {
      case 'spawn_failed':
        return 'did not start';
      case 'open_refused':
        return 'refused to open';
      case 'exited':
        return 'exited';
      case 'stopped':
        return 'stopped';
      case 'drained':
        return 'stopped at shutdown';
      case 'recovered':
        return 'lost at restart';
      default:
        return unrecognised(end);
    }
  }

  static endTone(end: Stored<RunEnd>): PhaseMark['tone'] {
    switch (end) {
      case 'stopped':
      case 'drained':
        return 'neutral';
      case 'exited':
      case 'recovered':
        return 'warn';
      case 'spawn_failed':
      case 'open_refused':
        return 'danger';
      default:
        unrecognised(end);
        return 'neutral';
    }
  }

  /** One line saying what an approval asks. */
  static ask(ask: ApprovalAsk): { action: string; detail: string | null; reason: string | null } {
    switch (ask.kind) {
      case 'command':
        return {
          action: 'Run a command',
          detail: ask.command,
          reason: ask.reason,
        };
      case 'file_change':
        return { action: 'Change files', detail: null, reason: ask.reason };
      case 'tool':
        return { action: `Use ${ask.tool}`, detail: ask.detail, reason: null };
      case 'unrecognized':
        return { action: `Allow ${ask.text}`, detail: null, reason: null };
      default:
        return assertNever(ask);
    }
  }

  /** Why a run cannot start with a client, or null when it can. */
  static unavailable(status: Stored<ClientStatus>): string | null {
    switch (status) {
      case 'ready':
        return null;
      case 'not_installed':
        return 'not installed on the daemon’s host';
      case 'not_drivable':
        return 'Ren cannot drive it yet';
      default:
        return unrecognised(status);
    }
  }

  /**
   * Projects grouped by the folder they are in, groups in path order, and
   * each group's projects by name.
   */
  static places(projects: readonly RunProject[]): { parent: string; projects: RunProject[] }[] {
    const groups = new Map<string, RunProject[]>();
    for (const project of projects) {
      const parent = project.path.slice(0, project.path.length - project.name.length) || '/';
      groups.set(parent, [...(groups.get(parent) ?? []), project]);
    }
    return [...groups]
      .sort(([a], [b]) => a.localeCompare(b))
      .map(([parent, projects]) => ({
        parent,
        projects: projects.toSorted((a, b) => a.name.localeCompare(b.name)),
      }));
  }

  /** The projects matching what is typed, by name or path, ignoring case. */
  static choose(projects: readonly RunProject[], typed: string): RunProject[] {
    const text = typed.trim().toLowerCase();
    return projects.filter(
      (project) =>
        project.name.toLowerCase().includes(text) || project.path.toLowerCase().includes(text),
    );
  }

  /** What a project's repository is on, in a few words. */
  static branch(git: ProjectGit): string {
    switch (git.kind) {
      case 'branch':
        return git.name;
      case 'detached':
        return `at ${git.commit}`;
      case 'unreadable':
        return 'git';
      case 'none':
        return 'no git';
      default:
        return assertNever(git);
    }
  }

  /** A run's client, when the daemon can drive it to resume the conversation. */
  static drivable(harness: Stored<Harness>): Drivable | null {
    return DRIVABLE.find((drivable) => drivable === harness) ?? null;
  }

  /** The folder's last component, as the sidebar names a project. */
  static folder(workdir: string): string {
    const parts = workdir.split('/').filter((part) => part !== '');
    return parts.at(-1) ?? workdir;
  }

  /** The thread a run's conversation has, while the run has one open. */
  static thread(state: RunState): NativeThreadId | null {
    switch (state.phase) {
      case 'idle':
      case 'working':
        return state.thread;
      case 'starting':
      case 'stopping':
      case 'ended':
      case 'unrecognized':
        return null;
      default:
        return assertNever(state);
    }
  }

  /** Apply an update to the live list: a new run appears, a known one changes. */
  static update(runs: RunSnapshot[], update: RunUpdate): RunSnapshot[] {
    if (update.kind !== 'state') return runs;
    const index = runs.findIndex((run) => run.run === update.run);
    if (index === -1) {
      // A run started elsewhere; its folder arrives with the next listing.
      return runs;
    }
    const next = runs.slice();
    next[index] = { ...runs[index]!, state: update.state };
    return next;
  }
}

/** A run in the sidebar: live, or recorded and ended. */
export interface DeskRun {
  run: RunId;
  harness: Stored<Harness>;
  workdir: string;
  started_at: UnixMillis | null;
  /** The live state, while the daemon holds the run. */
  state: RunState | null;
  recorded: RecordedRun | null;
}

/** Runs grouped by folder, the folder with the newest run first. */
export interface Project {
  workdir: string;
  name: string;
  runs: DeskRun[];
  /** Approvals waiting across the folder's runs. */
  waiting: number;
}

export class Desk {
  /**
   * Join live and recorded runs and group them by folder. Ended runs in
   * `hidden` (archived) are left out, though the daemon still lists them.
   */
  static projects(
    live: RunSnapshot[],
    recorded: RecordedRun[],
    hidden: ReadonlySet<RunId> = new Set(),
  ): Project[] {
    const byRun = new Map<RunId, DeskRun>();
    for (const run of recorded) {
      byRun.set(run.id, {
        run: run.id,
        harness: run.harness,
        workdir: run.workdir,
        started_at: run.started_at,
        state: null,
        recorded: run,
      });
    }
    for (const run of live) {
      if (run.state.phase === 'ended' && hidden.has(run.run)) continue;
      const known = byRun.get(run.run);
      byRun.set(run.run, {
        run: run.run,
        harness: run.harness,
        workdir: run.workdir,
        started_at: known?.started_at ?? null,
        state: run.state,
        recorded: known?.recorded ?? null,
      });
    }
    const groups = new Map<string, DeskRun[]>();
    for (const run of byRun.values()) {
      const runs = groups.get(run.workdir) ?? [];
      runs.push(run);
      groups.set(run.workdir, runs);
    }
    // Live runs first, then newest first; a run not yet recorded is newest.
    const newest = (run: DeskRun) => run.started_at ?? Number.MAX_SAFE_INTEGER;
    const live_ = (run: DeskRun) => (run.state !== null && run.state.phase !== 'ended' ? 1 : 0);
    const projects = [...groups].map(([workdir, runs]) => {
      runs.sort((a, b) => live_(b) - live_(a) || newest(b) - newest(a));
      return {
        workdir,
        name: Runs.folder(workdir),
        runs,
        waiting: runs.reduce(
          (sum, run) => sum + (run.state ? Runs.controls(run.state).approvals.length : 0),
          0,
        ),
      };
    });
    projects.sort(
      (a, b) =>
        b.waiting - a.waiting ||
        Math.max(...b.runs.map(live_)) - Math.max(...a.runs.map(live_)) ||
        newest(b.runs[0]!) - newest(a.runs[0]!),
    );
    return projects;
  }

  /** The projects runs worked in, most recently started first, at most `count`. */
  static recent(runs: readonly DeskRun[], offered: readonly RunProject[], count = 3): RunProject[] {
    const newest = runs
      .filter((run) => run.started_at !== null)
      .toSorted((a, b) => (b.started_at ?? 0) - (a.started_at ?? 0));
    const recent: RunProject[] = [];
    for (const run of newest) {
      const project = offered.find((project) => project.path === run.workdir);
      if (project && !recent.includes(project)) recent.push(project);
      if (recent.length === count) break;
    }
    return recent;
  }

  /** A folder's runs that have not ended, then those that have, newest first. */
  static split(project: Project): { open: DeskRun[]; ended: DeskRun[] } {
    const ended = (run: DeskRun) => Desk.state(run).phase === 'ended';
    return {
      open: project.runs.filter((run) => !ended(run)),
      ended: project.runs.filter(ended),
    };
  }

  /** The state a sidebar run shows: live if held, else its recorded end. */
  static state(run: DeskRun): RunState {
    if (run.state) return run.state;
    const recorded = run.recorded;
    if (recorded?.end) return { phase: 'ended', end: recorded.end, exit: recorded.exit };
    // Recorded as started but not held by the daemon: it ended without a record yet.
    return { phase: 'unrecognized', text: 'not running' };
  }
}
