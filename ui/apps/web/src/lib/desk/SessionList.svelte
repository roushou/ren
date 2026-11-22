<!--
  The sessions, grouped by the folder each works in: folders with a session
  that needs you first. Each folder's `+` makes a session there at once, on
  the last client used; New session opens the form for any other place. A
  session is named by what it was asked, when that is known here, and says
  how it stands. A draft never asked anything is listed only while it is
  open. The archived sessions are a shelf of their own.

  A session with no process running can be deleted from its row: the
  trash control takes the place of its time on hover or focus, and asks
  in the row itself before anything is sent. The row says while the delete
  is under way, keeps a refusal's reason, and folds away once the session
  has left the list.

  While the lists are on their way, a folder of outline rows stands where
  the sessions will be, drawn with the rows' own markup.
-->
<script module lang="ts">
  /** Whether the lists have been read: on their way, read, or kept as they
   * were when the read failed. */
  export type Listing = 'pending' | 'read' | 'failed';
</script>

<script lang="ts">
  import { tick } from 'svelte';
  import { SvelteSet } from 'svelte/reactivity';
  import { slide } from 'svelte/transition';
  import { harnessLabel } from '#lib/activity.ts';
  import type { Shelf } from '#lib/api.ts';
  import Glyph from '#lib/Glyph.svelte';
  import Icon from '#lib/Icon.svelte';
  import Skeleton from '#lib/Skeleton.svelte';
  import { ago } from '#lib/overview.ts';
  import {
    Deletions,
    Sessions,
    type Deletion,
    type DeletionEvent,
    type DeskSession,
    type SessionFolder,
    type SessionId,
  } from '#lib/runs.ts';

  let {
    folders,
    listing,
    selected,
    now,
    title,
    place,
    scratch,
    shelf,
    archivedCount,
    busy,
    deletable,
    deletion,
    ondelete,
    onshelf,
    onselect,
    oncreate,
    onnew,
  }: {
    folders: readonly SessionFolder[];
    listing: Listing;
    selected: SessionId | null;
    now: number;
    /** What a session was asked, when known here. */
    title: (session: DeskSession) => string | null;
    /** A folder as the operator reads it: Scratch, or `~/…`. */
    place: (workdir: string) => string;
    /** The folder scratch sessions get theirs in; null: no scratch. */
    scratch: string | null;
    shelf: Shelf;
    archivedCount: number;
    busy: boolean;
    /** Whether sessions can be deleted here: control is configured. */
    deletable: boolean;
    deletion: Deletion;
    ondelete: (event: DeletionEvent) => void;
    onshelf: (shelf: Shelf) => void;
    onselect: (session: SessionId) => void;
    /** A session in `workdir` at once; null: a scratch folder of its own. */
    oncreate: (workdir: string | null) => void;
    onnew: () => void;
  } = $props();

  /** How many of a folder's sessions show before the rest fold. */
  const SHOWN = 5;
  /** Folders whose every session is shown. */
  /** What the listed sessions are doing, for the list's head. */
  const running = $derived(
    folders.reduce(
      (sum, folder) => sum + folder.sessions.filter((session) => Sessions.running(session)).length,
      0,
    ),
  );
  const waiting = $derived(folders.reduce((sum, folder) => sum + folder.waiting, 0));
  const unfolded = new SvelteSet<string>();

  /** No fold for those who asked for less motion. */
  const still =
    typeof window !== 'undefined' && window.matchMedia('(prefers-reduced-motion: reduce)').matches;

  function nameOf(session: DeskSession): string {
    const asked = title(session);
    return (
      asked ??
      (session.phase === 'draft' ? 'New session' : `${harnessLabel(session.harness)} session`)
    );
  }

  /** The outline's rows: each name's width, varied so they read as text. */
  const OUTLINE_ROWS = ['72%', '54%', '83%', '61%'] as const;

  /** Back to the row's trash control, so the keyboard stays where it was. */
  async function cancel(session: DeskSession) {
    ondelete({ kind: 'cancel' });
    await tick();
    document
      .querySelector<HTMLButtonElement>(`[data-session="${session.session}"] .remove`)
      ?.focus();
  }

  /** The question's safe answer takes the focus when it opens. */
  function focus(node: HTMLElement) {
    node.focus();
  }
</script>

{#snippet row(session: DeskSession)}
  {@const mark = Sessions.mark(session, harnessLabel(session.harness))}
  {@const asked = title(session)}
  {@const unused = !asked && session.phase === 'draft'}
  {@const asking = Deletions.of(deletion, session.session, 'list')}
  <div
    class="run-item"
    class:selected={session.session === selected}
    data-session={session.session}
    out:slide={{ duration: still ? 0 : 180 }}
  >
    {#if asking}
      {@render confirm(session, asking)}
    {:else}
      {@render select(session, mark, asked, unused)}
      {#if deletable}<button
          class="remove"
          disabled={deletion.phase === 'deleting'}
          title="Delete session"
          aria-label="Delete session {nameOf(session)}"
          onclick={() => ondelete({ kind: 'ask', session: session.session, from: 'list' })}
          ><Icon name="trash" size={12} /></button
        >{/if}
    {/if}
  </div>
{/snippet}

{#snippet confirm(session: DeskSession, asking: Exclude<Deletion, { phase: 'idle' }>)}
  {@const sending = asking.phase === 'deleting'}
  <div
    class="run-confirm"
    class:sending
    role="alertdialog"
    aria-labelledby="delete-{session.session}"
    aria-describedby="delete-why-{session.session}"
    aria-busy={sending}
    tabindex="-1"
    onkeydown={(event) => {
      if (event.key !== 'Escape' || sending) return;
      event.stopPropagation();
      void cancel(session);
    }}
  >
    <span class="confirm-name" id="delete-{session.session}"
      >{sending ? 'Deleting' : 'Delete'} <bdi>{nameOf(session)}</bdi>{sending ? '…' : '?'}</span
    >
    <span
      class="confirm-why"
      class:failed={asking.phase === 'failed'}
      id="delete-why-{session.session}"
      >{asking.phase === 'failed'
        ? asking.reason
        : sending
          ? 'Removing it and its recorded calls.'
          : Sessions.running(session)
            ? 'Its client stops, and its runs and recorded calls go too.'
            : 'Its runs and recorded calls go too.'}</span
    >
    <span class="confirm-actions">
      <button
        class="button small danger confirm"
        disabled={sending}
        onclick={() => ondelete({ kind: 'confirm' })}
        >{asking.phase === 'failed' ? 'Retry' : 'Delete'}</button
      >
      <button
        class="button small"
        disabled={sending}
        {@attach focus}
        onclick={() => void cancel(session)}>Cancel</button
      >
    </span>
  </div>
{/snippet}

{#snippet select(
  session: DeskSession,
  mark: ReturnType<typeof Sessions.mark>,
  asked: string | null,
  unused: boolean,
)}
  <button
    class="run-row"
    class:selected={session.session === selected}
    class:needs={mark.mark === 'yours'}
    aria-current={session.session === selected ? 'true' : undefined}
    onclick={() => onselect(session.session)}
  >
    <Glyph
      mark={unused ? 'draft' : mark.mark}
      label={unused || mark.label === harnessLabel(session.harness)
        ? `${unused ? 'Draft' : 'Resting'} · ${harnessLabel(session.harness)}`
        : `${mark.label} · ${harnessLabel(session.harness)}`}
    />
    <span class="run-name" class:unused
      >{asked ?? (unused ? 'New session' : `${harnessLabel(session.harness)} session`)}</span
    >
    <span
      class="run-when muted mono"
      title={session.activeAt ? ago(session.activeAt, now) : undefined}
      >{session.activeAt ? Sessions.age(session.activeAt, now) : 'now'}</span
    >
  </button>
{/snippet}

<nav class="run-list" aria-label="Sessions by folder">
  {#if shelf === 'archived'}
    <button class="new-run" onclick={() => onshelf('current')}>
      <Icon name="archive" size={13} /> Archived sessions
      <span class="muted back">Back to sessions</span>
    </button>
  {:else}
    <header class="list-head">
      <span class="cap"
        >Sessions<span class="count"
          >· {folders.reduce((sum, folder) => sum + folder.sessions.length, 0)}</span
        ></span
      >
      {#if running || waiting}<span class="doing mono"
          >{#if running}<span title="{running} running"
              ><Glyph mark="responding" label="Running" />{running}</span
            >{/if}{#if waiting}<span title="{waiting} waiting for you"
              ><Glyph mark="yours" label="Waiting for you" />{waiting}</span
            >{/if}</span
        >{/if}
      <button class="button small" onclick={onnew}>New</button>
    </header>
  {/if}
  {#if listing === 'pending'}
    <section class="folder" aria-busy="true" aria-label="Sessions, on their way">
      <header>
        <Icon name="file" size={12} />
        <Skeleton width="38%" />
      </header>
      {#each OUTLINE_ROWS as width, index (index)}
        <div class="run-item" aria-hidden="true">
          <div class="run-row outline">
            <Glyph mark="idle" />
            <span class="run-name"><Skeleton {width} /></span>
            <span class="run-when"><Skeleton width="2.4em" /></span>
          </div>
        </div>
      {/each}
    </section>
  {:else}
    {#each folders as folder (folder.workdir)}
      {@const shown = place(folder.workdir)}
      {@const all = unfolded.has(folder.workdir) || shelf === 'archived'}
      {@const visible = all
        ? folder.sessions
        : folder.sessions.filter(
            (session, index) =>
              index < SHOWN || session.session === selected || Sessions.running(session),
          )}
      <section class="folder" class:single={folders.length === 1}>
        <header title={folder.workdir} class:hidden={folders.length === 1}>
          <Icon name="file" size={12} />
          <strong>{folder.name}</strong>
          {#if folder.waiting}<span class="waiting mono"
              ><Glyph
                mark="yours"
                label={`${folder.waiting} waiting for you`}
              />{folder.waiting}</span
            >{/if}
          {#if shown.includes('/')}<span class="muted mono parent"
              ><bdi>{shown.slice(0, shown.lastIndexOf('/') + 1)}</bdi></span
            >{/if}
          {#if shelf === 'current'}<button
              class="add"
              disabled={busy}
              title={folder.workdir === scratch
                ? 'New session in a scratch folder of its own'
                : `New session in ${shown}`}
              aria-label="New session in {folder.name}"
              onclick={() => oncreate(folder.workdir === scratch ? null : folder.workdir)}
              ><Icon name="plus" size={12} /></button
            >{/if}
        </header>
        {#each visible as session (session.session)}{@render row(session)}{/each}
        {#if visible.length < folder.sessions.length}
          <button class="fold" onclick={() => unfolded.add(folder.workdir)}>
            <Icon name="chevron" size={10} />
            {folder.sessions.length - visible.length} more
          </button>
        {/if}
      </section>
    {:else}
      <p class="note">
        {shelf === 'archived'
          ? 'Nothing is archived.'
          : 'No sessions yet. Start one with New session.'}
      </p>
    {/each}
  {/if}
  {#if shelf === 'current' && scratch && !folders.some((folder) => folder.workdir === scratch)}
    <section class="folder">
      <header>
        <Icon name="file" size={12} />
        <strong>Scratch</strong>
        <button
          class="add"
          disabled={busy}
          title="New session in a scratch folder of its own"
          aria-label="New session in Scratch"
          onclick={() => oncreate(null)}><Icon name="plus" size={12} /></button
        >
      </header>
    </section>
  {/if}
  {#if shelf === 'current' && archivedCount}
    <button class="shelf" onclick={() => onshelf('archived')}>
      <Icon name="archive" size={12} /> Archived <span class="muted">{archivedCount}</span>
    </button>
  {/if}
</nav>

<style>
  .run-list {
    overflow: auto;
    min-height: 0;
  }
  .new-run {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    padding: 10px 12px;
    border: 0;
    border-bottom: 1px solid var(--line);
    background: none;
    color: var(--ink);
    font: inherit;
    font-size: 12px;
    text-align: left;
    cursor: pointer;
  }
  .new-run:hover {
    background: var(--row-hover);
  }
  /* The list's head: how many, and a new one. */
  .list-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 10px 12px;
  }
  .cap {
    color: var(--muted);
    font-family: var(--mono, monospace);
    font-size: 11px;
    letter-spacing: 0.08em;
    text-transform: uppercase;
  }
  .count {
    margin-left: 6px;
  }
  .doing {
    display: inline-flex;
    gap: 10px;
    margin-left: auto;
    margin-right: 10px;
    font-size: 11px;
  }
  .doing > span {
    display: inline-flex;
    align-items: center;
    gap: 4px;
  }
  .folder {
    border-bottom: 1px solid var(--line);
    padding-bottom: 4px;
  }
  /* One folder needs no heading: the project names it. */
  .folder.single {
    border-bottom: 0;
  }
  .folder header.hidden {
    display: none;
  }
  .folder header {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 10px 12px 4px;
    font-size: 12px;
    min-width: 0;
  }
  .folder header strong {
    white-space: nowrap;
  }
  .folder header .parent {
    flex: 1;
    min-width: 0;
    margin-left: auto;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: 11px;
    direction: rtl;
    text-align: left;
  }
  /* A quiet control: rounded as controls are, square everything else. */
  .add {
    display: grid;
    place-items: center;
    flex: none;
    width: 20px;
    height: 20px;
    margin-left: auto;
    padding: 0;
    border: 1px solid transparent;
    border-radius: 3px;
    background: none;
    color: var(--muted);
    cursor: pointer;
  }
  .folder header .parent + .add {
    margin-left: 0;
  }
  .add:hover,
  .add:focus-visible {
    border-color: var(--control-border);
    background: var(--surface);
    color: var(--ink);
  }
  .add:disabled {
    opacity: 0.5;
    cursor: default;
  }
  .run-row {
    display: grid;
    grid-template-columns: 10px minmax(0, 1fr) auto;
    column-gap: 10px;
    align-items: center;
    width: 100%;
    min-height: 34px;
    padding: 6px 12px;
    border: 0;
    border-bottom: 1px solid var(--line-subtle);
    box-shadow: inset 2px 0 transparent;
    background: none;
    color: var(--ink);
    font: inherit;
    font-size: 12.5px;
    font-weight: 500;
    text-align: left;
    cursor: pointer;
  }
  .run-row:hover {
    background: var(--row-hover);
  }
  .run-row.outline {
    cursor: default;
  }
  .run-row.outline:hover {
    background: none;
  }
  .run-row.selected {
    background: var(--row-selected);
    box-shadow: inset 2px 0 var(--accent);
  }
  /* A row and its trash control, which stands where its time does. */
  .run-item {
    position: relative;
  }
  .remove {
    position: absolute;
    top: 50%;
    right: 10px;
    display: grid;
    place-items: center;
    width: 22px;
    height: 22px;
    padding: 0;
    border: 1px solid transparent;
    border-radius: 3px;
    background: none;
    color: var(--muted);
    cursor: pointer;
    opacity: 0;
    transform: translateY(-50%);
    transition: opacity 120ms ease;
  }
  .run-item:hover .remove,
  .remove:focus-visible {
    opacity: 1;
  }
  .run-item:has(.remove):hover .run-when,
  .run-item:has(.remove:focus-visible) .run-when {
    visibility: hidden;
  }
  .remove:hover,
  .remove:focus-visible {
    border-color: var(--error);
    background: var(--error-soft);
    color: var(--error);
  }
  .remove:disabled {
    opacity: 0;
    pointer-events: none;
  }
  /* Without hover, the selected session's control stays in view. */
  @media (hover: none) {
    .run-item.selected .remove:not(:disabled) {
      opacity: 1;
    }
    .run-item.selected:has(.remove:not(:disabled)) .run-when {
      visibility: hidden;
    }
  }
  /* The question, in the row it is about: ruled in the danger color. */
  .run-confirm {
    position: relative;
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    column-gap: 8px;
    row-gap: 1px;
    align-items: center;
    padding: 5px 10px 5px 18px;
    background: var(--error-soft);
    box-shadow: inset 2px 0 0 var(--error);
    font-size: 12px;
    animation: ask 140ms ease-out;
  }
  .run-confirm:focus {
    outline: none;
  }
  .confirm-name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--ink);
  }
  .confirm-why {
    grid-column: 1;
    font-size: 11px;
    color: var(--muted);
  }
  .confirm-why.failed {
    color: var(--error);
  }
  .confirm-actions {
    grid-column: 2;
    grid-row: 1 / span 2;
    display: flex;
    gap: 4px;
  }
  .run-confirm.sending .confirm-name {
    color: var(--muted);
  }
  /* Under way: a bar runs along the row's foot until the session is gone. */
  .run-confirm.sending::after {
    content: '';
    position: absolute;
    left: 0;
    right: 0;
    bottom: 0;
    height: 2px;
    background: linear-gradient(90deg, transparent, var(--error), transparent);
    background-size: 40% 100%;
    background-repeat: no-repeat;
    animation: sending 1s linear infinite;
  }
  @keyframes ask {
    from {
      background: transparent;
      box-shadow: inset 0 0 0 var(--error);
    }
  }
  @keyframes sending {
    from {
      background-position: -40% 0;
    }
    to {
      background-position: 140% 0;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .remove {
      transition: none;
    }
    .run-confirm {
      animation: none;
    }
    .run-confirm.sending::after {
      animation: none;
      background: var(--error);
      opacity: 0.5;
    }
  }
  .run-row.needs .run-name {
    font-weight: 600;
  }
  .run-name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .run-when {
    font-size: 11px;
  }
  .new-run .back {
    margin-left: auto;
    font-size: 11px;
  }
  .shelf {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 100%;
    padding: 8px 12px;
    border: 0;
    border-bottom: 1px solid var(--line);
    background: none;
    color: var(--muted);
    font: inherit;
    font-size: 12px;
    text-align: left;
    cursor: pointer;
  }
  .shelf:hover {
    background: var(--row-hover);
    color: var(--ink);
  }
  .fold {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 100%;
    padding: 4px 12px 4px 33px;
    border: 0;
    background: none;
    color: var(--muted);
    font: inherit;
    font-size: 11px;
    cursor: pointer;
  }
  .fold:hover {
    color: var(--ink);
  }
  .note {
    margin: 0;
    padding: 12px;
    font-size: 12px;
    color: var(--muted);
  }
  .waiting {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: 11px;
    color: var(--ink);
  }
  .run-name.unused {
    color: var(--muted);
    font-style: italic;
  }
</style>
