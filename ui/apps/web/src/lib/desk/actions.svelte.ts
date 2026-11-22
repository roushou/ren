// What the desk does to its sessions, one action at a time: start a new
// one (a draft) or move it, change its client and settings (remembered for
// the next), answer what its client asks, stop it, archive and unarchive
// it, and delete it, asked and confirmed first.
import { tick, untrack } from 'svelte';
import {
  Deletions,
  Memory,
  RunCommandError,
  Runs,
  RunsClient,
  Sessions,
  type ChosenSettings,
  type Decision,
  type Deletion,
  type DeletionEvent,
  type DeskSession,
  type Drivable,
  type HistoryChange,
  type Remembered,
  type SessionId,
  type SessionScope,
  type Waiting,
} from '#lib/runs.ts';
import type { ReplyRequest } from '#lib/wire/ReplyRequest.ts';
import type { Acting } from './acting.svelte.ts';
import type { DeskFeed } from './feed.svelte.ts';
import type { DeskData } from './load.ts';

export interface SessionActionsOptions {
  client: RunsClient;
  feed: DeskFeed;
  acting: Acting;
  /** The desk's data, as its page has it now. */
  data: () => DeskData;
  /** Which sessions the desk holds. */
  scope: () => SessionScope;
  /** The session the desk shows, by its id, and as listed. */
  chosen: () => SessionId | null;
  session: () => DeskSession | null;
  /** Show another session, or none. */
  select: (next: SessionId | null) => void;
  /** Put the keyboard in the composer. */
  focus: () => void;
}

export class SessionActions {
  readonly #client: RunsClient;
  readonly #feed: DeskFeed;
  readonly #acting: Acting;
  readonly #scope: () => SessionScope;
  readonly #chosen: () => SessionId | null;
  readonly #session: () => DeskSession | null;
  readonly #select: (next: SessionId | null) => void;
  readonly #focus: () => void;

  /** A session's delete: asked, confirmed, and sent, one at a time. */
  deletion = $state.raw<Deletion>(Deletions.idle);
  /** The client and settings last chosen here: a new session starts with them. */
  remembered = $state.raw<Remembered>(Memory.parse(null));
  /** The delete asked in the header, of the session it shows. */
  readonly asking = $derived.by(() => {
    const chosen = this.#chosen();
    return chosen ? Deletions.of(this.deletion, chosen, 'header') : null;
  });

  constructor(options: SessionActionsOptions) {
    this.#client = options.client;
    this.#feed = options.feed;
    this.#acting = options.acting;
    this.#scope = options.scope;
    this.#chosen = options.chosen;
    this.#session = options.session;
    this.#select = options.select;
    this.#focus = options.focus;
    this.remembered = Memory.parse(untrack(options.data).remembered);

    // A delete is done when its session has left both shelves.
    $effect(() => {
      const deletion = this.deletion;
      if (deletion.phase === 'idle' || deletion.phase === 'confirming') return;
      const target = deletion.session;
      if (this.#feed.lists(target)) return;
      untrack(() => {
        this.deleting({ kind: 'gone' });
        if (this.#chosen() === target) this.#select(null);
      });
    });
  }

  /** The delete after `event`; an event that does not apply is a fault here. */
  deleting = (event: DeletionEvent) => {
    const next = Deletions.step(this.deletion, event);
    if (Deletions.illegal(next)) {
      console.warn(`A session delete cannot ${next.event} while ${next.state}.`);
      return;
    }
    this.deletion = next;
    if (event.kind === 'confirm' && next.phase === 'deleting') void this.#remove(next.session);
  };

  /** A question asked in the header is about the session it showed: another
   * session shown drops it. */
  dropAsked() {
    const deletion = this.deletion;
    if (
      (deletion.phase === 'confirming' || deletion.phase === 'failed') &&
      deletion.from === 'header'
    )
      this.deleting({ kind: 'cancel' });
  }

  /**
   * Delete a session: it goes with its ended runs and their recorded calls,
   * even one whose client never ran. It is done when the lists no longer
   * hold it; a delete the daemon took but has not applied by the last read
   * says so and can be sent again.
   */
  async #remove(target: SessionId) {
    try {
      await this.#client.toSession(target, { kind: 'delete' });
      const gone = await this.#feed.until(() => !this.#feed.lists(target));
      const deletion = this.deletion;
      if (!gone && deletion.phase === 'deleting' && deletion.session === target) {
        this.deleting({
          kind: 'refused',
          reason: 'Asked, but still listed: the daemon may apply it shortly.',
        });
      }
    } catch (error) {
      this.deleting({ kind: 'refused', reason: RunCommandError.say(error) });
    }
  }

  /** Archive or unarchive the selected session. Archiving stops its process. */
  async edit(change: Exclude<HistoryChange, 'delete'>) {
    const target = this.#chosen();
    if (!target) return;
    await this.#acting.run(async () => {
      await this.#client.toSession(target, { kind: change });
      await this.#feed.until(() =>
        this.#feed.holds(target, change === 'archive' ? 'archived' : 'current'),
      );
      if (change === 'archive') this.#select(null);
    });
  }

  /** Stop the selected session's client. */
  stop() {
    const target = this.#chosen();
    if (target) void this.#acting.run(() => this.#client.toSession(target, { kind: 'stop' }));
  }

  answer(waiting: Waiting, decision: Decision) {
    const target = this.#chosen();
    if (!target) return;
    void this.#acting.run(() =>
      this.#client.toSession(target, { kind: 'answer', approval: waiting.approval, decision }),
    );
  }

  reply(waiting: Waiting, request: ReplyRequest) {
    const target = this.#chosen();
    if (!target) return;
    void this.#acting.run(() =>
      this.#client.toSession(target, { kind: 'reply', approval: waiting.approval, request }),
    );
  }

  #remember(next: Remembered) {
    this.remembered = next;
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
  async newDraft(dir?: string | null) {
    // A project's desk starts in the project's repository; Scratch, in a
    // scratch folder of its own.
    const scope = this.#scope();
    const place =
      dir !== undefined ? dir : scope.kind === 'project' ? scope.project.repository : null;
    const feed = this.#feed;
    const unused = Sessions.unused(feed.sessions, feed.live);
    await this.#acting.run(async () => {
      if (unused) {
        const here =
          place === undefined ||
          (place === null
            ? Sessions.inScratch(unused.workdir, feed.scratch)
            : unused.workdir === place);
        if (!here) {
          await this.#client.toSession(unused.session, { kind: 'folder', dir: place });
          await feed.readSessions();
        }
        this.#select(unused.session);
      } else {
        const remembered = this.remembered;
        const harness = remembered.harness;
        // The daemon keeps each client's models; without them, the client's own.
        const models = await this.#client.models(harness).catch(() => []);
        const policies = feed.clientOf(harness)?.approvals ?? [];
        // With no session to follow and no scratch folder, the first place offered.
        const folder = place === undefined ? (feed.offered[0]?.path ?? null) : place;
        const made = await this.#client.createSession(
          harness,
          folder,
          Memory.settings(remembered, harness, models, policies),
        );
        await Promise.all([feed.readSessions(), feed.readLive()]);
        this.#select(made);
      }
      await tick();
      this.#focus();
    });
  }

  /** Move the open draft to `dir` (null: a new scratch folder of its own). */
  moveDraft = (dir: string | null) => {
    const target = this.#chosen();
    if (!target) return;
    void this.#acting.run(async () => {
      await this.#client.toSession(target, { kind: 'folder', dir });
      await this.#feed.readSessions();
    });
  };

  /** Change the open session's settings from its next turn on, and keep
   * them for the next session on its client. */
  changeSettings = (settings: ChosenSettings) => {
    const session = this.#session();
    const harness = session ? Runs.drivable(session.harness) : null;
    if (harness) this.#remember(Memory.set(this.remembered, harness, settings));
    const target = this.#chosen();
    if (target)
      void this.#acting.run(async () => {
        await this.#client.toSession(target, { kind: 'settings', ...settings });
        await Promise.all([this.#feed.readSessions(), this.#feed.readLive()]);
      });
  };

  /** Put the open draft on another client, and keep it as the one chosen. */
  changeClient = (harness: Drivable) => {
    this.#remember(Memory.chose(this.remembered, harness));
    const target = this.#chosen();
    if (target)
      void this.#acting.run(async () => {
        await this.#client.toSession(target, { kind: 'client', harness });
        await Promise.all([this.#feed.readSessions(), this.#feed.readLive()]);
      });
  };
}
