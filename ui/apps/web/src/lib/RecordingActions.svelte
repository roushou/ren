<!--
  Archive, unarchive, or delete a recording, through this UI server's
  control proxy. Shown only when the server drives runs (it holds the
  daemon's control token). A delete asks once more first.
-->
<script lang="ts">
  import { onMount, untrack } from 'svelte';
  import { goto } from '$app/navigation';
  import { ManagementApi, type RecordingId } from './api.ts';
  import { RunCommandError, RunsClient, type HistoryChange } from './runs.ts';

  let {
    recording,
    shelf,
  }: {
    recording: RecordingId;
    /** Whether it is archived, from the recording's own listing; null when
     * the page did not list it, and it is read here instead. */
    shelf: boolean | null;
  } = $props();

  const client = new RunsClient();
  /** Whether it is archived; null until read. Known from the page when it
   * listed the recording, so the buttons render with the page. */
  let archived = $state<boolean | null>(untrack(() => shelf));
  let confirming = $state(false);
  let busy = $state(false);
  let notice = $state<string | null>(null);

  async function readShelf() {
    try {
      const listed = await new ManagementApi().recordings(100, undefined, 'archived');
      archived = listed.recordings.some((entry) => entry.id === recording);
    } catch {
      archived = false;
    }
  }

  async function edit(change: HistoryChange) {
    busy = true;
    notice = null;
    confirming = false;
    try {
      await client.edit({ kind: 'recording', id: recording }, change);
      if (change === 'delete') {
        await goto('/recordings');
        return;
      }
      archived = change === 'archive';
    } catch (error) {
      notice = error instanceof RunCommandError ? error.message : 'The change was not made.';
    } finally {
      busy = false;
    }
  }

  onMount(() => {
    if (archived === null) void readShelf();
  });
</script>

<span class="recording-actions">
  {#if notice}<span class="danger-text">{notice}</span>{/if}
  {#if confirming}
    <span class="danger-text">Delete this recording’s calls for good?</span>
    <button class="button small danger confirm" disabled={busy} onclick={() => edit('delete')}
      >Delete</button
    >
    <button class="button small" onclick={() => (confirming = false)}>Cancel</button>
  {:else if archived !== null}
    <button
      class="button small"
      disabled={busy}
      onclick={() => edit(archived ? 'unarchive' : 'archive')}
      title={archived
        ? 'List it with the current recordings again'
        : 'Hide it from the recordings list until it has new traffic'}
      >{archived ? 'Unarchive' : 'Archive'}</button
    >
    <button class="button small danger" disabled={busy} onclick={() => (confirming = true)}
      >Delete</button
    >
  {/if}
</span>

<style>
  .recording-actions {
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }
  .danger-text {
    color: var(--error);
    font-size: 11px;
  }
</style>
