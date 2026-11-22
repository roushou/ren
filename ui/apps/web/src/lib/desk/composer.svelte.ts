// The session desk's composer: each session's draft, the prompts ↑ recalls,
// what the word at the caret is offered (the client's commands after `/`
// or `$`, the folder's files after `@`), the prompts on their way, and the
// images about to go with the next. It sends through the runs client and
// reads the feed's lists again once something went.
import { tick, untrack } from 'svelte';
import { SvelteMap } from 'svelte/reactivity';
import { harnessLabel } from '#lib/activity.ts';
import {
  Gallery,
  Tray,
  type Chip,
  type KeptAttachment,
  type UploadedImage,
} from '#lib/attachments.ts';
import { assertNever } from '#lib/decode.ts';
import { Mentions } from '#lib/mention.ts';
import { Outbox, type Outgoing, type OutgoingDelivery, type Starts } from '#lib/outbox.ts';
import { Refusals } from '#lib/refusal.ts';
import {
  Compose,
  RunCommandError,
  RunsClient,
  Sessions,
  type ClientCommand,
  type ComposeMode,
  type Delivery,
  type DeskSession,
  type FileMatch,
  type RunControls,
  type SessionId,
} from '#lib/runs.ts';
import type { Acting } from './acting.svelte.ts';
import type { MenuEntry } from './ComposerMenu.svelte';
import type { DeskFeed } from './feed.svelte.ts';
import type { DeskData } from './load.ts';

/** How Enter sends while a turn runs: queued after it, or into it. */
export type Sending = Exclude<Delivery, 'start'>;

export interface ComposerOptions {
  client: RunsClient;
  feed: DeskFeed;
  acting: Acting;
  /** The desk's data, as its page has it now. */
  data: () => DeskData;
  /** The session the desk shows, by its id, and as listed. */
  chosen: () => SessionId | null;
  session: () => DeskSession | null;
  /** What can be asked of the session now. */
  controls: () => RunControls | null;
  /** How its composer works now. */
  compose: () => ComposeMode | null;
  /** Its client is signed out here: a prompt would only fail. */
  signedOut: () => boolean;
  /** A prompt went: the conversation follows its newest entry again. */
  onsent: () => void;
}

export class ComposerState {
  readonly #client: RunsClient;
  readonly #feed: DeskFeed;
  readonly #acting: Acting;
  readonly #data: () => DeskData;
  readonly #chosen: () => SessionId | null;
  readonly #session: () => DeskSession | null;
  readonly #controls: () => RunControls | null;
  readonly #compose: () => ComposeMode | null;
  readonly #signedOut: () => boolean;
  readonly #onsent: () => void;

  /** The prompt's field, once drawn. */
  field = $state<HTMLTextAreaElement>();
  /** Each session's draft, kept while the page is open. */
  readonly drafts = new SvelteMap<SessionId, string>();
  /** How far back ↑ has recalled the run's prompts; null: not recalling. */
  recalling = $state<number | null>(null);
  /** What was written before recalling, for ↓ to come back to. */
  #beforeRecall = '';
  readonly draft = $derived.by(() => {
    const chosen = this.#chosen();
    return chosen ? (this.drafts.get(chosen) ?? '') : '';
  });

  /** Every session's prompts on their way, shown until the conversation shows them. */
  outbox = $state.raw(new Outbox());
  readonly outgoing = $derived.by(() => {
    const chosen = this.#chosen();
    return chosen ? this.outbox.of(chosen) : [];
  });
  /** One of the session's prompts has no answer from the daemon yet: the next waits for it. */
  readonly sending = $derived.by(() => {
    const chosen = this.#chosen();
    return chosen !== null && this.outbox.waiting(chosen);
  });

  /** Each session's images about to go with its next prompt. */
  readonly trays = new SvelteMap<SessionId, Tray>();
  readonly tray = $derived.by(() => {
    const chosen = this.#chosen();
    return chosen ? (this.trays.get(chosen) ?? new Tray()) : new Tray();
  });
  /** Each session's attachments as the daemon lists them, to show what a prompt sent. */
  readonly kept = new SvelteMap<SessionId, KeptAttachment[]>();
  readonly gallery = $derived.by(() => {
    const chosen = this.#chosen();
    return new Gallery(chosen ? (this.kept.get(chosen) ?? []) : []);
  });
  /** The tray's uploaded images, to step through in the viewer. */
  readonly trayViewable = $derived.by(() => {
    const chosen = this.#chosen();
    return chosen
      ? this.tray.chips.flatMap((chip) =>
          chip.state === 'ready' ? [{ src: Gallery.url(chosen, chip.id), read: chip.read }] : [],
        )
      : [];
  });
  /** Whether the composer offers images: not for a client that takes none. */
  readonly imagesTaken = $derived.by(() => this.#session()?.images !== 'never');

  /** Where the caret is in the prompt, for what the word there asks to be offered. */
  caret = $state(0);
  /** Where an offer closed with Esc starts: it stays closed until another word asks. */
  dismissed = $state<number | null>(null);
  // A closed composer's prompt is disabled, so nothing asks there.
  readonly mention = $derived(Mentions.at(this.draft, Math.min(this.caret, this.draft.length)));
  readonly offering = $derived(
    this.mention && this.mention.start !== this.dismissed ? this.mention : null,
  );
  /** Each session's commands, by its client and folder, as its client lists them. */
  readonly commandLists = new SvelteMap<string, ClientCommand[]>();
  readonly commandsKey = $derived.by(() => {
    const session = this.#session();
    return session ? `${session.session} ${session.harness} ${session.workdir}` : null;
  });
  commandsNote = $state<string | null>(null);
  /** The newest file search's answer, with what it searched for. */
  found = $state.raw<{ session: SessionId; query: string; files: FileMatch[] } | null>(null);
  filesNote = $state<string | null>(null);
  readonly offeredCommands = $derived(
    this.offering?.kind === 'command'
      ? Mentions.commands(this.commandLists.get(this.commandsKey ?? '') ?? [], this.offering)
      : [],
  );
  readonly offeredFiles = $derived.by(() =>
    this.offering?.kind === 'file' && this.found && this.found.session === this.#chosen()
      ? this.found.files
      : [],
  );
  readonly entries = $derived.by((): MenuEntry[] => {
    const offering = this.offering;
    if (offering?.kind === 'command') {
      return this.offeredCommands.map((command) => {
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
      return this.offeredFiles.map((file) => ({
        key: file.path,
        parts: Mentions.marked(file.path, file.positions),
        hint: null,
        tag: file.kind === 'directory' ? 'folder' : null,
        detail: '',
      }));
    }
    return [];
  });
  /** The entry ↑ ↓ choose; a new list starts at its best one. */
  active = $derived.by(() => (void this.entries, 0));
  readonly menuId = 'composer-offers';
  /** What the menu says instead of entries: loading, nothing found, or why it failed. */
  readonly menuNote = $derived.by(() => {
    if (this.offering?.kind === 'file') return this.filesNote ?? (this.found ? null : 'Searching…');
    return (
      this.commandsNote ??
      ((this.commandLists.get(this.commandsKey ?? '')?.length ?? 0) > 0
        ? 'No command matches.'
        : null)
    );
  });

  /** Whether the composer offers to stop the running turn. */
  readonly stopOffered = $derived.by(
    () => this.#data().runsControl && this.#controls()?.interrupt !== 'none',
  );
  /** Whether the running turn can be stopped now. */
  readonly canStop = $derived.by(
    () =>
      this.#data().runsControl &&
      this.#controls()?.interrupt === 'ready' &&
      !this.#acting.busy &&
      this.#chosen() !== null,
  );
  /** Whether what is written can go now, queued or into the turn. */
  readonly sendable = $derived.by(
    () =>
      !this.#acting.busy &&
      !this.sending &&
      (this.draft.trim() !== '' || this.tray.ids().length > 0) &&
      !this.tray.uploading() &&
      this.#compose()?.kind !== 'closed' &&
      !this.#signedOut(),
  );

  constructor(options: ComposerOptions) {
    this.#client = options.client;
    this.#feed = options.feed;
    this.#acting = options.acting;
    this.#data = options.data;
    this.#chosen = options.chosen;
    this.#session = options.session;
    this.#controls = options.controls;
    this.#compose = options.compose;
    this.#signedOut = options.signedOut;
    this.#onsent = options.onsent;

    $effect(() => {
      // A prompt the conversation shows is the client's echo of one on its way.
      const target = this.#chosen();
      const prompts = this.#feed.transcript?.prompts() ?? [];
      if (target) untrack(() => (this.outbox = this.outbox.shown(target, prompts)));
    });

    $effect(() => {
      const target = this.#chosen();
      if (target) untrack(() => void this.readAttachments(target));
    });

    $effect(() => {
      // The session's commands, read as it opens, before any `/`: its
      // running client's list, or what its client offers in its folder
      // (listed once per client and folder by starting it briefly there).
      // An empty list is asked again when a word asks for commands.
      const key = this.commandsKey;
      const target = this.#chosen();
      const harness = this.#session()?.harness;
      const asking = this.offering?.kind === 'command' ? this.offering.start : null;
      if (!key || !target || !harness) return;
      const known = untrack(() => this.commandLists.get(key));
      if (known && (known.length > 0 || asking === null)) return;
      this.commandsNote = 'Reading the commands…';
      const abort = new AbortController();
      this.#client
        .sessionCommands(target, abort.signal)
        .then((commands) => {
          this.commandLists.set(key, commands);
          this.commandsNote =
            commands.length > 0 ? null : `${harnessLabel(harness)} offers no commands here.`;
        })
        .catch((error: unknown) => {
          if (!abort.signal.aborted) this.commandsNote = RunCommandError.say(error);
        });
      return () => abort.abort();
    });

    $effect(() => {
      // A short pause after typing, then the folder is searched.
      const target = this.#chosen();
      const query = this.offering?.kind === 'file' ? this.offering.query : null;
      if (!target || query === null) return;
      const abort = new AbortController();
      const timer = setTimeout(() => {
        this.#client
          .sessionFiles(target, query, 20, abort.signal)
          .then((files) => {
            this.found = { session: target, query, files };
            this.filesNote = files.length > 0 ? null : 'No file matches.';
          })
          .catch((error: unknown) => {
            if (!abort.signal.aborted) this.filesNote = RunCommandError.say(error);
          });
      }, 90);
      return () => {
        clearTimeout(timer);
        abort.abort();
      };
    });
  }

  write(text: string) {
    const chosen = this.#chosen();
    if (chosen) this.drafts.set(chosen, text);
  }

  /** Focus the prompt once what changed is drawn. */
  focusSoon() {
    void tick().then(() => this.field?.focus());
  }

  /** Write `prompt` as `session`'s draft, ready to send. */
  prefill(session: SessionId, prompt: string) {
    this.drafts.set(session, prompt);
    this.focusSoon();
  }

  /** Add `quote` to what is written, to ask about it. */
  quote(quote: string) {
    const target = this.#chosen();
    if (!target) return;
    const written = this.drafts.get(target) ?? '';
    this.drafts.set(target, written ? `${written}\n\n${quote}\n\n` : `${quote}\n\n`);
  }

  async readAttachments(target: SessionId) {
    if (!this.#data().runsControl) return;
    try {
      this.kept.set(target, await this.#client.attachments(target, this.#feed.signal));
    } catch {
      // Without the listing, sent images show by name.
    }
  }

  /** Upload each image to the session; each is a chip until it is sent. */
  async addImages(files: readonly File[]) {
    const target = this.#chosen();
    if (!target || !this.imagesTaken) return;
    for (const file of files) {
      const start = (this.trays.get(target) ?? new Tray()).adding(file.name || 'image');
      this.trays.set(target, start.tray);
      const settle = (change: (now: Tray) => Tray) =>
        this.trays.set(target, change(this.trays.get(target) ?? new Tray()));
      if (!Gallery.takes(file.type)) {
        settle((now) => now.refused(start.key, Refusals.say('image_required')));
        continue;
      }
      try {
        const bytes = await file.arrayBuffer();
        const made = await this.#client.attach(target, file.name || 'image', file.type, bytes);
        settle((now) => now.uploaded(start.key, made.id, made.read));
      } catch (error) {
        const code = error instanceof RunCommandError ? error.code : null;
        settle((now) =>
          now.refused(start.key, code ? Refusals.say(code) : RunCommandError.say(error)),
        );
      }
    }
  }

  /** Take an image out of the tray; one uploaded is removed from the session. */
  async removeChip(chip: Chip) {
    const target = this.#chosen();
    if (!target) return;
    if (chip.state === 'ready') {
      try {
        await this.#client.detach(target, chip.id);
      } catch (error) {
        this.#acting.fail(error);
        return;
      }
    }
    this.trays.set(target, (this.trays.get(target) ?? new Tray()).without(chip.key));
  }

  /** Write the chosen entry in place of the word that asked for it. */
  pick = (index: number) => {
    const offering = this.offering;
    if (!offering) return;
    const command = this.offeredCommands[index];
    const file = this.offeredFiles[index];
    const written =
      offering.kind === 'command'
        ? command && Mentions.command(this.draft, offering, command)
        : file && Mentions.file(this.draft, offering, file);
    if (!written) return;
    this.write(written.text);
    this.caret = written.caret;
    void tick().then(() => {
      this.field?.focus();
      this.field?.setSelectionRange(written.caret, written.caret);
    });
  };

  trackCaret = (event: Event) => {
    this.caret = (event.currentTarget as HTMLTextAreaElement).selectionStart;
  };

  /** What was typed: recalling and a dismissed offer end with it. */
  typed = (event: Event & { currentTarget: HTMLTextAreaElement }) => {
    this.recalling = null;
    this.dismissed = null;
    this.write(event.currentTarget.value);
    this.trackCaret(event);
  };

  /**
   * Send what is written to the session: it starts a turn (starting or
   * resuming its client when none runs), or, while a turn runs, is queued
   * for after it or steers it. A prompt that starts or steers a turn shows
   * in the conversation at once, marked with how far it has got.
   */
  async send(delivery: Sending = 'queue') {
    const text = this.draft.trim();
    const target = this.#chosen();
    const tray = this.tray;
    const images = tray.ready();
    if (!target || (!text && images.length === 0) || tray.uploading()) return;
    const compose = this.#compose();
    if (!compose || this.#signedOut() || this.outbox.waiting(target)) return;
    switch (compose.kind) {
      case 'start':
      case 'continue':
        return this.#deliver(target, text, images, 'start');
      case 'during':
        if (delivery === 'steer' && compose.steer)
          return this.#deliver(target, text, images, 'steer');
        return this.#queue(target, text, compose.queued);
      case 'closed':
        return;
      default:
        return assertNever(compose);
    }
  }

  /** Show the prompt as on its way, clear the composer, and send it. */
  #deliver(target: SessionId, text: string, images: UploadedImage[], how: OutgoingDelivery) {
    const session = this.#session();
    const starts: Starts = session && Sessions.running(session) ? 'nothing' : 'client';
    const made = this.outbox.sending(
      { session: target, text, images, delivery: how },
      this.#feed.transcript?.prompts() ?? [],
      starts,
    );
    this.outbox = made.outbox;
    this.trays.set(target, (this.trays.get(target) ?? new Tray()).sent());
    this.drafts.delete(target);
    this.recalling = null;
    this.#onsent();
    void this.#transmit(made.key);
  }

  /** Ask the daemon to take prompt `key`, and mark what it answered. */
  async #transmit(key: number) {
    const item = this.outbox.items.find((one) => one.key === key);
    if (!item) return;
    try {
      await this.#client.toSession(item.session, {
        kind: 'prompt',
        text: item.text,
        delivery: item.delivery,
        attachments: item.images.map((image) => image.id),
      });
      this.outbox = this.outbox.sent(key);
    } catch (error) {
      this.outbox = this.outbox.failed(key, error);
    }
    // The feeds say the same soon; read now so the page catches up at once.
    void this.#catchUp(item.session);
  }

  /** Queue the prompt for after the running turn, after what was queued before. */
  async #queue(target: SessionId, text: string, queued: string | null) {
    const attachments = this.tray.ids();
    await this.#acting.run(async () => {
      await this.#client.toSession(target, {
        kind: 'prompt',
        text: queued ? `${queued}\n\n${text}` : text,
        delivery: 'queue',
        attachments,
      });
      this.trays.set(target, (this.trays.get(target) ?? new Tray()).sent());
      this.drafts.delete(target);
      this.recalling = null;
      this.#onsent();
    });
    void this.#catchUp(target);
  }

  #catchUp(target: SessionId) {
    return Promise.all([
      this.#feed.readSessions(),
      this.#feed.readLive(),
      this.readAttachments(target),
    ]);
  }

  /** Send a prompt that was refused, or whose outcome is unknown, again. */
  resend(item: Outgoing) {
    this.outbox = this.outbox.without(item.key);
    this.#deliver(item.session, item.text, item.images, item.delivery);
  }

  /** Take a prompt that did not go back into the composer, with its images. */
  takeBack(item: Outgoing) {
    this.outbox = this.outbox.without(item.key);
    const written = this.drafts.get(item.session) ?? '';
    this.drafts.set(item.session, written ? `${item.text}\n\n${written}` : item.text);
    this.trays.set(
      item.session,
      (this.trays.get(item.session) ?? new Tray()).restored(item.images),
    );
    if (item.session === this.#chosen()) this.focusSoon();
  }

  /** Take the queued prompt back into the composer to change it. */
  async editQueued(queued: string) {
    const target = this.#chosen();
    if (!target) return;
    await this.#acting.run(async () => {
      await this.#client.toSession(target, { kind: 'unqueue' });
      this.write(this.draft ? `${queued}\n\n${this.draft}` : queued);
      this.field?.focus();
    });
  }

  /** Drop the prompt queued for after the turn. */
  unqueue() {
    const target = this.#chosen();
    if (target) void this.#acting.run(() => this.#client.toSession(target, { kind: 'unqueue' }));
  }

  /** Interrupt the running turn; the client ends it as interrupted. */
  stop = () => {
    const target = this.#chosen();
    if (target) void this.#acting.run(() => this.#client.toSession(target, { kind: 'interrupt' }));
  };

  key = (event: KeyboardEvent) => {
    if (event.isComposing) return;
    // While something is offered, the arrows, Enter, and Tab are its.
    const offering = this.offering;
    if (offering) {
      const count = this.entries.length;
      if (event.key === 'Escape') {
        event.preventDefault();
        this.dismissed = offering.start;
        return;
      }
      if (count > 0 && (event.key === 'ArrowDown' || event.key === 'ArrowUp')) {
        event.preventDefault();
        this.active = (this.active + (event.key === 'ArrowDown' ? 1 : count - 1)) % count;
        return;
      }
      if (
        count > 0 &&
        (event.key === 'Tab' ||
          (event.key === 'Enter' && !event.shiftKey && !event.metaKey && !event.ctrlKey))
      ) {
        event.preventDefault();
        this.pick(this.active);
        return;
      }
    }
    if (event.key === 'Escape' && this.canStop) {
      event.preventDefault();
      this.stop();
      return;
    }
    if (event.key === 'Enter' && !event.shiftKey) {
      event.preventDefault();
      // Cmd or Ctrl+Enter sends into the running turn, where the client can.
      void this.send(event.metaKey || event.ctrlKey ? 'steer' : 'queue');
      return;
    }
    const field = event.currentTarget as HTMLTextAreaElement;
    const atStart = field.selectionStart === 0 && field.selectionEnd === 0;
    const prompts = this.#feed.transcript?.prompts() ?? [];
    if (event.key === 'ArrowUp' && atStart && (this.draft === '' || this.recalling !== null)) {
      if (this.recalling === null) this.#beforeRecall = this.draft;
      const next = Compose.recall(prompts, this.recalling, true);
      if (!next) return;
      event.preventDefault();
      this.recalling = next.at;
      this.write(next.text);
    } else if (event.key === 'ArrowDown' && this.recalling !== null) {
      const next = Compose.recall(prompts, this.recalling, false);
      if (!next) return;
      event.preventDefault();
      this.recalling = next.at;
      this.write(next.at === null ? this.#beforeRecall : next.text);
    }
  };
}
