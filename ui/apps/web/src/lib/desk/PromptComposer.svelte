<!--
  A session's composer: the prompt queued for after the running turn, the
  word that its client is signed out, then the prompt itself, with the
  images about to go, what the word at the caret is offered, the folder and
  settings of the next turn, and the controls that send it or stop the turn.
  Images come by the picker, a paste, or a drop.
-->
<script lang="ts">
  import Icon from '#lib/Icon.svelte';
  import { harnessLabel } from '#lib/activity.ts';
  import { Gallery, type Viewable } from '#lib/attachments.ts';
  import { SignIns } from '#lib/signIn.ts';
  import {
    DRIVABLE,
    MIDWAY,
    Runs,
    RunsClient,
    Sessions,
    type ChosenSettings,
    type ComposeMode,
    type DeskSession,
    type Drivable,
    type RunClient,
    type RunControls,
    type SessionFolder,
  } from '#lib/runs.ts';
  import ComposerMenu from './ComposerMenu.svelte';
  import FolderPicker from './FolderPicker.svelte';
  import RunSettingsBar from './RunSettingsBar.svelte';
  import Thumbnail from './Thumbnail.svelte';
  import type { ComposerState } from './composer.svelte.ts';
  import type { DeskFeed } from './feed.svelte.ts';

  let {
    composer,
    feed,
    client,
    session,
    compose,
    controls,
    sessionClient,
    signedOut,
    busy,
    folders,
    onmove,
    onsettings,
    onclient,
    onsignin,
    onview,
  }: {
    composer: ComposerState;
    feed: DeskFeed;
    client: RunsClient;
    session: DeskSession;
    compose: ComposeMode;
    controls: RunControls;
    /** What the session's client takes mid-run, as last read. */
    sessionClient: RunClient | null;
    /** The session's client is signed out here: nothing is sent. */
    signedOut: boolean;
    busy: boolean;
    /** The sessions by folder, for where a draft can move. */
    folders: SessionFolder[];
    /** Move the open draft to `dir` (null: a new scratch folder of its own). */
    onmove: (dir: string | null) => void;
    onsettings: (settings: ChosenSettings) => void;
    /** Change the draft's client. */
    onclient: (harness: Drivable) => void;
    onsignin: (harness: Drivable) => void;
    /** Open `images` full size, from the one at `start`. */
    onview: (images: readonly Viewable[], start: number) => void;
  } = $props();

  /** The file picker, opened by the attach button. */
  let picker = $state<HTMLInputElement>();
  /** Something is being dragged over the composer. */
  let dropping = $state(false);

  /** The key that sends into a running turn, as this platform names it. */
  const navigatorKey =
    typeof navigator !== 'undefined' && /Mac|iPhone|iPad/.test(navigator.platform) ? '⌘' : 'Ctrl';

  const label = $derived(harnessLabel(session.harness));
  const drivable = $derived(Runs.drivable(session.harness));
  const chosen = $derived(session.session);

  const placeholder = $derived(
    signedOut
      ? `${label} is signed out. Write your prompt now; it sends once ${label} is signed in.`
      : compose.kind === 'start'
        ? `Ask ${label} to do something. Enter sends, ↑ recalls an earlier prompt.`
        : compose.kind === 'during'
          ? compose.steer
            ? `Enter queues for after this turn; ${navigatorKey}+Enter sends it into the turn now; Esc stops the turn.`
            : `Enter queues for after this turn; ${label} reads it then. Esc stops the turn.`
          : compose.kind === 'continue'
            ? `Write to ${label} to go on.`
            : compose.reason,
  );

  function imagesOf(transfer: DataTransfer | null): File[] {
    return [...(transfer?.files ?? [])].filter((file) => file.type.startsWith('image/'));
  }
</script>

{#if compose.kind === 'during' && compose.queued}
  <div class="queued" role="status">
    <span class="queued-label">After this turn</span>
    <span class="queued-text">{compose.queued}</span>
    <button
      class="text-button"
      disabled={busy}
      onclick={() => composer.editQueued(compose.queued ?? '')}>Edit</button
    >
    <button class="text-button" disabled={busy} onclick={() => composer.unqueue()}>Remove</button>
  </div>
{/if}
{#if sessionClient && signedOut && drivable}
  {@const harness = drivable}
  <div class="signed-out" role="status">
    <span
      ><Icon name="lock" size={12} />
      {label} is signed out on the daemon’s host, so nothing can be sent. Signed in some other way? Check
      again.</span
    >
    <button
      class="button small"
      type="button"
      disabled={feed.checking}
      onclick={() => void feed.checkSignIn(harness)}
      >{feed.checking ? 'Checking…' : 'Check again'}</button
    >
  </div>
{/if}
<form
  class="composer"
  class:locked={signedOut}
  class:dropping
  onsubmit={(event) => {
    event.preventDefault();
    void composer.send('queue');
  }}
  ondragover={(event) => {
    if (!composer.imagesTaken || compose.kind === 'closed') return;
    if ([...(event.dataTransfer?.types ?? [])].includes('Files')) {
      event.preventDefault();
      dropping = true;
    }
  }}
  ondragleave={() => (dropping = false)}
  ondrop={(event) => {
    dropping = false;
    const files = imagesOf(event.dataTransfer);
    if (files.length === 0 || !composer.imagesTaken || compose.kind === 'closed') return;
    event.preventDefault();
    void composer.addImages(files);
  }}
>
  {#if composer.tray.chips.length > 0}
    <ul class="tray" aria-label="Images to send">
      {#each composer.tray.chips as chip (chip.key)}
        <li class="tray-chip {chip.state}">
          {#if chip.state === 'ready'}
            {@const read = chip.read}
            <Thumbnail
              src={Gallery.url(chosen, chip.id)}
              {read}
              shown="tray"
              onview={() => {
                const start = composer.tray.chips
                  .filter((one) => one.state === 'ready')
                  .findIndex((one) => one.key === chip.key);
                onview(composer.trayViewable, start);
              }}
            />
            <span class="tray-name" title="{read.name} · {Gallery.describe(read)}">{read.name}</span
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
              onclick={() => void composer.removeChip(chip)}><Icon name="close" size={10} /></button
            >
          {/if}
        </li>
      {/each}
    </ul>
  {/if}
  {#if composer.offering}
    <ComposerMenu
      id={composer.menuId}
      heading={composer.offering.kind === 'file'
        ? 'Files in the run’s folder'
        : composer.offering.sigil === '$'
          ? `${label} skills`
          : `${label} commands`}
      entries={composer.entries}
      active={composer.active}
      note={composer.menuNote}
      onpick={composer.pick}
      onpoint={(index) => (composer.active = index)}
    />
  {/if}
  <textarea
    bind:this={composer.field}
    value={composer.draft}
    oninput={composer.typed}
    onkeydown={composer.key}
    onpaste={(event) => {
      const files = imagesOf(event.clipboardData);
      if (files.length === 0 || !composer.imagesTaken) return;
      event.preventDefault();
      void composer.addImages(files);
    }}
    onkeyup={composer.trackCaret}
    onclick={composer.trackCaret}
    onfocus={composer.trackCaret}
    role="combobox"
    aria-autocomplete="list"
    aria-expanded={composer.offering !== null}
    aria-controls={composer.offering ? composer.menuId : undefined}
    aria-activedescendant={composer.offering && composer.entries.length > 0
      ? `${composer.menuId}-${composer.active}`
      : undefined}
    rows="3"
    disabled={compose.kind === 'closed'}
    {placeholder}
    aria-label="Prompt"></textarea>
  <div class="composer-bar">
    {#if session.phase === 'draft' && session.shelf === 'open'}
      <FolderPicker
        workdir={session.workdir}
        offered={feed.offered}
        scratch={feed.scratch}
        runs={folders.flatMap((folder) =>
          folder.sessions.map((entry) => ({
            workdir: entry.workdir,
            started_at: entry.runs[0]?.started_at ?? null,
          })),
        )}
        disabled={busy ? 'Waiting for the last change' : null}
        onchange={onmove}
      />
    {/if}
    {#if drivable && sessionClient}
      <RunSettingsBar
        harness={drivable}
        settings={session.settings}
        midway={Sessions.running(session) ? sessionClient.midway : MIDWAY}
        approvals={sessionClient.approvals}
        listModels={(harness) => client.models(harness)}
        {busy}
        onchange={onsettings}
        choice={session.phase === 'draft'
          ? {
              clients: DRIVABLE.filter((harness) =>
                feed.clients.some((entry) => entry.harness === harness && entry.status === 'ready'),
              ),
              hint: (harness) => {
                const entry = feed.clientOf(harness);
                return entry ? SignIns.hint(entry.sign_in) : null;
              },
              onclient,
            }
          : null}
      />
    {/if}
    <span class="composer-actions">
      {#if composer.imagesTaken && compose.kind !== 'closed'}
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
            void composer.addImages(files);
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
          disabled={!composer.sendable}
          title="Add it to the turn that is running"
          onclick={() => void composer.send('steer')}>Send now</button
        >{/if}
      {#if composer.stopOffered}<button
          type="button"
          class="button"
          disabled={!composer.canStop}
          title={controls.interrupt === 'pending'
            ? 'The turn can be stopped once it has started'
            : 'Stop the running turn (Esc)'}
          onclick={composer.stop}><Icon name="stop" size={12} />Stop</button
        >{/if}
      {#if signedOut && drivable && (sessionClient?.sign_in_methods.length ?? 0) > 0}
        {@const harness = drivable}
        <!-- Signed out: the one action that leads on is signing in. -->
        <button class="button primary" type="button" onclick={() => onsignin(harness)}
          ><Icon name="lock" size={12} />Sign in to {label}</button
        >
      {:else}
        <button
          class="button primary"
          disabled={!composer.sendable}
          title={signedOut
            ? `Sign ${label} in to send`
            : composer.sending
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

<style>
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
</style>
