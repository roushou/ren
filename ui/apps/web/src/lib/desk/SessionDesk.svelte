<script lang="ts">
  import Toolbar from '#lib/Toolbar.svelte';
  import { onMount, tick, untrack } from 'svelte';
  import { SvelteMap } from 'svelte/reactivity';
  import { goto } from '$app/navigation';
  import { page } from '$app/state';
  import {
    ManagementApi,
    clock,
    short,
    type CallView,
    type RecordingId,
    type Shelf,
    type TokenTotals,
  } from '#lib/api.ts';
  import { ago } from '#lib/overview.ts';
  import { harnessLabel } from '#lib/activity.ts';
  import { callStatus, spentSummary, statusBadge, tokenSummary } from '#lib/display.ts';
  import {
    Deletions,
    Desk,
    RunCommandError,
    Runs,
    RunsClient,
    Transcript,
    UNSTARTED,
    runUpdate,
    type Decision,
    type DeskRun,
    type Entry,
    type Remembered,
    Compose,
    Setup,
    type RunClient,
    type Drivable,
    type RunPlace,
    type Deletion,
    type DeletionEvent,
    type HistoryChange,
    type Read,
    type RunId,
    type RunSnapshot,
    type Waiting,
    Sessions,
    Memory,
    MIDWAY,
    DRIVABLE,
    type DeskSession,
    type SessionBody,
    type SessionScope,
    type SessionId,
    type StoredConversation,
    type ClientCommand,
    type FileMatch,
  } from '#lib/runs.ts';
  import { Mentions } from '#lib/mention.ts';
  import { Refusals } from '#lib/refusal.ts';
  import { Outbox, type Outgoing, type OutgoingDelivery, type Starts } from '#lib/outbox.ts';
  import {
    Gallery,
    Tray,
    type AttachmentRead,
    type Chip,
    type KeptAttachment,
    type UploadedImage,
  } from '#lib/attachments.ts';
  import ComposerMenu, { type MenuEntry } from './ComposerMenu.svelte';
  import CommandStep from './CommandStep.svelte';
  import QuestionCard from './QuestionCard.svelte';
  import StepGroup from './StepGroup.svelte';
  import { Steps } from '#lib/steps.ts';
  import Icon from '#lib/Icon.svelte';
  import Markdown from 'markdown/Markdown.svelte';
  import ActionMenu, { type ActionItem } from '#lib/ActionMenu.svelte';
  import { assertNever } from '#lib/decode.ts';
  import FolderPicker from './FolderPicker.svelte';
  import ImageViewer from './ImageViewer.svelte';
  import SignInSheet from '#lib/SignInSheet.svelte';
  import { SignIns } from '#lib/signIn.ts';
  import RunSettingsBar from './RunSettingsBar.svelte';
  import SessionList, { type Listing } from './SessionList.svelte';
  import Skeleton from '#lib/Skeleton.svelte';
  import type { Arriving } from '#lib/arrival.ts';
  import type { Snapshot } from '#lib/snapshots.ts';
  import type { ReplyRequest } from '#lib/wire/ReplyRequest.ts';
  import type { DeskData } from './load.ts';

  let {
    data,
    scope,
  }: {
    data: DeskData;
    /** Which sessions this desk holds: one project's, or Scratch. */
    scope: SessionScope;
  } = $props();

  const client = new RunsClient();
  const api = new ManagementApi();

  /** What the server read for the first render; the feed follows from here.
   * Null when it read nothing: runs are off, or the page came by a
   * navigation in the browser and reads it all itself. */
  const initial = untrack(() => (data.opening.kind === 'read' ? data.opening.start : null));
  const first = <T,>(read: Read<T> | undefined, none: T): T =>
    read?.state === 'read' ? read.value : none;
  let live = $state.raw<RunSnapshot[]>(first(initial?.live, []));
  /** Every session, current and archived, as stored and served. */
  let everySession = $state.raw<SessionBody[]>(first(initial?.sessions, []));
  let everyArchived = $state.raw<SessionBody[]>(first(initial?.archivedSessions, []));
  /** Those this desk holds. */
  const sessions = $derived(Sessions.within(everySession, scope));
  const archivedSessions = $derived(Sessions.within(everyArchived, scope));
  /** Whether the lists have been read: on a navigation in the browser they
   * are on their way, and the list draws its outline until they come. */
  let listing = $state<Listing>(
    untrack(() => (data.opening.kind === 'unread' ? 'pending' : 'read')),
  );
  /** Which sessions the list shows. */
  let shelf = $state<Shelf>('current');
  /** A session's delete: asked, confirmed, and sent, one at a time. */
  let deletion = $state.raw<Deletion>(Deletions.idle);
  /** Why runs cannot be listed or driven, when they cannot. */
  let unavailable = $state<string | null>(
    initial?.live.state === 'failed' ? initial.live.reason : null,
  );
  /** The runs feed: opening for the first time, open, lost and being
   * reopened by the browser, or given up on. Only a lost or given-up feed
   * is worth a word in the header; the first opening is not a reconnect. */
  let feed = $state<'opening' | 'open' | 'reconnecting' | 'closed'>('opening');
  /** Each run's conversation since this page started following it. */
  const transcripts = new SvelteMap<RunId, Transcript>();
  // The selected run's conversation, as the page was loaded with it.
  untrack(() => {
    const given = data.conversation;
    if (given?.conversation.state === 'ready' && given.conversation.snapshot.value) {
      transcripts.set(given.run, new Transcript().load(given.conversation.snapshot.value));
    }
  });
  /** The session the page shows, from the URL. */
  const chosen = $derived(data.session);
  /** On a phone the page shows one pane at a time: the list, a session, or
   * the new-session form that New session opened. */
  let starting = $state(false);
  const pane = $derived<'list' | 'session' | 'new'>(chosen ? 'session' : starting ? 'new' : 'list');
  /** The folder scratch sessions get theirs in; null when there is none. */
  let scratch = $state<string | null>(first(initial?.places, null)?.scratch ?? null);
  const folders = $derived(
    // A project's desk has no Scratch group: a session started there would
    // leave the project.
    Sessions.folders(
      shelf === 'current' ? sessions : archivedSessions,
      live,
      scope.kind === 'project' ? null : scratch,
      chosen,
    ),
  );
  const session = $derived.by((): DeskSession | null => {
    const body = [...sessions, ...archivedSessions].find((entry) => entry.session === chosen);
    return body ? Sessions.desk(body, live) : null;
  });
  /** The run the page shows: the session's newest process. */
  const selected = $derived(session?.run ?? null);
  const current = $derived.by((): DeskRun | null => {
    if (!session?.run) return null;
    const run = session.run;
    const record = session.runs.find((entry) => entry.id === run) ?? null;
    return {
      run,
      harness: session.harness,
      workdir: session.workdir,
      started_at: record?.started_at ?? null,
      state: live.find((entry) => entry.run === run)?.state ?? null,
      recorded: record,
      settings: session.settings,
    };
  });
  const runState = $derived(current ? Desk.state(current) : null);
  /** What can be asked of it: of its client, or, with none running, a prompt. */
  const controls = $derived(runState ? Runs.controls(runState) : session ? UNSTARTED : null);
  /**
   * How the session stands, in its header: while its client runs, when it
   * could not start, or that it is a draft. Resting, it says nothing: the
   * next prompt starts its client again.
   */
  const headerMark = $derived.by(() => {
    if (!session) return null;
    const marked = Sessions.mark(session, '');
    return marked.label === '' ? null : marked;
  });
  const transcript = $derived(selected ? (transcripts.get(selected) ?? new Transcript()) : null);
  const waiting = $derived(folders.reduce((sum, folder) => sum + folder.waiting, 0));
  const liveCount = $derived(live.filter((run) => run.state.phase !== 'ended').length);

  /** The selected run's stored conversation: on its way, read, or why it
   * could not be. */
  type ConversationRead =
    | { run: RunId; state: 'pending' }
    | { run: RunId; state: 'read' }
    | { run: RunId; state: 'failed'; reason: string };
  let conversationRead = $state.raw<ConversationRead | null>(null);
  /** Why the selected run's conversation could not be read. */
  const readError = $derived(
    conversationRead?.state === 'failed' && conversationRead.run === selected
      ? conversationRead.reason
      : null,
  );
  /** The selected run's conversation is on its way, with nothing of it read
   * here yet: the pane draws its outline. */
  const conversationPending = $derived(
    conversationRead?.state === 'pending' && conversationRead.run === selected && !transcript?.read,
  );

  /** The projects the daemon offers to start a run in. */
  let offered = $state.raw<RunPlace[]>(first(initial?.places, null)?.places ?? []);
  /** The clients a run can start with, and whether each is ready. */
  let clients = $state.raw<RunClient[]>(first(initial?.clients, []));
  /** The client whose sign-in sheet is open. */
  let signingIn = $state<Drivable | null>(null);
  const tabs = [
    ['conversation', 'Conversation'],
    ['activity', 'Activity'],
    ['files', 'Files'],
    ['calls', 'Calls'],
  ] as const;
  /** The turns a conversation's outline draws: each prompt's width and its
   * reply's lines, varied so they read as text. */
  const OUTLINE_TURNS = [
    { prompt: '46%', reply: ['88%', '74%', '52%'] },
    { prompt: '31%', reply: ['81%', '63%'] },
    { prompt: '57%', reply: ['85%', '90%', '68%', '40%'] },
  ] as const;
  /** What the selected run shows. */
  let tab = $state<(typeof tabs)[number][0]>('conversation');
  /** The selected run's model calls, read when its Calls tab opens. */
  let calls = $state.raw<{ recording: RecordingId; calls: CallView[] } | null>(null);
  /** Now, for how long ago runs started; moves every half minute. */
  let now = $state(Date.now());
  let busy = $state(false);
  let notice = $state<string | null>(null);
  let conversation = $state<HTMLElement>();
  let composer = $state<HTMLTextAreaElement>();
  /** Follow the newest entry unless the operator scrolled up. */
  let following = true;

  function message(error: unknown): string {
    if (error instanceof RunCommandError) return error.message;
    if (error instanceof TypeError) return 'Cannot reach the Ren UI server.';
    return error instanceof Error ? error.message : 'Something went wrong.';
  }

  async function readLive() {
    if (!data.runsControl) return;
    try {
      live = await client.live();
      unavailable = null;
    } catch (error) {
      unavailable = message(error);
    }
  }

  async function readProjects() {
    if (!data.runsControl) return;
    try {
      ({ places: offered, scratch } = await client.places());
    } catch {
      // The list is a convenience; a typed path still starts a run.
    }
  }

  async function readClients() {
    if (!data.runsControl) return;
    try {
      clients = await client.clients();
    } catch {
      // Without the list, both drivable clients are offered.
    }
  }

  /** The tokens the chosen session's calls used, as the gateway recorded
   * them: read when it opens and again as each turn ends. */
  let spent = $state.raw<{ session: SessionId; tokens: TokenTotals } | null>(null);
  async function readSpent(target: SessionId) {
    try {
      const usage = await api.usage({ session: target });
      if (chosen === target) spent = { session: target, tokens: usage.totals.tokens };
    } catch {
      // Without it, the header leaves the tokens out.
    }
  }
  const spentLine = $derived(spent && spent.session === chosen ? spentSummary(spent.tokens) : null);

  async function readCalls(recording: RecordingId) {
    try {
      const page = await api.recordingCalls(recording, null);
      calls = { recording, calls: page.calls };
    } catch (error) {
      notice = message(error);
    }
  }

  /** What a session was asked first: as stored, else from its conversation here. */
  function title(entry: { run: RunId | null; asked?: string | null }): string | null {
    const text =
      entry.asked ?? (entry.run ? (transcripts.get(entry.run)?.firstPrompt() ?? null) : null);
    return text ? text.split('\n')[0]!.trim() || null : null;
  }

  async function readSessions() {
    if (!data.runsControl) return;
    try {
      [everySession, everyArchived] = await Promise.all([
        client.sessions(),
        client.sessions('archived'),
      ]);
      listing = 'read';
    } catch {
      // The list stays as it was; the feed brings changes.
      if (listing === 'pending') listing = 'failed';
    }
  }

  /** Whether `session` is on either shelf of the lists as last read. */
  function listed(session: SessionId, bodies: SessionBody[]): boolean {
    return bodies.some((entry) => entry.session === session);
  }

  /**
   * Read the lists again until `moved` holds: the daemon applies a history
   * change after what it is recording. Whether it held by the last read.
   */
  async function until(moved: () => boolean): Promise<boolean> {
    for (const delay of [150, 400, 1000, 2000]) {
      await new Promise((resolve) => setTimeout(resolve, delay));
      await readSessions();
      if (moved()) return true;
    }
    return false;
  }

  /**
   * Archive or unarchive the selected session. Archiving stops its process.
   */
  async function edit(change: Exclude<HistoryChange, 'delete'>) {
    const target = chosen;
    if (!target) return;
    await act(async () => {
      await client.toSession(target, { kind: change });
      await until(() => listed(target, change === 'archive' ? archivedSessions : sessions));
      if (change === 'archive') select(null);
    });
  }

  /** The delete after `event`; an event that does not apply is a fault here. */
  function deleting(event: DeletionEvent) {
    const next = Deletions.step(deletion, event);
    if (Deletions.illegal(next)) {
      console.warn(`A session delete cannot ${next.event} while ${next.state}.`);
      return;
    }
    deletion = next;
    if (event.kind === 'confirm' && next.phase === 'deleting') void remove(next.session);
  }

  /**
   * Delete a session: it goes with its ended runs and their recorded calls,
   * even one whose client never ran. It is done when the lists no longer
   * hold it; a delete the daemon took but has not applied by the last read
   * says so and can be sent again.
   */
  async function remove(target: SessionId) {
    try {
      await client.toSession(target, { kind: 'delete' });
      const gone = await until(
        () => !listed(target, sessions) && !listed(target, archivedSessions),
      );
      if (!gone && deletion.phase === 'deleting' && deletion.session === target) {
        deleting({
          kind: 'refused',
          reason: 'Asked, but still listed: the daemon may apply it shortly.',
        });
      }
    } catch (error) {
      deleting({ kind: 'refused', reason: message(error) });
    }
  }

  /** The delete asked in the header, of the session it shows. */
  const asking = $derived(chosen ? Deletions.of(deletion, chosen, 'header') : null);

  // A delete is done when its session has left both shelves.
  $effect(() => {
    if (deletion.phase === 'idle' || deletion.phase === 'confirming') return;
    const target = deletion.session;
    if (listed(target, sessions) || listed(target, archivedSessions)) return;
    untrack(() => {
      deleting({ kind: 'gone' });
      if (chosen === target) select(null);
    });
  });

  /**
   * Read the run's conversation, whole, as the daemon stored it, keeping
   * what streamed in meanwhile; and its record, for its session. Shown, the
   * conversation stands as pending while it is read; read again quietly,
   * what is shown stays until the read lands, and a failure changes nothing.
   */
  async function readConversation(run: RunId, how: 'shown' | 'again' = 'shown') {
    if (how === 'shown' && selected === run) conversationRead = { run, state: 'pending' };
    try {
      const conversation = await client.conversation(run);
      transcripts.set(run, (transcripts.get(run) ?? new Transcript()).load(conversation));
      if (selected === run) conversationRead = { run, state: 'read' };
    } catch (error) {
      if (how === 'shown' && selected === run)
        conversationRead = { run, state: 'failed', reason: message(error) };
    }
  }

  /**
   * Take the conversation the page was loaded with: at once when it was
   * read, or once it arrives. A read that failed is made again here, which
   * says why when it cannot.
   */
  function adopt(run: RunId, given: Arriving<StoredConversation>) {
    if (given.state === 'ready') {
      took(run, given.snapshot);
      return;
    }
    conversationRead = { run, state: 'pending' };
    void given.snapshot.then((snapshot) => took(run, snapshot));
  }

  function took(run: RunId, snapshot: Snapshot<StoredConversation>) {
    if (snapshot.value === null) {
      if (selected === run) void readConversation(run);
      return;
    }
    transcripts.set(run, (transcripts.get(run) ?? new Transcript()).load(snapshot.value));
    if (selected === run) conversationRead = { run, state: 'read' };
  }

  async function act(action: () => Promise<unknown>) {
    busy = true;
    notice = null;
    try {
      await action();
    } catch (error) {
      notice = message(error);
    } finally {
      busy = false;
    }
  }

  function select(next: SessionId | null, how: 'push' | 'replace' = 'push') {
    void goto(next ? `?session=${next}` : page.url.pathname, {
      replaceState: how === 'replace',
    });
  }

  /** The client and settings last chosen here: a new session starts with them. */
  let remembered = $state<Remembered>(untrack(() => Memory.parse(data.remembered)));

  function remember(next: Remembered) {
    remembered = next;
    // Remembering is a convenience: a refused cookie changes nothing else.
    document.cookie = Memory.cookie(next);
  }

  /**
   * A new session, opened with its prompt ready to write: a draft, which
   * runs no client until its first prompt. In `dir` (null: a new scratch
   * folder of its own), or where the last session asked something worked.
   * A draft never asked anything is reused, moved there, rather than
   * another made; a new one is on the client last chosen, with the settings
   * kept for it that still apply.
   */
  async function newDraft(dir?: string | null) {
    // A project's desk starts in the project's repository; Scratch, in a
    // scratch folder of its own.
    const place =
      dir !== undefined ? dir : scope.kind === 'project' ? scope.project.repository : null;
    const unused = Sessions.unused(sessions, live);
    await act(async () => {
      if (unused) {
        const here =
          place === undefined ||
          (place === null ? Sessions.inScratch(unused.workdir, scratch) : unused.workdir === place);
        if (!here) {
          await client.toSession(unused.session, { kind: 'folder', dir: place });
          await readSessions();
        }
        select(unused.session);
      } else {
        const harness = remembered.harness;
        // The daemon keeps each client's models; without them, the client's own.
        const models = await client.models(harness).catch(() => []);
        const policies = clients.find((entry) => entry.harness === harness)?.approvals ?? [];
        // With no session to follow and no scratch folder, the first place offered.
        const folder = place === undefined ? (offered[0]?.path ?? null) : place;
        const made = await client.createSession(
          harness,
          folder,
          Memory.settings(remembered, harness, models, policies),
        );
        await Promise.all([readSessions(), readLive()]);
        select(made);
      }
      await tick();
      composer?.focus();
    });
  }

  /** Move the open draft to `dir` (null: a new scratch folder of its own). */
  function moveDraft(dir: string | null) {
    const target = chosen;
    if (!target) return;
    void act(async () => {
      await client.toSession(target, { kind: 'folder', dir });
      await readSessions();
    });
  }

  /** The sessions in the list's order, for j and k. */
  const order = $derived(folders.flatMap((folder) => folder.sessions));

  function typing(target: EventTarget | null): boolean {
    return (
      target instanceof HTMLElement &&
      (target.isContentEditable || ['INPUT', 'TEXTAREA', 'SELECT'].includes(target.tagName))
    );
  }

  function shortcut(event: KeyboardEvent) {
    if (event.metaKey || event.ctrlKey || event.altKey || typing(event.target)) return;
    switch (event.key) {
      case 'n':
        event.preventDefault();
        void newDraft();
        break;
      case 'j':
      case 'k': {
        if (!order.length) return;
        event.preventDefault();
        const at = order.findIndex((entry) => entry.session === chosen);
        const next = event.key === 'j' ? Math.min(at + 1, order.length - 1) : Math.max(at - 1, 0);
        select(order[at === -1 ? 0 : next]!.session);
        break;
      }
      case 'y':
      case 'a':
      case 'd': {
        const waiting = controls?.approvals[0];
        if (!waiting) return;
        event.preventDefault();
        answer(
          waiting,
          event.key === 'y' ? 'accept' : event.key === 'a' ? 'accept_for_session' : 'decline',
        );
        break;
      }
    }
  }

  /** The key that sends into a running turn, as this platform names it. */
  const navigatorKey =
    typeof navigator !== 'undefined' && /Mac|iPhone|iPad/.test(navigator.platform) ? '⌘' : 'Ctrl';
  /** The processes whose conversation was read again once they opened. */
  const reread = new Set<RunId>();

  /** Groups of steps the operator opened, by their first step: the rest
   * stay folded, running or failed alike. */
  const folds = new SvelteMap<string, boolean>();

  /** Each session's draft, kept while the page is open. */
  const drafts = new SvelteMap<SessionId, string>();
  /** How far back ↑ has recalled the run's prompts; null: not recalling. */
  let recalling = $state<number | null>(null);
  /** What was written before recalling, for ↓ to come back to. */
  let beforeRecall = '';
  const draft = $derived(chosen ? (drafts.get(chosen) ?? '') : '');
  function write(text: string) {
    if (chosen) drafts.set(chosen, text);
  }
  /** Every session's prompts on their way, shown until the conversation shows them. */
  let outbox = $state.raw(new Outbox());
  const outgoing = $derived(chosen ? outbox.of(chosen) : []);
  /** One of the session's prompts has no answer from the daemon yet: the next waits for it. */
  const sending = $derived(chosen !== null && outbox.waiting(chosen));
  $effect(() => {
    // A prompt the conversation shows is the client's echo of one on its way.
    const target = chosen;
    const prompts = transcript?.prompts() ?? [];
    if (target) untrack(() => (outbox = outbox.shown(target, prompts)));
  });
  /** Each session's images about to go with its next prompt. */
  const trays = new SvelteMap<SessionId, Tray>();
  const tray = $derived(chosen ? (trays.get(chosen) ?? new Tray()) : new Tray());
  /** Each session's attachments as the daemon lists them, to show what a prompt sent. */
  const kept = new SvelteMap<SessionId, KeptAttachment[]>();
  const gallery = $derived(new Gallery(chosen ? (kept.get(chosen) ?? []) : []));
  /** The image open full size over the page, if one is. */
  let viewing = $state.raw<{ src: string; read: AttachmentRead } | null>(null);
  /** Whether the composer offers images: not for a client that takes none. */
  const imagesTaken = $derived(session?.images !== 'never');
  let picker = $state<HTMLInputElement>();
  /** Something is being dragged over the composer. */
  let dropping = $state(false);

  async function readAttachments(target: SessionId) {
    if (!data.runsControl) return;
    try {
      kept.set(target, await client.attachments(target));
    } catch {
      // Without the listing, sent images show by name.
    }
  }

  $effect(() => {
    const target = chosen;
    if (target) untrack(() => void readAttachments(target));
  });

  /** Upload each image to the session; each is a chip until it is sent. */
  async function addImages(files: readonly File[]) {
    const target = chosen;
    if (!target || !imagesTaken) return;
    for (const file of files) {
      const start = (trays.get(target) ?? new Tray()).adding(file.name || 'image');
      trays.set(target, start.tray);
      const settle = (change: (now: Tray) => Tray) =>
        trays.set(target, change(trays.get(target) ?? new Tray()));
      if (!Gallery.takes(file.type)) {
        settle((now) => now.refused(start.key, Refusals.say('image_required')));
        continue;
      }
      try {
        const bytes = await file.arrayBuffer();
        const made = await client.attach(target, file.name || 'image', file.type, bytes);
        settle((now) => now.uploaded(start.key, made.id, made.read));
      } catch (error) {
        const code = error instanceof RunCommandError ? error.code : null;
        settle((now) => now.refused(start.key, code ? Refusals.say(code) : message(error)));
      }
    }
  }

  /** Take an image out of the tray; one uploaded is removed from the session. */
  async function removeChip(chip: Chip) {
    const target = chosen;
    if (!target) return;
    if (chip.state === 'ready') {
      try {
        await client.detach(target, chip.id);
      } catch (error) {
        notice = message(error);
        return;
      }
    }
    trays.set(target, (trays.get(target) ?? new Tray()).without(chip.key));
  }

  function imagesOf(transfer: DataTransfer | null): File[] {
    return [...(transfer?.files ?? [])].filter((file) => file.type.startsWith('image/'));
  }
  /** Where the caret is in the prompt, for what the word there asks to be offered. */
  let caret = $state(0);
  /** Where an offer closed with Esc starts: it stays closed until another word asks. */
  let dismissed = $state<number | null>(null);
  // A closed composer's prompt is disabled, so nothing asks there.
  const mention = $derived(Mentions.at(draft, Math.min(caret, draft.length)));
  const offering = $derived(mention && mention.start !== dismissed ? mention : null);
  /** Each run's commands, as its client listed them. */
  /** Each session's commands, by its client and folder, as its client lists them. */
  const commandLists = new SvelteMap<string, ClientCommand[]>();
  const commandsKey = $derived(
    session ? `${session.session} ${session.harness} ${session.workdir}` : null,
  );
  let commandsNote = $state<string | null>(null);
  /** The newest file search's answer, with what it searched for. */
  let found = $state<{ session: SessionId; query: string; files: FileMatch[] } | null>(null);
  let filesNote = $state<string | null>(null);
  let active = $state(0);
  const menuId = 'composer-offers';

  $effect(() => {
    // The session's commands, read as it opens, before any `/`: its running
    // client's list, or what its client offers in its folder (listed once
    // per client and folder by starting it briefly there). An empty list is
    // asked again when a word asks for commands.
    const key = commandsKey;
    const target = chosen;
    const harness = session?.harness;
    const asking = offering?.kind === 'command' ? offering.start : null;
    if (!key || !target || !harness) return;
    const known = untrack(() => commandLists.get(key));
    if (known && (known.length > 0 || asking === null)) return;
    commandsNote = 'Reading the commands…';
    const abort = new AbortController();
    client
      .sessionCommands(target, abort.signal)
      .then((commands) => {
        commandLists.set(key, commands);
        commandsNote =
          commands.length > 0 ? null : `${harnessLabel(harness)} offers no commands here.`;
      })
      .catch((error: unknown) => {
        if (!abort.signal.aborted) commandsNote = message(error);
      });
    return () => abort.abort();
  });

  $effect(() => {
    // A short pause after typing, then the folder is searched.
    const target = chosen;
    const query = offering?.kind === 'file' ? offering.query : null;
    if (!target || query === null) return;
    const abort = new AbortController();
    const timer = setTimeout(() => {
      client
        .sessionFiles(target, query, 20, abort.signal)
        .then((files) => {
          found = { session: target, query, files };
          filesNote = files.length > 0 ? null : 'No file matches.';
        })
        .catch((error: unknown) => {
          if (!abort.signal.aborted) filesNote = message(error);
        });
    }, 90);
    return () => {
      clearTimeout(timer);
      abort.abort();
    };
  });

  const offeredCommands = $derived(
    offering?.kind === 'command'
      ? Mentions.commands(commandLists.get(commandsKey ?? '') ?? [], offering)
      : [],
  );
  const offeredFiles = $derived(
    offering?.kind === 'file' && found && found.session === chosen ? found.files : [],
  );
  const entries = $derived.by((): MenuEntry[] => {
    if (offering?.kind === 'command') {
      return offeredCommands.map((command) => {
        const at = command.name.toLowerCase().indexOf(offering.query.toLowerCase());
        const hits =
          offering.query === '' || at < 0
            ? []
            : Array.from(offering.query, (_, index) => command.sigil.length + at + index);
        return {
          key: command.invocation,
          parts: Mentions.marked(command.invocation, hits),
          hint: command.hint,
          tag: command.kind === 'command' ? null : command.kind,
          detail: command.description,
        };
      });
    }
    if (offering?.kind === 'file') {
      return offeredFiles.map((file) => ({
        key: file.path,
        parts: Mentions.marked(file.path, file.positions),
        hint: null,
        tag: file.kind === 'directory' ? 'folder' : null,
        detail: '',
      }));
    }
    return [];
  });
  $effect(() => {
    // A new list starts at its best entry.
    void entries;
    active = 0;
  });

  /** Write the chosen entry in place of the word that asked for it. */
  function pick(index: number) {
    if (!offering) return;
    const written =
      offering.kind === 'command'
        ? offeredCommands[index] && Mentions.command(draft, offering, offeredCommands[index])
        : offeredFiles[index] && Mentions.file(draft, offering, offeredFiles[index]);
    if (!written) return;
    write(written.text);
    caret = written.caret;
    void tick().then(() => {
      composer?.focus();
      composer?.setSelectionRange(written.caret, written.caret);
    });
  }

  function trackCaret(event: Event) {
    caret = (event.currentTarget as HTMLTextAreaElement).selectionStart;
  }

  /** What the selected run's client takes mid-run. */
  const client_ = $derived(clients.find((entry) => entry.harness === session?.harness) ?? null);
  /** The session's client is signed out here: a prompt would only fail, so none is sent. */
  const signedOut = $derived(client_ !== null && SignIns.signedOut(client_.sign_in));
  let checking = $state(false);
  /** Read the client's sign-in again: it may have been signed in another way. */
  async function checkSignIn(harness: Drivable) {
    checking = true;
    try {
      await client.checkSignIn(harness);
    } catch {
      // The line stays as it is; the next read of the clients says.
    } finally {
      checking = false;
    }
    await readClients();
  }

  /**
   * Send what is written to the session: it starts a turn (starting or
   * resuming its client when none runs), or, while a turn runs, is queued
   * for after it or steers it. A prompt that starts or steers a turn shows
   * in the conversation at once, marked with how far it has got.
   */
  async function send(delivery: 'queue' | 'steer' = 'queue') {
    const text = draft.trim();
    const target = chosen;
    const images = tray.ready();
    if (!target || (!text && images.length === 0) || tray.uploading()) return;
    if (!compose || signedOut || outbox.waiting(target)) return;
    switch (compose.kind) {
      case 'start':
      case 'continue':
        return deliver(target, text, images, 'start');
      case 'during':
        if (delivery === 'steer' && compose.steer) return deliver(target, text, images, 'steer');
        return queue(target, text, compose.queued);
      case 'closed':
        return;
      default:
        return assertNever(compose);
    }
  }

  /** Show the prompt as on its way, clear the composer, and send it. */
  function deliver(
    target: SessionId,
    text: string,
    images: UploadedImage[],
    how: OutgoingDelivery,
  ) {
    const starts: Starts = session && Sessions.running(session) ? 'nothing' : 'client';
    const made = outbox.sending(
      { session: target, text, images, delivery: how },
      transcript?.prompts() ?? [],
      starts,
    );
    outbox = made.outbox;
    trays.set(target, (trays.get(target) ?? new Tray()).sent());
    drafts.delete(target);
    recalling = null;
    following = true;
    void transmit(made.key);
  }

  /** Ask the daemon to take prompt `key`, and mark what it answered. */
  async function transmit(key: number) {
    const item = outbox.items.find((one) => one.key === key);
    if (!item) return;
    try {
      await client.toSession(item.session, {
        kind: 'prompt',
        text: item.text,
        delivery: item.delivery,
        attachments: item.images.map((image) => image.id),
      });
      outbox = outbox.sent(key);
    } catch (error) {
      outbox = outbox.failed(key, error);
    }
    // The feeds say the same soon; read now so the page catches up at once.
    void Promise.all([readSessions(), readLive(), readAttachments(item.session)]);
  }

  /** Queue the prompt for after the running turn, after what was queued before. */
  async function queue(target: SessionId, text: string, queued: string | null) {
    const attachments = tray.ids();
    await act(async () => {
      await client.toSession(target, {
        kind: 'prompt',
        text: queued ? `${queued}\n\n${text}` : text,
        delivery: 'queue',
        attachments,
      });
      trays.set(target, (trays.get(target) ?? new Tray()).sent());
      drafts.delete(target);
      recalling = null;
      following = true;
    });
    void Promise.all([readSessions(), readLive(), readAttachments(target)]);
  }

  /** Send a prompt that was refused, or whose outcome is unknown, again. */
  function resend(item: Outgoing) {
    outbox = outbox.without(item.key);
    deliver(item.session, item.text, item.images, item.delivery);
  }

  /** Take a prompt that did not go back into the composer, with its images. */
  function takeBack(item: Outgoing) {
    outbox = outbox.without(item.key);
    const written = drafts.get(item.session) ?? '';
    drafts.set(item.session, written ? `${item.text}\n\n${written}` : item.text);
    trays.set(item.session, (trays.get(item.session) ?? new Tray()).restored(item.images));
    if (item.session === chosen) void tick().then(() => composer?.focus());
  }

  function answer(waiting: Waiting, decision: Decision) {
    const target = chosen;
    if (!target) return;
    void act(() =>
      client.toSession(target, { kind: 'answer', approval: waiting.approval, decision }),
    );
  }

  function reply(waiting: Waiting, request: ReplyRequest) {
    const target = chosen;
    if (!target) return;
    void act(() =>
      client.toSession(target, { kind: 'reply', approval: waiting.approval, request }),
    );
  }

  /** Take the queued prompt back into the composer to change it. */
  async function editQueued(queued: string) {
    const target = chosen;
    if (!target) return;
    await act(async () => {
      await client.toSession(target, { kind: 'unqueue' });
      write(draft ? `${queued}\n\n${draft}` : queued);
      composer?.focus();
    });
  }

  function composerKey(event: KeyboardEvent) {
    if (event.isComposing) return;
    // While something is offered, the arrows, Enter, and Tab are its.
    if (offering) {
      const count = entries.length;
      if (event.key === 'Escape') {
        event.preventDefault();
        dismissed = offering.start;
        return;
      }
      if (count > 0 && (event.key === 'ArrowDown' || event.key === 'ArrowUp')) {
        event.preventDefault();
        active = (active + (event.key === 'ArrowDown' ? 1 : count - 1)) % count;
        return;
      }
      if (
        count > 0 &&
        (event.key === 'Tab' ||
          (event.key === 'Enter' && !event.shiftKey && !event.metaKey && !event.ctrlKey))
      ) {
        event.preventDefault();
        pick(active);
        return;
      }
    }
    if (event.key === 'Enter' && !event.shiftKey) {
      event.preventDefault();
      // Cmd or Ctrl+Enter sends into the running turn, where the client can.
      void send(event.metaKey || event.ctrlKey ? 'steer' : 'queue');
      return;
    }
    const field = event.currentTarget as HTMLTextAreaElement;
    const atStart = field.selectionStart === 0 && field.selectionEnd === 0;
    if (event.key === 'ArrowUp' && atStart && (draft === '' || recalling !== null)) {
      const prompts = selected ? (transcripts.get(selected)?.prompts() ?? []) : [];
      if (recalling === null) beforeRecall = draft;
      const next = Compose.recall(prompts, recalling, true);
      if (!next) return;
      event.preventDefault();
      recalling = next.at;
      write(next.text);
    } else if (event.key === 'ArrowDown' && recalling !== null) {
      const prompts = selected ? (transcripts.get(selected)?.prompts() ?? []) : [];
      const next = Compose.recall(prompts, recalling, false);
      if (!next) return;
      event.preventDefault();
      recalling = next.at;
      write(next.at === null ? beforeRecall : next.text);
    }
  }

  // The conversation scrolls from its end (`column-reverse`): its newest
  // entry is at scrollTop 0, and scrolling up makes scrollTop negative.
  function onScroll() {
    if (!conversation) return;
    following = Math.abs(conversation.scrollTop) < 48;
  }

  $effect(() => {
    // Keep the newest entry in view as the conversation grows.
    void transcript?.entries;
    if (!following || !conversation) return;
    void tick().then(() => conversation?.scrollTo({ top: 0 }));
  });

  $effect(() => {
    const run = selected;
    conversationRead = null;
    calls = null;
    // A question asked in the header is about the session it showed.
    untrack(() => {
      if (
        (deletion.phase === 'confirming' || deletion.phase === 'failed') &&
        deletion.from === 'header'
      )
        deleting({ kind: 'cancel' });
    });
    tab = 'conversation';
    following = true;
    if (!run) return;
    // Loaded with the page when it was opened on this run; read otherwise.
    // Untracked: the effect follows the selection, not the transcripts it
    // writes, or writing one would run it again.
    untrack(() => {
      const given = data.conversation;
      if (given?.run === run) {
        adopt(run, given.conversation);
      } else {
        void readConversation(run);
      }
    });
  });

  $effect(() => {
    // A session's tokens are read as it opens, and again once a turn ends:
    // not while one runs, when they are still coming in.
    const target = chosen;
    const settled = runState?.phase !== 'starting' && runState?.phase !== 'working';
    if (target && settled) untrack(() => void readSpent(target));
  });

  $effect(() => {
    // The Calls tab reads the calls of its session's recording when it opens.
    const recording = current?.recorded?.recording ?? null;
    if (tab === 'calls' && recording && calls?.recording !== recording) void readCalls(recording);
  });

  $effect(() => {
    // The composer is where the keyboard goes once a run takes prompts.
    if (controls?.prompt) composer?.focus({ preventScroll: true });
  });

  onMount(() => {
    const ticking = setInterval(() => (now = Date.now()), 30_000);
    // The server read them for the first render; only what it could not
    // read is asked for again. Live runs are read once the feed opens.
    if (initial?.places.state !== 'read') void readProjects();
    if (initial?.clients.state !== 'read') void readClients();
    if (initial?.sessions.state !== 'read' || initial.archivedSessions.state !== 'read')
      void readSessions();
    if (!initial) void readLive();
    if (!data.runsControl) return () => clearInterval(ticking);
    const source = new EventSource('/control/runs/events');
    source.addEventListener('open', () => {
      feed = 'open';
      // Whatever changed while the feed was closed.
      void readLive();
    });
    source.addEventListener('error', () => {
      feed = source.readyState === EventSource.CLOSED ? 'closed' : 'reconnecting';
    });
    source.addEventListener('lagged', () => void readLive());
    // Sessions change as they are made, configured, archived, or get a new
    // process: each change reads the list again.
    const sessionFeed = new EventSource('/control/sessions/events');
    sessionFeed.addEventListener('session', () => void readSessions());
    sessionFeed.addEventListener('lagged', () => void readSessions());
    source.addEventListener('run', (event) => {
      let update;
      try {
        update = runUpdate(JSON.parse((event as MessageEvent<string>).data), '');
      } catch {
        return;
      }
      if (update.kind === 'state') {
        // A process is read as soon as the page selects it, which can be
        // before the daemon has stored it: read it again, once, when it
        // has opened.
        if (
          update.run === selected &&
          update.state.phase !== 'starting' &&
          !reread.has(update.run)
        ) {
          reread.add(update.run);
          void readConversation(update.run, 'again');
        }
        if (!live.some((run) => run.run === update.run)) {
          void readLive();
        } else {
          live = Runs.update(live, update);
        }
        if (update.state.phase === 'ended' || update.state.phase === 'idle') void readSessions();
        return;
      }
      transcripts.set(
        update.run,
        (transcripts.get(update.run) ?? new Transcript()).apply(update.event, update.run),
      );
    });
    return () => {
      source.close();
      sessionFeed.close();
      clearInterval(ticking);
    };
  });

  /** Its first prompt, whole, for the title's tooltip. */
  function firstPrompt(run: DeskRun): string | null {
    return transcripts.get(run.run)?.firstPrompt()?.trim() || null;
  }

  /** The session's facts after its state: client and settings, place, and when. */
  const facts = $derived(
    session
      ? [
          [harnessLabel(session.harness), ...Setup.words(session.settings)].join(' · '),
          Runs.place(session.workdir, scratch, offered),
          ...(session.activeAt ? [ago(session.activeAt, now)] : []),
        ]
      : [],
  );

  /** What the run's menu says of it: the details the header leaves out. */
  const details = $derived(
    session && current
      ? [
          `Session ${short(session.session)}`,
          `Process ${short(current.run)}`,
          current.workdir,
          ...(runState?.phase === 'ended'
            ? [Runs.exitDetail(runState.exit)].filter((d) => d !== null)
            : []),
        ]
      : [],
  );

  /** The run's other actions, least risky first; destructive ones last. */
  const menu = $derived.by((): ActionItem[] => {
    if (!controls || !session) return [];
    const items: ActionItem[] = [];
    const target = session.session;
    if (data.runsControl && controls.stop) {
      items.push({
        label: 'Stop its client',
        onselect: () => void act(() => client.toSession(target, { kind: 'stop' })),
      });
    }
    const recording = current?.recorded?.recording;
    if (recording)
      items.push({
        label: 'Open its calls',
        onselect: () => void goto(`/recordings/${recording}`),
      });
    if (data.runsControl) {
      const archived_ = session.shelf === 'archived';
      items.push({
        label: archived_ ? 'Unarchive' : 'Archive',
        onselect: () => void edit(archived_ ? 'unarchive' : 'archive'),
      });
      if (!Sessions.running(session)) {
        items.push({
          label: 'Delete…',
          danger: true,
          onselect: () => deleting({ kind: 'ask', session: target, from: 'header' }),
        });
      }
    }
    return items;
  });

  const compose = $derived(
    session ? Compose.session(current ? runState : null, client_?.midway ?? []) : null,
  );

  /** What the header says of the runs feed. */
  function feedNote(state: typeof feed): string {
    switch (state) {
      case 'opening':
      case 'open':
        return '';
      case 'reconnecting':
        return ' · updates reconnecting';
      case 'closed':
        return ' · updates stopped, reload to resume';
      default: {
        const never: never = state;
        return never;
      }
    }
  }
</script>

{#snippet toolbarScope()}
  <div class="scope-group scope-identity">
    <span class="scope-title">{scope.kind === 'project' ? 'Sessions' : 'Scratch'}</span>
    <span class="muted"
      >{liveCount} running{waiting ? ` · ${waiting} waiting for you` : ''}{data.runsControl
        ? feedNote(feed)
        : ''}</span
    >
  </div>
{/snippet}

{#snippet ask(waiting: Waiting, index: number)}
  {#if waiting.ask.kind === 'questions'}
    <QuestionCard
      title={waiting.ask.title}
      questions={waiting.ask.questions}
      {index}
      {busy}
      onreply={(request) => reply(waiting, request)}
      ondecide={(decision) => answer(waiting, decision)}
    />
  {:else}
    {@render permission(waiting, index)}
  {/if}
{/snippet}

{#snippet permission(waiting: Waiting, index: number)}
  {@const line = Runs.ask(waiting.ask)}
  <div class="approval" role="group" aria-label="Approval {index + 1}">
    <div class="approval-text">
      <strong><Icon name="shield" size={13} /> {line.action}</strong>
      {#if line.detail}<pre class="approval-detail mono">{line.detail}</pre>{/if}
      {#if waiting.ask.kind === 'command' && waiting.ask.cwd}<span class="muted mono"
          >in {waiting.ask.cwd}</span
        >{/if}
      {#if line.reason}<span class="muted">{line.reason}</span>{/if}
    </div>
    <div class="approval-actions">
      <button class="button primary small" disabled={busy} onclick={() => answer(waiting, 'accept')}
        >Allow</button
      >
      <button
        class="button small"
        disabled={busy}
        onclick={() => answer(waiting, 'accept_for_session')}
        title="Allow this, and the same kind of action for the rest of the session"
        >Allow for this run</button
      >
      <button
        class="button small"
        disabled={busy}
        onclick={() => answer(waiting, 'decline')}
        title="Refuse it; the turn goes on">Deny</button
      >
      <button
        class="button small"
        disabled={busy}
        onclick={() => answer(waiting, 'cancel')}
        title="Refuse it and end the turn">Deny and stop</button
      >
    </div>
  </div>
{/snippet}

<!-- A prompt on its way: drawn as a prompt, with how far it has got. -->
{#snippet outgoingView(item: Outgoing)}
  {@const progress = item.progress}
  <div class="entry prompt outgoing {progress.kind}">
    {#if item.images.length > 0}
      <div class="sent-images">
        {#each item.images as image (image.id)}
          {@const src = Gallery.url(item.session, image.id)}
          <button
            type="button"
            class="sent-image"
            title="{image.read.name} · {Gallery.describe(image.read)}"
            aria-label="View {image.read.name}"
            onclick={() => (viewing = { src, read: image.read })}
            ><img {src} alt={image.read.name} /></button
          >
        {/each}
      </div>
    {/if}
    {#if item.text}<Markdown text={item.text} />{/if}
    <div class="outgoing-state" role="status">
      {#if progress.kind === 'sending'}
        <span class="phase-dot tone-live"></span>
        {progress.starts === 'client' && session
          ? `Starting ${harnessLabel(session.harness)}…`
          : 'Sending…'}
      {:else if progress.kind === 'sent'}
        <Icon name="check" size={11} /> Sent
      {:else}
        <Icon name="alert" size={12} />
        <span class="outgoing-reason"
          >{progress.kind === 'refused'
            ? `Not sent: ${progress.reason}`
            : `It may not have been sent: ${progress.reason} Check the conversation before sending it again.`}</span
        >
        <button class="text-button" onclick={() => takeBack(item)}>Edit</button>
        <button
          class="text-button"
          disabled={sending}
          title={progress.kind === 'unknown' ? 'It may arrive twice' : null}
          onclick={() => resend(item)}>Send again</button
        >
      {/if}
    </div>
  </div>
{/snippet}

{#snippet entryView(entry: Entry)}
  {#if entry.kind === 'prompt'}
    <div class="entry prompt">
      {#if entry.images.length > 0}
        <div class="sent-images">
          {#each entry.images as image, index (index)}
            {@const id = gallery.find(image)}
            {#if id && chosen}
              {@const src = Gallery.url(chosen, id)}
              <button
                type="button"
                class="sent-image"
                title="{image.name} · {Gallery.describe(image)}"
                aria-label="View {image.name}"
                onclick={() => (viewing = { src, read: image })}
                ><img {src} alt={image.name} loading="lazy" /></button
              >
            {:else}
              <span class="sent-image named" title={Gallery.describe(image)}
                ><Icon name="image" size={12} />{image.name}</span
              >
            {/if}
          {/each}
        </div>
      {/if}
      {#if entry.text}<Markdown text={entry.text} />{/if}
    </div>
  {:else if entry.kind === 'message'}
    <div class="entry reply" class:streaming={!entry.done}>
      <Markdown text={entry.text} />
    </div>
  {:else if entry.kind === 'reasoning'}
    {#if !entry.done}<div class="entry tool muted">
        <Icon name="thought" size={12} /> Thinking…
      </div>{/if}
  {:else if entry.kind === 'command'}
    <CommandStep {entry} />
  {:else if entry.kind === 'file_change'}
    <div class="entry tool">
      <Icon name="pencil" size={12} />
      <span class="tool-target mono" title={entry.paths.join('\n') || undefined}
        >{entry.paths.map((path) => Runs.within(path, session?.workdir ?? null)).join(', ') ||
          'files'}</span
      >
      <span class="chip {entry.status === 'completed' ? 'success' : 'neutral'}"
        >{entry.status === 'in_progress' ? 'editing' : entry.status}</span
      >
    </div>
  {:else if entry.kind === 'tool'}
    <div class="entry tool">
      <Icon name="tool" size={12} /><span class="tool-name">{entry.name}</span>
      {#each entry.effects as effect, index (index)}<span class="tool-target mono"
          >{effect.target}</span
        >{/each}
      {#if !entry.done}<span class="chip live">running</span>{/if}
    </div>
  {:else if entry.kind === 'questions'}
    <div class="entry asked">
      {#each entry.questions as question (question.key)}
        {@const said = entry.answers[question.key]}
        <div class="asked-row">
          <Icon name="sessions" size={12} />
          <span class="asked-text">{question.text}</span>
          {#if said}<span class="asked-answer">{said}</span>
          {:else}<span class="chip {entry.status === 'in_progress' ? 'live' : 'neutral'}"
              >{entry.status === 'in_progress' ? 'waiting for you' : 'not answered'}</span
            >{/if}
        </div>
      {/each}
    </div>
  {:else if entry.kind === 'turn_end'}
    {#if entry.end.kind !== 'completed'}<p
        class="entry notice-entry {entry.end.kind === 'interrupted' ? 'warn' : 'danger'}"
      >
        Turn {entry.end.kind === 'interrupted'
          ? 'interrupted'
          : entry.end.kind === 'failed'
            ? `failed${entry.end.message ? `: ${entry.end.message}` : ''}`
            : `ended: ${entry.end.status}`}
      </p>{/if}
  {:else if entry.kind === 'notice'}
    <p class="entry notice-entry {entry.tone}">{entry.text}</p>
  {/if}
{/snippet}

{#snippet activity()}
  {@const steps = Runs.turns(transcript?.entries ?? []).map((turn) => ({
    prompt: turn[0]?.kind === 'prompt' ? turn[0].text : 'Turn',
    steps: turn.filter(
      (entry) => entry.kind === 'command' || entry.kind === 'file_change' || entry.kind === 'tool',
    ),
  }))}
  {#each steps.filter((turn) => turn.steps.length) as turn, index (index)}
    <div class="band">{turn.prompt.split('\n')[0]}</div>
    {#each turn.steps as entry, step (step)}{@render entryView(entry)}{/each}
  {:else}
    <p class="quiet-empty">No steps yet.</p>
  {/each}
{/snippet}

{#snippet files()}
  {@const written = [
    ...new Set(
      (transcript?.entries ?? []).flatMap((entry) =>
        entry.kind === 'file_change' ? entry.paths : [],
      ),
    ),
  ]}
  <div class="band">Written <span class="count">{written.length}</span></div>
  {#each written as path (path)}<div class="entry tool mono" title={path}>
      <Icon name="pencil" size={11} />
      {Runs.within(path, session?.workdir ?? null)}
    </div>{:else}<p class="quiet-empty">Nothing written yet.</p>{/each}
{/snippet}

{#snippet callList()}
  {#if !current?.recorded?.recording}
    <p class="quiet-empty">No calls recorded yet.</p>
  {:else if !calls}
    <p class="quiet-empty">Reading calls…</p>
  {:else}
    <table class="calls">
      <thead
        ><tr><th>Started</th><th>Model</th><th>Ended</th><th>Time</th><th>Tokens</th></tr></thead
      >
      <tbody>
        {#each calls.calls as call (call.id)}
          {@const badge = statusBadge(callStatus(call))}
          {@const tokens = tokenSummary(call.usage)}
          <tr>
            <td class="mono muted">{clock(call.started_at)}</td>
            <td class="mono">{call.model ?? '—'}</td>
            <td><span class="chip {badge.tone}">{badge.label}</span></td>
            <td class="mono muted"
              >{call.ended_at
                ? `${Math.round((call.ended_at - call.started_at) / 100) / 10} s`
                : '…'}</td
            >
            <td class="mono muted" title={tokens?.title}
              >{tokens ? `${tokens.input} in · ${tokens.output} out` : '—'}</td
            >
          </tr>
        {/each}
      </tbody>
    </table>
    <a class="foot-link" href="/recordings/{current.recorded.recording}"
      >Open its recording <Icon name="arrow" size={11} /></a
    >
  {/if}
{/snippet}

{#snippet turnsOutline()}
  <!-- A conversation on its way: turns ruled off as they will be, with
       bars where each prompt and reply go. -->
  {#each OUTLINE_TURNS as turn, index (index)}
    <section class="turn" aria-hidden="true">
      <div class="entry prompt outline"><Skeleton width={turn.prompt} /></div>
      <div class="entry reply outline">
        {#each turn.reply as width, line (line)}<Skeleton {width} />{/each}
      </div>
    </section>
  {/each}
{/snippet}

{#snippet sessionOutline()}
  <!-- A session on its way, before the lists say what it is: its header,
       its tabs, and its conversation's outline. -->
  <header class="run-head" aria-busy="true">
    <div class="run-heading">
      <h2 class="run-title"><Skeleton width="42%" /></h2>
      <p class="run-facts"><Skeleton width="16em" /></p>
    </div>
  </header>
  <div class="tabs" role="tablist" aria-label="What the run shows">
    {#each tabs as [name, label] (name)}
      <button
        role="tab"
        aria-selected={name === 'conversation'}
        class:chosen={name === 'conversation'}
        disabled>{label}</button
      >
    {/each}
  </div>
  <div class="conversation" aria-busy="true">{@render turnsOutline()}</div>
{/snippet}

<svelte:window onkeydown={shortcut} />

<Toolbar scope={toolbarScope} />
<div class="runs-page" data-pane={pane}>
  <aside class="runs-sidebar" aria-label="Runs">
    {#if unavailable}<p class="sidebar-note danger">{unavailable}</p>{/if}
    <SessionList
      {folders}
      {listing}
      selected={chosen}
      {now}
      {title}
      place={(workdir) => Runs.place(workdir, scratch, offered)}
      scratch={scope.kind === 'project' ? null : scratch}
      {shelf}
      archivedCount={archivedSessions.length}
      {busy}
      deletable={data.runsControl}
      {deletion}
      ondelete={deleting}
      onshelf={(next) => {
        shelf = next;
        select(null);
      }}
      onselect={(next) => select(next)}
      oncreate={(dir) => void newDraft(dir)}
      onnew={() => {
        starting = true;
        void newDraft();
      }}
    />
  </aside>

  <section class="run-main" aria-label="Run">
    <button
      class="pane-back"
      type="button"
      onclick={() => {
        starting = false;
        select(null);
      }}><span class="pane-back-icon"><Icon name="chevron" size={14} /></span>Sessions</button
    >
    {#if !session || !controls}
      {#if data.runsControl && chosen && !session && listing === 'pending'}
        {@render sessionOutline()}
      {:else if data.runsControl}
        <div class="empty-state">
          <h2>{chosen ? 'This session is not listed' : 'No session open'}</h2>
          <p>
            Open one from the list, or start a new one: write its first prompt, and its client
            starts with it.
          </p>
          <button
            class="button primary"
            type="button"
            disabled={busy}
            onclick={() => void newDraft()}
            ><Icon name="spark" size={12} />New session <kbd>n</kbd></button
          >
        </div>
        {#if notice}<div class="notice danger" role="alert">
            <Icon name="alert" size={14} /><span>{notice}</span>
          </div>{/if}
      {:else}
        <div class="empty-state">
          <div class="empty-icon"><Icon name="lock" size={26} /></div>
          <h2>This UI is read-only</h2>
          <p>
            To start and drive agent runs here, give the UI server the daemon’s control token file
            with <code>REN_CONTROL_TOKEN_FILE</code> (and, when published with Tailscale Serve, the
            allowed logins in <code>REN_CONTROL_LOGINS</code>), and add folders to the daemon’s
            <code>[runs] roots</code>.
          </p>
        </div>
      {/if}
    {:else}
      <header class="run-head">
        <div class="run-heading">
          <h2 class="run-title" title={(current && firstPrompt(current)) ?? undefined}>
            {title({ run: current?.run ?? null, asked: session.asked }) ??
              `New ${harnessLabel(session.harness)} session`}
          </h2>
          <p class="run-facts">
            {#if headerMark}<span class="state tone-{headerMark.tone}"
                ><span class="dot"></span>{headerMark.label}</span
              >{/if}
            {#each facts as fact, index (fact)}{#if headerMark || index > 0}<span
                  class="sep"
                  aria-hidden="true">·</span
                >{/if}<span>{fact}</span>{/each}
            {#if spentLine}<span class="sep" aria-hidden="true">·</span><span
                title={spentLine.title}>{spentLine.text}</span
              >{/if}
          </p>
        </div>
        {#if asking}
          <span class="run-actions confirming" role="alert" aria-busy={asking.phase === 'deleting'}>
            <span class="confirm-text"
              >{asking.phase === 'deleting'
                ? 'Deleting this session and its recorded calls…'
                : asking.phase === 'failed'
                  ? asking.reason
                  : 'Delete this session and its recorded calls for good?'}</span
            >
            <button
              class="button small danger confirm"
              disabled={asking.phase === 'deleting'}
              onclick={() => deleting({ kind: 'confirm' })}
              >{asking.phase === 'failed' ? 'Retry' : 'Delete'}</button
            >
            <button
              class="button small"
              disabled={asking.phase === 'deleting'}
              onclick={() => deleting({ kind: 'cancel' })}>Cancel</button
            >
          </span>
        {:else}
          <span class="run-actions">
            {#if data.runsControl && controls.interrupt}<button
                class="button small"
                disabled={busy}
                onclick={() =>
                  chosen && void act(() => client.toSession(chosen, { kind: 'interrupt' }))}
                >Interrupt</button
              >{/if}
            <ActionMenu label="Session actions" items={menu} {details} disabled={busy} />
          </span>
        {/if}
      </header>

      <div class="tabs" role="tablist" aria-label="What the run shows">
        {#each tabs as [name, label] (name)}
          <button
            role="tab"
            aria-selected={tab === name}
            class:chosen={tab === name}
            onclick={() => (tab = name)}>{label}</button
          >
        {/each}
      </div>

      {#if tab === 'conversation'}
        <!-- Scrolled from its end, so it opens on the newest entry from the
             first paint, before the page's scripts run. -->
        <div
          class="conversation from-end"
          bind:this={conversation}
          onscroll={onScroll}
          aria-busy={conversationPending}
        >
          <div class="conversation-body">
            {#if conversationPending}
              {@render turnsOutline()}
            {:else}
              {#if readError}<p class="entry notice-entry danger">{readError}</p>{/if}
              {#if transcript?.earlier}<p class="quiet-empty">
                  Earlier turns are not shown: the conversation is longer than one read.
                </p>{/if}
              <!-- The whole conversation, every turn open, as one scroll. -->
              {#each Runs.turns(transcript?.entries ?? []) as turn, index (index)}
                <section class="turn">
                  {#each Steps.blocks(turn, session?.workdir ?? null) as block, at (at)}
                    {#if block.kind === 'steps'}
                      <StepGroup
                        steps={block.steps}
                        state={block.state}
                        open={folds.get(block.key) ?? false}
                        ontoggle={(open) => folds.set(block.key, open)}
                      />
                    {:else}
                      {@render entryView(block.entry)}
                    {/if}
                  {/each}
                </section>
              {/each}
            {/if}
            {#each outgoing as item (item.key)}{@render outgoingView(item)}{/each}
            {#if runState?.phase === 'working' && !controls.approvals.length}<div
                class="entry tool muted working"
              >
                <span class="phase-dot tone-live"></span> Working…
              </div>{/if}
            {#if transcript?.read && !transcript.entries.length && runState?.phase === 'idle'}
              <p class="quiet-empty">The conversation is open. Say what to do.</p>
            {:else if !current && outgoing.length === 0}
              <p class="quiet-empty">
                Write the first prompt: {harnessLabel(session.harness)} starts with it, in {Runs.place(
                  session.workdir,
                  scratch,
                  offered,
                )}.
              </p>
            {/if}
          </div>
        </div>
      {:else if tab === 'activity'}
        <div class="conversation">
          {@render activity()}
        </div>
      {:else if tab === 'files'}
        <div class="conversation">
          {@render files()}
        </div>
      {:else}
        <div class="conversation">
          {@render callList()}
        </div>
      {/if}

      {#if controls.approvals.length}
        <div class="approvals" aria-label="Waiting for your answer">
          {#each controls.approvals as waiting, index (waiting.approval)}{@render ask(
              waiting,
              index,
            )}{/each}
        </div>
      {/if}
      {#if notice}<div class="notice danger" role="alert">
          <Icon name="alert" size={14} /><span>{notice}</span>
        </div>{/if}
      {#if data.runsControl && compose && !(compose.kind === 'closed' && runState?.phase === 'ended')}
        {#if compose.kind === 'during' && compose.queued}
          <div class="queued" role="status">
            <span class="queued-label">After this turn</span>
            <span class="queued-text">{compose.queued}</span>
            <button
              class="text-button"
              disabled={busy}
              onclick={() => editQueued(compose.queued ?? '')}>Edit</button
            >
            <button
              class="text-button"
              disabled={busy}
              onclick={() =>
                chosen && void act(() => client.toSession(chosen, { kind: 'unqueue' }))}
              >Remove</button
            >
          </div>
        {/if}
        {#if client_ && signedOut && Runs.drivable(session.harness)}
          {@const harness = Runs.drivable(session.harness)}
          <div class="signed-out" role="status">
            <span
              ><Icon name="lock" size={12} />
              {harnessLabel(session.harness)} is signed out on the daemon’s host, so nothing can be sent.
              Signed in some other way? Check again.</span
            >
            {#if harness}
              <button
                class="button small"
                type="button"
                disabled={checking}
                onclick={() => void checkSignIn(harness)}
                >{checking ? 'Checking…' : 'Check again'}</button
              >
            {/if}
          </div>
        {/if}
        <form
          class="composer"
          class:locked={signedOut}
          class:dropping
          onsubmit={(event) => {
            event.preventDefault();
            void send('queue');
          }}
          ondragover={(event) => {
            if (!imagesTaken || compose.kind === 'closed') return;
            if ([...(event.dataTransfer?.types ?? [])].includes('Files')) {
              event.preventDefault();
              dropping = true;
            }
          }}
          ondragleave={() => (dropping = false)}
          ondrop={(event) => {
            dropping = false;
            const files = imagesOf(event.dataTransfer);
            if (files.length === 0 || !imagesTaken || compose.kind === 'closed') return;
            event.preventDefault();
            void addImages(files);
          }}
        >
          {#if tray.chips.length > 0}
            <ul class="tray" aria-label="Images to send">
              {#each tray.chips as chip (chip.key)}
                <li class="tray-chip {chip.state}">
                  {#if chip.state === 'ready' && chosen}
                    {@const src = Gallery.url(chosen, chip.id)}
                    {@const read = chip.read}
                    <button
                      type="button"
                      class="tray-view"
                      aria-label="View {read.name}"
                      onclick={() => (viewing = { src, read })}><img {src} alt="" /></button
                    >
                    <span class="tray-name" title={Gallery.describe(chip.read)}
                      >{chip.read.name}</span
                    >
                  {:else if chip.state === 'uploading'}
                    <Icon name="image" size={12} />
                    <span class="tray-name">{chip.name}</span>
                    <span class="muted">Uploading…</span>
                  {:else if chip.state === 'refused'}
                    <Icon name="alert" size={12} />
                    <span class="tray-name">{chip.name}</span>
                    <span class="refused-reason">{chip.reason}</span>
                  {/if}
                  {#if chip.state !== 'uploading'}
                    <button
                      type="button"
                      class="tray-remove"
                      aria-label="Remove {chip.state === 'ready' ? chip.read.name : chip.name}"
                      onclick={() => void removeChip(chip)}><Icon name="close" size={10} /></button
                    >
                  {/if}
                </li>
              {/each}
            </ul>
          {/if}
          {#if offering}
            <ComposerMenu
              id={menuId}
              heading={offering.kind === 'file'
                ? 'Files in the run’s folder'
                : offering.sigil === '$'
                  ? `${harnessLabel(session.harness)} skills`
                  : `${harnessLabel(session.harness)} commands`}
              {entries}
              {active}
              note={offering.kind === 'file'
                ? (filesNote ?? (found ? null : 'Searching…'))
                : (commandsNote ??
                  ((commandLists.get(commandsKey ?? '')?.length ?? 0) > 0
                    ? 'No command matches.'
                    : null))}
              onpick={pick}
              onpoint={(index) => (active = index)}
            />
          {/if}
          <textarea
            bind:this={composer}
            value={draft}
            oninput={(event) => {
              recalling = null;
              dismissed = null;
              write(event.currentTarget.value);
              trackCaret(event);
            }}
            onkeydown={composerKey}
            onpaste={(event) => {
              const files = imagesOf(event.clipboardData);
              if (files.length === 0 || !imagesTaken) return;
              event.preventDefault();
              void addImages(files);
            }}
            onkeyup={trackCaret}
            onclick={trackCaret}
            onfocus={trackCaret}
            role="combobox"
            aria-autocomplete="list"
            aria-expanded={offering !== null}
            aria-controls={offering ? menuId : undefined}
            aria-activedescendant={offering && entries.length > 0
              ? `${menuId}-${active}`
              : undefined}
            rows="3"
            disabled={compose.kind === 'closed'}
            placeholder={signedOut
              ? `${harnessLabel(session.harness)} is signed out. Write your prompt now; it sends once ${harnessLabel(session.harness)} is signed in.`
              : compose.kind === 'start'
                ? `Ask ${harnessLabel(session.harness)} to do something. Enter sends, ↑ recalls an earlier prompt.`
                : compose.kind === 'during'
                  ? compose.steer
                    ? `Enter queues for after this turn; ${navigatorKey}+Enter sends it into the turn now.`
                    : `Enter queues for after this turn; ${harnessLabel(session.harness)} reads it then.`
                  : compose.kind === 'continue'
                    ? `Write to ${harnessLabel(session.harness)} to go on.`
                    : compose.reason}
            aria-label="Prompt"></textarea>
          <div class="composer-bar">
            {#if session.phase === 'draft' && session.shelf === 'open'}
              <FolderPicker
                workdir={session.workdir}
                {offered}
                {scratch}
                runs={folders.flatMap((folder) =>
                  folder.sessions.map((entry) => ({
                    workdir: entry.workdir,
                    started_at: entry.runs[0]?.started_at ?? null,
                  })),
                )}
                disabled={busy ? 'Waiting for the last change' : null}
                onchange={moveDraft}
              />
            {/if}
            {#if Runs.drivable(session.harness) && client_}
              <RunSettingsBar
                harness={Runs.drivable(session.harness) ?? 'codex'}
                settings={session.settings}
                midway={Sessions.running(session) ? client_.midway : MIDWAY}
                approvals={client_.approvals}
                listModels={(harness) => client.models(harness)}
                {busy}
                onchange={(settings) => {
                  const harness = Runs.drivable(session.harness);
                  if (harness) remember(Memory.set(remembered, harness, settings));
                  if (chosen)
                    void act(async () => {
                      await client.toSession(chosen, { kind: 'settings', ...settings });
                      await Promise.all([readSessions(), readLive()]);
                    });
                }}
                choice={session.phase === 'draft'
                  ? {
                      clients: DRIVABLE.filter((harness) =>
                        clients.some(
                          (entry) => entry.harness === harness && entry.status === 'ready',
                        ),
                      ),
                      hint: (harness) => {
                        const entry = clients.find((offered) => offered.harness === harness);
                        return entry ? SignIns.hint(entry.sign_in) : null;
                      },
                      onclient: (harness) => {
                        remember(Memory.chose(remembered, harness));
                        if (chosen)
                          void act(async () => {
                            await client.toSession(chosen, { kind: 'client', harness });
                            await Promise.all([readSessions(), readLive()]);
                          });
                      },
                    }
                  : null}
              />
            {/if}
            <span class="composer-actions">
              {#if imagesTaken && compose.kind !== 'closed'}
                <input
                  bind:this={picker}
                  class="picker"
                  type="file"
                  accept="image/png,image/jpeg,image/gif,image/webp"
                  multiple
                  tabindex="-1"
                  onchange={(event) => {
                    const files = [...(event.currentTarget.files ?? [])];
                    event.currentTarget.value = '';
                    void addImages(files);
                  }}
                />
                <button
                  type="button"
                  class="button"
                  title="Attach images. They are kept and sent as they are, metadata included."
                  aria-label="Attach images"
                  onclick={() => picker?.click()}><Icon name="image" size={12} /></button
                >
              {/if}
              {#if compose.kind === 'during' && compose.steer}<button
                  type="button"
                  class="button"
                  disabled={busy ||
                    sending ||
                    (!draft.trim() && tray.ids().length === 0) ||
                    tray.uploading() ||
                    signedOut}
                  title="Add it to the turn that is running"
                  onclick={() => void send('steer')}>Send now</button
                >{/if}
              {#if signedOut && Runs.drivable(session.harness) && (client_?.sign_in_methods.length ?? 0) > 0}
                <!-- Signed out: the one action that leads on is signing in. -->
                <button
                  class="button primary"
                  type="button"
                  onclick={() => (signingIn = Runs.drivable(session.harness))}
                  ><Icon name="lock" size={12} />Sign in to {harnessLabel(session.harness)}</button
                >
              {:else}
                <button
                  class="button primary"
                  disabled={busy ||
                    sending ||
                    (!draft.trim() && tray.ids().length === 0) ||
                    tray.uploading() ||
                    compose.kind === 'closed' ||
                    signedOut}
                  title={signedOut
                    ? `Sign ${harnessLabel(session.harness)} in to send`
                    : sending
                      ? 'The last prompt is still on its way'
                      : null}
                  >{compose.kind === 'during'
                    ? 'Queue'
                    : compose.kind === 'continue'
                      ? 'Continue'
                      : 'Send'}</button
                >
              {/if}
            </span>
          </div>
        </form>
      {/if}
    {/if}
  </section>
</div>
{#if viewing}
  <ImageViewer
    src={viewing.src}
    name={viewing.read.name}
    detail={Gallery.describe(viewing.read)}
    onclose={() => (viewing = null)}
  />
{/if}
{#if signingIn}
  {@const harness = signingIn}
  <SignInSheet
    {harness}
    client={harnessLabel(harness)}
    methods={clients.find((entry) => entry.harness === harness)?.sign_in_methods ?? []}
    start={(method) => client.startSignIn(harness, method)}
    answer={(answer) => client.signInAnswer(harness, answer)}
    wait={() => client.waitSignIn(harness)}
    cancel={() => client.cancelSignIn(harness)}
    onclose={() => {
      signingIn = null;
      void readClients();
    }}
  />
{/if}

<style>
  .runs-page {
    flex: 1;
    min-height: 0;
    display: grid;
    grid-template-columns: minmax(220px, 280px) 1fr;
    background: var(--canvas);
  }
  .runs-sidebar {
    display: flex;
    flex-direction: column;
    min-height: 0;
    border-right: 1px solid var(--line);
    background: var(--sidebar);
  }
  .sidebar-note {
    margin: 0;
    padding: 12px;
    font-size: 12px;
    color: var(--muted);
  }
  .sidebar-note.danger {
    color: var(--error);
    border-bottom: 1px solid var(--line);
  }
  .phase-dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    flex-shrink: 0;
    background: var(--muted);
  }
  .tone-live.phase-dot {
    background: var(--live);
  }
  .tone-warn.phase-dot {
    background: var(--warning);
  }
  .tone-ok.phase-dot {
    background: var(--success);
  }
  .tone-danger.phase-dot {
    background: var(--error);
  }
  .tabs {
    display: flex;
    border-bottom: 1px solid var(--line);
    padding: 0 12px;
  }
  .tabs button {
    padding: 8px 12px;
    border: 0;
    border-bottom: 2px solid transparent;
    background: none;
    color: var(--muted);
    font: inherit;
    font-size: 12px;
    cursor: pointer;
  }
  .tabs button.chosen {
    color: var(--ink);
    border-bottom-color: var(--accent, var(--ink));
  }
  .lost {
    margin: 0;
    padding: 10px 16px;
    border-bottom: 1px solid var(--line);
    font-size: 12px;
  }
  /* A heading row: what the rows under it belong to. */
  .band {
    padding: 7px 16px;
    border-bottom: 1px solid var(--line);
    background: var(--sidebar);
    color: var(--muted);
    font-size: 11px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .band .count {
    margin-left: 4px;
    color: var(--ink);
  }
  table.calls {
    font-size: 12px;
  }
  table.calls th:first-child,
  table.calls td:first-child {
    padding-left: 16px;
  }
  table.calls td {
    vertical-align: middle;
  }
  .foot-link {
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 8px 16px;
    border-bottom: 1px solid var(--line);
    font-size: 12px;
  }
  .run-main :global(.quiet-empty) {
    margin: 0;
    padding: 10px 16px;
    border-bottom: 1px solid var(--line);
    font-size: 12px;
  }
  .run-main > .empty-state {
    margin: auto;
  }
  .run-main {
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
    background: var(--surface);
  }
  .run-head {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 10px 16px;
    border-bottom: 1px solid var(--line);
    font-size: 12px;
  }
  .run-heading {
    flex: 1;
    min-width: 0;
  }
  .run-title {
    margin: 0;
    overflow: hidden;
    font-size: 14px;
    font-weight: 600;
    white-space: nowrap;
    text-overflow: ellipsis;
  }
  .run-facts {
    display: flex;
    align-items: center;
    gap: 6px;
    margin: 3px 0 0;
    overflow: hidden;
    color: var(--muted);
    white-space: nowrap;
  }
  .run-facts > span {
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .run-facts .sep {
    flex: none;
  }
  .state {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    flex: none;
    color: var(--ink);
  }
  .state .dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--muted);
  }
  .state.tone-live .dot {
    background: var(--live);
  }
  .state.tone-ok .dot {
    background: var(--success);
  }
  .state.tone-warn .dot {
    background: var(--warning);
  }
  .state.tone-danger .dot {
    background: var(--error);
  }
  .confirm-text {
    color: var(--error);
  }
  .run-actions {
    display: flex;
    flex: none;
    align-items: center;
    gap: 6px;
  }
  .conversation {
    flex: 1;
    min-height: 0;
    overflow: auto;
  }
  /* Laid out from its end, so the browser opens it scrolled to the newest
     entry, and keeps it there as entries are added while it is. */
  .conversation.from-end {
    display: flex;
    flex-direction: column-reverse;
  }
  /* A conversation shorter than the pane still starts at its top. */
  .conversation-body {
    margin-bottom: auto;
  }
  /* A turn: its prompt, steps, and reply as rows, ruled off from the next. */
  .turn {
    border-bottom: 1px solid var(--line);
  }
  .entry.asked {
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: 6px 16px;
    border-bottom: 1px solid var(--line-subtle);
    font-size: 12px;
  }
  .asked-row {
    display: flex;
    align-items: baseline;
    gap: 8px;
    min-width: 0;
  }
  .asked-text {
    color: var(--muted);
  }
  .asked-answer {
    font-weight: 600;
    color: var(--ink);
    overflow-wrap: anywhere;
  }
  .entry {
    padding: 0 16px;
    font-size: 13px;
  }
  /* A prompt reads as Markdown, like a reply, with its line breaks kept
     as written. */
  .entry.prompt :global(.markdown) {
    padding: 10px 0;
  }
  .entry.prompt :global(.markdown p) {
    white-space: pre-wrap;
  }
  /* A prompt's images, as thumbnails that touch, opened full size on click. */
  .sent-images {
    display: flex;
    flex-wrap: wrap;
    margin-top: 10px;
    border: 1px solid var(--line);
    width: fit-content;
  }
  .sent-image {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 12px;
  }
  button.sent-image,
  .tray-view {
    padding: 0;
    border: 0;
    background: transparent;
    cursor: zoom-in;
  }
  .sent-image + .sent-image {
    border-left: 1px solid var(--line);
  }
  .sent-image img {
    display: block;
    max-width: 240px;
    max-height: 160px;
  }
  .sent-image.named {
    padding: 6px 8px;
  }
  /* An entry's outline: its bars stacked as its lines will be. */
  .entry.outline {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 7px;
    padding-block: 12px;
  }
  /* A prompt is drawn on the inset surface the bars use: its bar is ruled. */
  .entry.prompt.outline :global(.skeleton) {
    background: var(--line);
  }
  .tabs button:disabled {
    cursor: default;
  }
  .entry.prompt {
    background: var(--surface-inset);
    box-shadow: inset 2px 0 var(--role-user);
    border-bottom: 1px solid var(--line);
  }
  /* A prompt on its way: its words as sent, dimmed until the client has them. */
  .entry.prompt.outgoing.sending :global(.markdown),
  .entry.prompt.outgoing.sent :global(.markdown) {
    color: var(--muted);
  }
  .entry.prompt.outgoing.refused,
  .entry.prompt.outgoing.unknown {
    box-shadow: inset 2px 0 var(--error);
  }
  .outgoing-state {
    display: flex;
    align-items: center;
    gap: 6px;
    padding-bottom: 8px;
    color: var(--muted);
    font-size: 12px;
  }
  .entry.prompt.outgoing.refused .outgoing-state,
  .entry.prompt.outgoing.unknown .outgoing-state {
    color: var(--error);
  }
  .outgoing-reason {
    min-width: 0;
  }
  .entry.reply {
    padding-top: 10px;
    padding-bottom: 10px;
    box-shadow: inset 2px 0 var(--role-assistant);
  }
  .entry.reply.streaming :global(.markdown > :last-child::after) {
    content: '▍';
    color: var(--muted);
  }
  .entry.tool {
    display: flex;
    align-items: center;
    gap: 8px;
    padding-top: 6px;
    padding-bottom: 6px;
    border-bottom: 1px solid var(--line-subtle);
    font-size: 12px;
    color: var(--ink);
  }
  .entry.tool.muted {
    color: var(--muted);
  }
  .tool-name {
    color: var(--role-tool);
  }
  /* A step's state sits at the row's end, however long its target. */
  .entry.tool > .chip {
    flex: none;
    margin-left: auto;
  }
  .tool-target {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .notice-entry {
    margin: 0;
    padding-top: 8px;
    padding-bottom: 8px;
    border-bottom: 1px solid var(--line-subtle);
    font-size: 12px;
  }
  .notice-entry.warn {
    color: var(--warning);
  }
  .notice-entry.danger {
    color: var(--error);
  }
  .working {
    color: var(--muted);
  }
  .approvals {
    border-top: 1px solid var(--warning-border);
    background: var(--warning-soft);
  }
  .approval {
    display: flex;
    align-items: flex-start;
    gap: 12px;
    padding: 10px 16px;
    font-size: 12px;
  }
  .approval + .approval {
    border-top: 1px solid var(--warning-border);
  }
  .approval-text {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 0;
    flex: 1;
  }
  .approval-text strong {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    color: var(--warning);
  }
  .approval-detail {
    margin: 0;
    padding: 6px 8px;
    background: var(--surface);
    border: 1px solid var(--warning-border);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    max-height: 160px;
    overflow: auto;
  }
  .approval-actions {
    display: flex;
    gap: 6px;
    flex-wrap: wrap;
    justify-content: flex-end;
  }
  /* The prompt fills its strip; only the Send control sits inside it. */
  .composer {
    position: relative;
    display: flex;
    flex-direction: column;
    border-top: 1px solid var(--line);
    background: var(--surface);
  }
  .composer:focus-within {
    box-shadow: inset 2px 0 var(--accent);
  }
  /* Signed out: the composer's edge turns to the warning above it. */
  .composer.locked,
  .composer.locked:focus-within {
    box-shadow: inset 2px 0 var(--warning);
  }
  .composer textarea {
    min-width: 0;
    resize: vertical;
    min-height: 64px;
    max-height: 40vh;
    padding: 12px 16px;
    border: 0;
    outline: none;
    background: transparent;
    color: var(--ink);
    font: inherit;
    font-size: 13px;
  }
  /* Something dragged over the composer: its edge says it takes the drop. */
  .composer.dropping {
    box-shadow: inset 2px 0 var(--accent);
    background: var(--surface-inset);
  }
  /* The images about to go: square chips that touch, ruled off by 1px. */
  .tray {
    display: flex;
    flex-wrap: wrap;
    margin: 0;
    padding: 0;
    list-style: none;
    border-bottom: 1px solid var(--line);
  }
  .tray-chip {
    display: flex;
    align-items: center;
    gap: 6px;
    max-width: 280px;
    height: 40px;
    padding: 0 6px 0 0;
    border-right: 1px solid var(--line);
    font-size: 12px;
  }
  .tray-view {
    display: block;
  }
  .tray-chip img {
    width: 40px;
    height: 40px;
    object-fit: cover;
    border-right: 1px solid var(--line);
  }
  .tray-chip.uploading,
  .tray-chip.refused {
    padding-left: 8px;
  }
  .tray-name {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .refused-reason {
    color: var(--error);
    white-space: nowrap;
  }
  .tray-remove {
    display: inline-flex;
    padding: 3px;
    border: 0;
    border-radius: 2px;
    background: transparent;
    color: inherit;
    cursor: pointer;
  }
  .tray-remove:hover {
    background: var(--surface-inset);
  }
  .picker {
    display: none;
  }
  .composer-bar {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 0 12px 10px 16px;
  }
  .composer-actions {
    display: flex;
    gap: 6px;
    margin-left: auto;
  }
  /* The session's client is signed out: said above the composer. */
  .signed-out {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 6px 16px;
    border-top: 1px solid var(--warning-border);
    background: var(--warning-soft);
    color: var(--warning);
    font-size: 12px;
  }
  .signed-out span {
    flex: 1;
    display: flex;
    align-items: center;
    gap: 6px;
  }
  /* The prompt queued for after the turn, ruled off above the composer. */
  .queued {
    display: flex;
    align-items: baseline;
    gap: 10px;
    padding: 8px 16px;
    border-top: 1px solid var(--line);
    background: var(--surface);
    font-size: 12px;
  }
  .queued-label {
    flex: none;
    color: var(--muted);
  }
  .queued-text {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }
  /* Back to the list: a phone's way out of a session. */
  .pane-back {
    display: none;
    align-items: center;
    gap: 4px;
    padding: 9px 12px;
    border: 0;
    border-bottom: 1px solid var(--line);
    background: var(--sidebar);
    color: var(--muted);
    font: inherit;
    font-size: 12px;
    text-align: left;
    cursor: pointer;
  }
  .pane-back:hover {
    color: var(--ink);
  }
  .pane-back-icon {
    display: inline-flex;
    transform: rotate(180deg);
  }
  @media (max-width: 760px) {
    .runs-page {
      grid-template-columns: 1fr;
      grid-template-rows: 1fr;
    }
    .runs-sidebar {
      border-right: 0;
    }
    .runs-page[data-pane='list'] .run-main,
    .runs-page:not([data-pane='list']) .runs-sidebar {
      display: none;
    }
    .pane-back {
      display: flex;
    }
    .approval {
      flex-direction: column;
    }
  }
</style>
