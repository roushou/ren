// What a session desk follows of the daemon: the live runs, the sessions on
// both shelves, the places and clients a session starts with, and each
// run's conversation; read once, then kept current by the runs and sessions
// feeds. `connect` opens the feeds; its cleanup closes them and abandons
// every read and retry still on its way, so nothing lands after the desk
// has gone.
import { untrack } from 'svelte';
import { SvelteMap } from 'svelte/reactivity';
import type { Shelf } from '#lib/api.ts';
import type { Arriving } from '#lib/arrival.ts';
import type { Snapshot } from '#lib/snapshots.ts';
import {
  RunCommandError,
  Runs,
  RunsClient,
  Sessions,
  Transcript,
  runUpdate,
  type DeskRun,
  type Drivable,
  type Read,
  type RunClient,
  type RunId,
  type RunPlace,
  type RunSnapshot,
  type RunsFeed,
  type RunsStart,
  type SessionBody,
  type SessionId,
  type SessionScope,
  type StoredConversation,
} from '#lib/runs.ts';
import type { Listing } from './SessionList.svelte';
import type { DeskData } from './load.ts';

/** The selected run's stored conversation: on its way, read, or why it
 * could not be. */
type ConversationRead =
  | { run: RunId; state: 'pending' }
  | { run: RunId; state: 'read' }
  | { run: RunId; state: 'failed'; reason: string };

/** How a conversation is read: shown, standing as pending while it is; or
 * again, quietly, keeping what is shown. */
export type ConversationReading = 'shown' | 'again';

export interface DeskFeedOptions {
  client: RunsClient;
  /** The desk's data, as its page has it now. */
  data: () => DeskData;
  /** Which sessions the desk holds. */
  scope: () => SessionScope;
  /** The run the desk shows, if any. */
  selected: () => RunId | null;
}

export class DeskFeed {
  readonly #client: RunsClient;
  readonly #data: () => DeskData;
  readonly #scope: () => SessionScope;
  readonly #selected: () => RunId | null;
  /** What the server read for the first render; the feed follows from
   * here. Null when it read nothing: runs are off, or the page came by a
   * navigation in the browser and reads it all itself. */
  readonly #initial: RunsStart | null;
  /** Abandons every read and retry on its way once the desk has gone. */
  readonly #abort = new AbortController();
  /** The processes whose conversation was read again once they opened. */
  readonly #reread = new Set<RunId>();

  live = $state.raw<RunSnapshot[]>([]);
  /** Every session, current and archived, as stored and served. */
  #every = $state.raw<SessionBody[]>([]);
  #everyArchived = $state.raw<SessionBody[]>([]);
  /** Those this desk holds. */
  readonly sessions = $derived.by(() => Sessions.within(this.#every, this.#scope()));
  readonly archived = $derived.by(() => Sessions.within(this.#everyArchived, this.#scope()));
  /** Whether the lists have been read: on a navigation in the browser they
   * are on their way, and the list draws its outline until they come. */
  listing = $state<Listing>('read');
  /** Why runs cannot be listed or driven, when they cannot. */
  unavailable = $state<string | null>(null);
  /** The runs feed, of which only a lost or given-up one is worth a word. */
  feed = $state<RunsFeed>('opening');
  /** Each run's conversation since this page started following it. */
  readonly transcripts = new SvelteMap<RunId, Transcript>();
  /** The projects the daemon offers to start a run in. */
  offered = $state.raw<RunPlace[]>([]);
  /** The folder scratch sessions get theirs in; null when there is none. */
  scratch = $state<string | null>(null);
  /** The clients a run can start with, and whether each is ready. */
  clients = $state.raw<RunClient[]>([]);
  /** A client's sign-in is being read again. */
  checking = $state(false);
  #conversationRead = $state.raw<ConversationRead | null>(null);

  /** The selected run's conversation, as far as it is known here. */
  readonly transcript = $derived.by(() => {
    const selected = this.#selected();
    return selected ? (this.transcripts.get(selected) ?? new Transcript()) : null;
  });
  /** Why the selected run's conversation could not be read. */
  readonly readError = $derived.by(() => {
    const read = this.#conversationRead;
    return read?.state === 'failed' && read.run === this.#selected() ? read.reason : null;
  });
  /** The selected run's conversation is on its way, with nothing of it read
   * here yet: the pane draws its outline. */
  readonly conversationPending = $derived.by(
    () =>
      this.#conversationRead?.state === 'pending' &&
      this.#conversationRead.run === this.#selected() &&
      !this.transcript?.read,
  );

  constructor(options: DeskFeedOptions) {
    this.#client = options.client;
    this.#data = options.data;
    this.#scope = options.scope;
    this.#selected = options.selected;
    const data = untrack(options.data);
    const initial = data.opening.kind === 'read' ? data.opening.start : null;
    this.#initial = initial;
    this.live = DeskFeed.#first(initial?.live, []);
    this.#every = DeskFeed.#first(initial?.sessions, []);
    this.#everyArchived = DeskFeed.#first(initial?.archivedSessions, []);
    this.listing = data.opening.kind === 'unread' ? 'pending' : 'read';
    this.unavailable = initial?.live.state === 'failed' ? initial.live.reason : null;
    const places = DeskFeed.#first(initial?.places, null);
    this.offered = places?.places ?? [];
    this.scratch = places?.scratch ?? null;
    this.clients = DeskFeed.#first(initial?.clients, []);
    // The selected run's conversation, as the page was loaded with it.
    const given = data.conversation;
    if (given?.conversation.state === 'ready' && given.conversation.snapshot.value) {
      this.transcripts.set(given.run, new Transcript().load(given.conversation.snapshot.value));
    }
  }

  static #first<T>(read: Read<T> | undefined, none: T): T {
    return read?.state === 'read' ? read.value : none;
  }

  /** Aborted once the desk has gone: reads made for it stop with it. */
  get signal(): AbortSignal {
    return this.#abort.signal;
  }

  /** What the header says of the runs feed. */
  get note(): string {
    return Runs.feedNote(this.feed);
  }

  async readLive() {
    if (!this.#data().runsControl) return;
    try {
      this.live = await this.#client.live(this.signal);
      this.unavailable = null;
    } catch (error) {
      if (!this.signal.aborted) this.unavailable = RunCommandError.say(error);
    }
  }

  async readProjects() {
    if (!this.#data().runsControl) return;
    try {
      ({ places: this.offered, scratch: this.scratch } = await this.#client.places(this.signal));
    } catch {
      // The list is a convenience; a typed path still starts a run.
    }
  }

  async readClients() {
    if (!this.#data().runsControl) return;
    try {
      this.clients = await this.#client.clients(this.signal);
    } catch {
      // Without the list, both drivable clients are offered.
    }
  }

  async readSessions() {
    if (!this.#data().runsControl) return;
    try {
      [this.#every, this.#everyArchived] = await Promise.all([
        this.#client.sessions('current', this.signal),
        this.#client.sessions('archived', this.signal),
      ]);
      this.listing = 'read';
    } catch {
      // The list stays as it was; the feed brings changes.
      if (this.listing === 'pending' && !this.signal.aborted) this.listing = 'failed';
    }
  }

  /** Whether `session` is on `shelf` of the lists as last read. */
  holds(session: SessionId, shelf: Shelf): boolean {
    const bodies = shelf === 'current' ? this.sessions : this.archived;
    return bodies.some((entry) => entry.session === session);
  }

  /** Whether `session` is on either shelf of the lists as last read. */
  lists(session: SessionId): boolean {
    return this.holds(session, 'current') || this.holds(session, 'archived');
  }

  /**
   * Read the lists again until `moved` holds: the daemon applies a history
   * change after what it is recording. Whether it held by the last read;
   * false at once once the desk has gone.
   */
  async until(moved: () => boolean): Promise<boolean> {
    for (const delay of DeskFeed.#RETRIES_MS) {
      await this.#pause(delay);
      if (this.signal.aborted) return false;
      await this.readSessions();
      if (moved()) return true;
    }
    return false;
  }

  static readonly #RETRIES_MS = [150, 400, 1000, 2000] as const;

  #pause(ms: number): Promise<void> {
    return new Promise((resolve) => {
      const timer = setTimeout(resolve, ms);
      this.signal.addEventListener(
        'abort',
        () => {
          clearTimeout(timer);
          resolve();
        },
        { once: true },
      );
    });
  }

  /** The client of `harness`, as last read; null when it is not listed. */
  clientOf(harness: string | undefined): RunClient | null {
    return this.clients.find((entry) => entry.harness === harness) ?? null;
  }

  /** Where `workdir` is, in words: a project's name, Scratch, or its path. */
  place(workdir: string): string {
    return Runs.place(workdir, this.scratch, this.offered);
  }

  /** What a session was asked first: as stored, else from its conversation here. */
  title(entry: { run: RunId | null; asked?: string | null }): string | null {
    return Sessions.title(
      entry.asked ?? null,
      entry.run ? (this.transcripts.get(entry.run)?.firstPrompt() ?? null) : null,
    );
  }

  /** Its first prompt, whole, for the title's tooltip. */
  firstPrompt(run: DeskRun): string | null {
    return this.transcripts.get(run.run)?.firstPrompt()?.trim() || null;
  }

  /** Read the client's sign-in again: it may have been signed in another way. */
  async checkSignIn(harness: Drivable) {
    this.checking = true;
    try {
      await this.#client.checkSignIn(harness);
    } catch {
      // The line stays as it is; the next read of the clients says.
    } finally {
      this.checking = false;
    }
    await this.readClients();
  }

  /**
   * Show `run`'s conversation: the one the page was loaded with when it was
   * opened on this run, read otherwise. Untracked: the caller follows the
   * selection, not the transcripts this writes.
   */
  show(run: RunId | null) {
    this.#conversationRead = null;
    if (!run) return;
    const given = this.#data().conversation;
    if (given?.run === run) {
      this.#adopt(run, given.conversation);
    } else {
      void this.readConversation(run);
    }
  }

  /**
   * Read the run's conversation, whole, as the daemon stored it, keeping
   * what streamed in meanwhile. Shown, the conversation stands as pending
   * while it is read; read again quietly, what is shown stays until the
   * read lands, and a failure changes nothing.
   */
  async readConversation(run: RunId, how: ConversationReading = 'shown') {
    const shown = () => this.#selected() === run;
    if (how === 'shown' && shown()) this.#conversationRead = { run, state: 'pending' };
    try {
      const conversation = await this.#client.conversation(run, this.signal);
      this.transcripts.set(run, (this.transcripts.get(run) ?? new Transcript()).load(conversation));
      if (shown()) this.#conversationRead = { run, state: 'read' };
    } catch (error) {
      if (how === 'shown' && shown() && !this.signal.aborted)
        this.#conversationRead = { run, state: 'failed', reason: RunCommandError.say(error) };
    }
  }

  /**
   * Take the conversation the page was loaded with: at once when it was
   * read, or once it arrives. A read that failed is made again here, which
   * says why when it cannot.
   */
  #adopt(run: RunId, given: Arriving<StoredConversation>) {
    if (given.state === 'ready') {
      this.#took(run, given.snapshot);
      return;
    }
    this.#conversationRead = { run, state: 'pending' };
    void given.snapshot.then((snapshot) => {
      if (!this.signal.aborted) this.#took(run, snapshot);
    });
  }

  #took(run: RunId, snapshot: Snapshot<StoredConversation>) {
    if (snapshot.value === null) {
      if (this.#selected() === run) void this.readConversation(run);
      return;
    }
    this.transcripts.set(run, (this.transcripts.get(run) ?? new Transcript()).load(snapshot.value));
    if (this.#selected() === run) this.#conversationRead = { run, state: 'read' };
  }

  /**
   * Read what the server could not for the first render, and follow the
   * runs and sessions feeds. The cleanup closes them and abandons whatever
   * is still on its way.
   */
  connect(): () => void {
    // The server read them for the first render; only what it could not
    // read is asked for again. Live runs are read once the feed opens.
    const initial = this.#initial;
    if (initial?.places.state !== 'read') void this.readProjects();
    if (initial?.clients.state !== 'read') void this.readClients();
    if (initial?.sessions.state !== 'read' || initial.archivedSessions.state !== 'read')
      void this.readSessions();
    if (!initial) void this.readLive();
    const abandon = () => this.#abort.abort();
    if (!untrack(this.#data).runsControl) return abandon;
    const source = new EventSource('/control/runs/events');
    source.addEventListener('open', () => {
      this.feed = 'open';
      // Whatever changed while the feed was closed.
      void this.readLive();
    });
    source.addEventListener('error', () => {
      this.feed = source.readyState === EventSource.CLOSED ? 'closed' : 'reconnecting';
    });
    source.addEventListener('lagged', () => void this.readLive());
    // Sessions change as they are made, configured, archived, or get a new
    // process: each change reads the list again.
    const sessionFeed = new EventSource('/control/sessions/events');
    sessionFeed.addEventListener('session', () => void this.readSessions());
    sessionFeed.addEventListener('lagged', () => void this.readSessions());
    source.addEventListener('run', (event) => this.#run(event as MessageEvent<string>));
    return () => {
      source.close();
      sessionFeed.close();
      abandon();
    };
  }

  #run(event: MessageEvent<string>) {
    let update;
    try {
      update = runUpdate(JSON.parse(event.data), '');
    } catch {
      return;
    }
    if (update.kind === 'state') {
      // A process is read as soon as the page selects it, which can be
      // before the daemon has stored it: read it again, once, when it has
      // opened.
      if (
        update.run === this.#selected() &&
        update.state.phase !== 'starting' &&
        !this.#reread.has(update.run)
      ) {
        this.#reread.add(update.run);
        void this.readConversation(update.run, 'again');
      }
      if (!this.live.some((run) => run.run === update.run)) {
        void this.readLive();
      } else {
        this.live = Runs.update(this.live, update);
      }
      if (update.state.phase === 'ended' || update.state.phase === 'idle') void this.readSessions();
      return;
    }
    this.transcripts.set(
      update.run,
      (this.transcripts.get(update.run) ?? new Transcript()).apply(update.event, update.run),
    );
  }
}
