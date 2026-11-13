<!--
  The sessions, grouped by the folder each works in: folders with a session
  that needs you first. Each folder's `+` makes a session there at once, on
  the last client used; New session opens the form for any other place. A
  session is named by what it was asked, when that is known here, and says
  how it stands. The archived sessions are a shelf of their own.
-->
<script lang="ts">
  import { SvelteSet } from 'svelte/reactivity';
  import { harnessLabel } from '#lib/activity.ts';
  import Icon from '#lib/Icon.svelte';
  import { ago } from '#lib/overview.ts';
  import {
    Sessions,
    type DeskSession,
    type SessionFolder,
    type SessionId,
    type Shelf,
  } from '#lib/runs.ts';

  let {
    folders,
    selected,
    now,
    title,
    place,
    scratch,
    shelf,
    archivedCount,
    busy,
    onshelf,
    onselect,
    oncreate,
    onnew,
  }: {
    folders: readonly SessionFolder[];
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
    onshelf: (shelf: Shelf) => void;
    onselect: (session: SessionId) => void;
    /** A session in `workdir` at once; null: a scratch folder of its own. */
    oncreate: (workdir: string | null) => void;
    onnew: () => void;
  } = $props();

  /** How many of a folder's sessions show before the rest fold. */
  const SHOWN = 5;
  /** Folders whose every session is shown. */
  const unfolded = new SvelteSet<string>();
</script>

{#snippet row(session: DeskSession)}
  {@const mark = Sessions.mark(session, harnessLabel(session.harness))}
  {@const asked = title(session)}
  {@const unused = !asked && session.phase === 'draft'}
  <button
    class="run-row"
    class:selected={session.session === selected}
    class:needs={mark.tone === 'warn'}
    aria-current={session.session === selected ? 'true' : undefined}
    onclick={() => onselect(session.session)}
  >
    <span
      class="phase-dot tone-{mark.tone}"
      class:draft={session.phase === 'draft'}
      class:resting={!Sessions.running(session) && mark.tone === 'neutral'}
    ></span>
    <span class="run-name" class:unused
      >{asked ?? (unused ? 'New session' : `${harnessLabel(session.harness)} session`)}</span
    >
    <span class="run-state tone-{mark.tone}"
      >{unused || mark.label === 'Draft'
        ? `Draft · ${harnessLabel(session.harness)}`
        : mark.label}</span
    >
    <span class="run-when muted">{session.activeAt ? ago(session.activeAt, now) : 'now'}</span>
  </button>
{/snippet}

<nav class="run-list" aria-label="Sessions by folder">
  {#if shelf === 'archived'}
    <button class="new-run" onclick={() => onshelf('current')}>
      <Icon name="archive" size={13} /> Archived sessions
      <span class="muted back">Back to sessions</span>
    </button>
  {:else}
    <button class="new-run" onclick={onnew}>
      <Icon name="spark" size={13} /> New session <kbd>n</kbd>
    </button>
  {/if}
  {#each folders as folder (folder.workdir)}
    {@const shown = place(folder.workdir)}
    {@const all = unfolded.has(folder.workdir) || shelf === 'archived'}
    {@const visible = all
      ? folder.sessions
      : folder.sessions.filter(
          (session, index) =>
            index < SHOWN || session.session === selected || Sessions.running(session),
        )}
    <section class="folder">
      <header title={folder.workdir}>
        <Icon name="file" size={12} />
        <strong>{folder.name}</strong>
        {#if folder.waiting}<span class="chip warning">{folder.waiting} needs you</span>{/if}
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
  .new-run kbd {
    margin-left: auto;
    font-size: 11px;
    color: var(--muted);
  }
  .folder {
    border-bottom: 1px solid var(--line);
    padding-bottom: 4px;
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
  .folder header .chip {
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
    grid-template-columns: 7px minmax(0, 1fr) auto;
    grid-template-rows: auto auto;
    column-gap: 8px;
    align-items: center;
    width: 100%;
    padding: 5px 12px 5px 18px;
    border: 0;
    background: none;
    color: var(--ink);
    font: inherit;
    font-size: 12px;
    text-align: left;
    cursor: pointer;
  }
  .run-row:hover {
    background: var(--row-hover);
  }
  .run-row.selected {
    background: var(--row-selected);
  }
  .run-row.needs .run-name {
    font-weight: 600;
  }
  .run-name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .run-state {
    grid-column: 2;
    grid-row: 2;
    font-size: 11px;
    color: var(--muted);
  }
  .run-state.tone-warn {
    color: var(--warning);
  }
  .run-state.tone-live {
    color: var(--live);
  }
  .run-state.tone-danger {
    color: var(--error);
  }
  .run-when {
    grid-column: 3;
    grid-row: 1 / span 2;
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
  .phase-dot {
    grid-row: 1 / span 2;
    box-sizing: border-box;
    width: 7px;
    height: 7px;
    border-radius: 50%;
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
  /* Nothing running: a ring, so the sessions that are live stand out. */
  .phase-dot.resting {
    background: none;
    box-shadow: inset 0 0 0 1.5px color-mix(in srgb, var(--muted) 65%, transparent);
  }
  .phase-dot.resting.draft {
    box-shadow: none;
    border: 1.5px dashed color-mix(in srgb, var(--muted) 65%, transparent);
  }
  .tone-live.phase-dot {
    animation: working 1.6s ease-in-out infinite;
  }
  @keyframes working {
    50% {
      box-shadow: 0 0 0 3px color-mix(in srgb, var(--live) 30%, transparent);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .tone-live.phase-dot {
      animation: none;
    }
  }
  .run-name.unused {
    color: var(--muted);
    font-style: italic;
  }
</style>
